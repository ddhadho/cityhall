use cityhall::replication::protocol::{
    recv_request, recv_response, send_request, send_response, SyncRequest, SyncResponse,
};
use cityhall::Entry;
use tokio::net::{TcpListener, TcpStream};

/// Helper: Start a test server that echoes requests back as responses
async fn start_echo_server(port: u16) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
            .await
            .unwrap();

        let (mut stream, _) = listener.accept().await.unwrap();

        // Read request
        let request = recv_request(&mut stream).await.unwrap().unwrap();

        // Echo back as segment data (for testing)
        let response = match request {
            SyncRequest::GetSegment { segment_number } => SyncResponse::SegmentData {
                segment_number,
                entries: vec![],
            },
            SyncRequest::ListSegments => SyncResponse::SegmentList {
                segments: vec![1, 2, 3],
                current_segment: 4,
            },
        };

        send_response(&mut stream, &response).await.unwrap();
    })
}

#[tokio::test]
async fn test_request_response_round_trip() {
    let port = 19001;

    // Start server
    let _server = start_echo_server(port).await;

    // Give server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Connect as client
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();

    // Send request
    let request = SyncRequest::GetSegment { segment_number: 42 };
    send_request(&mut stream, &request).await.unwrap();

    // Receive response
    let response = recv_response(&mut stream).await.unwrap().unwrap();

    match response {
        SyncResponse::SegmentData { segment_number, .. } => {
            assert_eq!(segment_number, 42);
        }
        _ => panic!("Unexpected response type"),
    }
}

#[tokio::test]
async fn test_list_segments_request() {
    let port = 19002;

    let _server = start_echo_server(port).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();

    // Send list segments request
    let request = SyncRequest::ListSegments;
    send_request(&mut stream, &request).await.unwrap();

    // Receive response
    let response = recv_response(&mut stream).await.unwrap().unwrap();

    match response {
        SyncResponse::SegmentList {
            segments,
            current_segment,
        } => {
            assert_eq!(segments, vec![1, 2, 3]);
            assert_eq!(current_segment, 4);
        }
        _ => panic!("Unexpected response type"),
    }
}

#[tokio::test]
async fn test_large_segment_data() {
    let port = 19003;

    // Start server that sends large response
    let server = tokio::spawn(async move {
        let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
            .await
            .unwrap();

        let (mut stream, _) = listener.accept().await.unwrap();

        // Read request
        let _request = recv_request(&mut stream).await.unwrap().unwrap();

        // Create large response (1000 entries)
        let entries: Vec<Entry> = (0..1000)
            .map(|i| Entry {
                key: format!("key_{}", i).into_bytes(),
                value: format!("value_{}", i).into_bytes(),
                timestamp: 1000 + i,
            })
            .collect();

        let response = SyncResponse::SegmentData {
            segment_number: 1,
            entries,
        };

        send_response(&mut stream, &response).await.unwrap();
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Connect and request
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();

    let request = SyncRequest::GetSegment { segment_number: 1 };
    send_request(&mut stream, &request).await.unwrap();

    // Receive large response
    let response = recv_response(&mut stream).await.unwrap().unwrap();

    match response {
        SyncResponse::SegmentData {
            segment_number,
            entries,
        } => {
            assert_eq!(segment_number, 1);
            assert_eq!(entries.len(), 1000);
            assert_eq!(entries[0].key, b"key_0");
            assert_eq!(entries[999].key, b"key_999");
        }
        _ => panic!("Unexpected response type"),
    }

    server.await.unwrap();
}

#[tokio::test]
async fn test_multiple_requests_same_connection() {
    let port = 19004;

    // Start server that handles multiple requests
    let server = tokio::spawn(async move {
        let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
            .await
            .unwrap();

        let (mut stream, _) = listener.accept().await.unwrap();

        // Handle first request
        let request1 = recv_request(&mut stream).await.unwrap().unwrap();
        match request1 {
            SyncRequest::GetSegment { segment_number } => {
                let response = SyncResponse::SegmentData {
                    segment_number,
                    entries: vec![],
                };
                send_response(&mut stream, &response).await.unwrap();
            }
            _ => panic!("Unexpected request"),
        }

        // Handle second request
        let request2 = recv_request(&mut stream).await.unwrap().unwrap();
        match request2 {
            SyncRequest::GetSegment { segment_number } => {
                let response = SyncResponse::SegmentData {
                    segment_number,
                    entries: vec![],
                };
                send_response(&mut stream, &response).await.unwrap();
            }
            _ => panic!("Unexpected request"),
        }
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Connect and send multiple requests
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();

    // Request 1
    send_request(&mut stream, &SyncRequest::GetSegment { segment_number: 1 })
        .await
        .unwrap();
    let response1 = recv_response(&mut stream).await.unwrap().unwrap();

    // Request 2
    send_request(&mut stream, &SyncRequest::GetSegment { segment_number: 2 })
        .await
        .unwrap();
    let response2 = recv_response(&mut stream).await.unwrap().unwrap();

    // Verify both responses
    match (response1, response2) {
        (
            SyncResponse::SegmentData {
                segment_number: n1, ..
            },
            SyncResponse::SegmentData {
                segment_number: n2, ..
            },
        ) => {
            assert_eq!(n1, 1);
            assert_eq!(n2, 2);
        }
        _ => panic!("Unexpected response types"),
    }

    server.await.unwrap();
}

#[tokio::test]
async fn test_connection_close_handling() {
    let port = 19005;

    // Start server that closes connection
    tokio::spawn(async move {
        let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
            .await
            .unwrap();

        let (stream, _) = listener.accept().await.unwrap();

        // Close connection immediately
        drop(stream);
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Connect and try to send
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();

    // Give time for connection to close
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Try to receive (should get None)
    let result = recv_request(&mut stream).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_error_response() {
    let port = 19006;

    // Start server that sends error
    tokio::spawn(async move {
        let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
            .await
            .unwrap();

        let (mut stream, _) = listener.accept().await.unwrap();

        let _request = recv_request(&mut stream).await.unwrap().unwrap();

        let response = SyncResponse::Error {
            message: "Test error".to_string(),
        };

        send_response(&mut stream, &response).await.unwrap();
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();

    let request = SyncRequest::GetSegment { segment_number: 1 };
    send_request(&mut stream, &request).await.unwrap();

    let response = recv_response(&mut stream).await.unwrap().unwrap();

    match response {
        SyncResponse::Error { message } => {
            assert_eq!(message, "Test error");
        }
        _ => panic!("Expected error response"),
    }
}
