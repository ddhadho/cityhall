use cityhall::{Entry, Wal};
use cityhall::replication::leader::ReplicationServer;
use cityhall::replication::protocol::{
    send_request, recv_response,
    SyncRequest, SyncResponse,
};
use std::sync::Arc;
use parking_lot::RwLock;
use tempfile::tempdir;
use tokio::net::TcpStream;
use tokio::time::{sleep, Duration};

/// Test: Start server, connect as replica, request segment
#[tokio::test]
async fn test_leader_serves_segment() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("leader.wal");
    
    // Setup leader WAL with data
    let mut wal = Wal::new(&path, 1024).unwrap();
    wal.set_segment_size_limit(5_000);
    
    // Write entries to create segments
    for i in 0..50 {
        let entry = Entry {
            key: format!("metric.cpu.{}", i).into_bytes(),

            value: vec![b'x'; 100],
            timestamp: 1000 + i,
        };
        wal.append(&entry).unwrap();
    }
    wal.flush().unwrap();
    
    // Get first closed segment number
    let segments = wal.list_closed_segments().unwrap();
    assert!(segments.len() > 0, "Need at least one closed segment");
    let first_segment = segments[0];
    
    let wal = Arc::new(RwLock::new(wal));
    
    // Start replication server
    let server = ReplicationServer::new(Arc::clone(&wal), 17879);
    tokio::spawn(async move {
        server.serve().await.unwrap();
    });
    
    // Give server time to start
    sleep(Duration::from_millis(100)).await;
    
    // Connect as replica
    let mut stream = TcpStream::connect("127.0.0.1:17879")
        .await
        .expect("Failed to connect to leader");
    
    // Request segment
    let request = SyncRequest::GetSegment {
        segment_number: first_segment,
    };
    send_request(&mut stream, &request).await.unwrap();
    
    // Receive response
    let response = recv_response(&mut stream).await.unwrap().unwrap();
    
    match response {
        SyncResponse::SegmentData { segment_number, entries } => {
            assert_eq!(segment_number, first_segment);
            assert!(entries.len() > 0, "Should receive entries");
            
            // Verify entry content
            assert!(entries[0].key.starts_with(b"metric.cpu."));
            
            println!("✓ Received {} entries from segment {}", entries.len(), segment_number);
        }
        _ => panic!("Expected SegmentData response, got: {:?}", response),
    }
}

/// Test: Request non-existent segment
#[tokio::test]
async fn test_leader_segment_not_found() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("leader.wal");
    
    let wal = Wal::new(&path, 1024).unwrap();
    let wal = Arc::new(RwLock::new(wal));
    
    // Start server
    let server = ReplicationServer::new(Arc::clone(&wal), 17880);
    tokio::spawn(async move {
        server.serve().await.unwrap();
    });
    
    sleep(Duration::from_millis(100)).await;
    
    // Connect and request non-existent segment
    let mut stream = TcpStream::connect("127.0.0.1:17880").await.unwrap();
    
    let request = SyncRequest::GetSegment { segment_number: 999 };
    send_request(&mut stream, &request).await.unwrap();
    
    let response = recv_response(&mut stream).await.unwrap().unwrap();
    
    match response {
        SyncResponse::SegmentNotFound { segment_number } => {
            assert_eq!(segment_number, 999);
            println!("✓ Correctly returned SegmentNotFound");
        }
        _ => panic!("Expected SegmentNotFound, got: {:?}", response),
    }
}

/// Test: Request list of segments
#[tokio::test]
async fn test_leader_list_segments() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("leader.wal");
    
    let mut wal = Wal::new(&path, 1024).unwrap();
    wal.set_segment_size_limit(5_000);
    
    // Create multiple segments
    for i in 0..100 {
        let entry = Entry {
            key: format!("key_{}", i).into_bytes(),
            value: vec![0u8; 100],
            timestamp: 1000 + i,
        };
        wal.append(&entry).unwrap();
    }
    wal.flush().unwrap();
    
    let expected_segments = wal.list_closed_segments().unwrap();
    let expected_current = wal.current_segment_number();
    
    let wal = Arc::new(RwLock::new(wal));
    
    // Start server
    let server = ReplicationServer::new(Arc::clone(&wal), 17881);
    tokio::spawn(async move {
        server.serve().await.unwrap();
    });
    
    sleep(Duration::from_millis(100)).await;
    
    // Connect and request segment list
    let mut stream = TcpStream::connect("127.0.0.1:17881").await.unwrap();
    
    let request = SyncRequest::ListSegments;
    send_request(&mut stream, &request).await.unwrap();
    
    let response = recv_response(&mut stream).await.unwrap().unwrap();
    
    match response {
        SyncResponse::SegmentList { segments, current_segment } => {
            assert_eq!(segments, expected_segments);
            assert_eq!(current_segment, expected_current);
            
            println!("✓ Received segment list: {:?} (current: {})", segments, current_segment);
        }
        _ => panic!("Expected SegmentList, got: {:?}", response),
    }
}

/// Test: Multiple replicas connecting simultaneously
#[tokio::test]
async fn test_leader_multiple_replicas() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("leader.wal");
    
    let mut wal = Wal::new(&path, 1024).unwrap();
    wal.set_segment_size_limit(5_000);
    
    // Create data
    for i in 0..50 {
        let entry = Entry {
            key: format!("key_{}", i).into_bytes(),
            value: vec![0u8; 200],
            timestamp: 1000 + i,
        };
        wal.append(&entry).unwrap();
    }
    wal.flush().unwrap();
    
    let segments = wal.list_closed_segments().unwrap();
    let first_segment = segments[0];
    
    let wal = Arc::new(RwLock::new(wal));
    
    // Start server
    let server = ReplicationServer::new(Arc::clone(&wal), 17882);
    tokio::spawn(async move {
        server.serve().await.unwrap();
    });
    
    sleep(Duration::from_millis(100)).await;
    
    // Spawn 3 replicas connecting simultaneously
    let mut handles = vec![];
    
    for replica_id in 1..=3 {
        let segment = first_segment;
        
        let handle = tokio::spawn(async move {
            let mut stream = TcpStream::connect("127.0.0.1:17882").await.unwrap();
            
            let request = SyncRequest::GetSegment { segment_number: segment };
            send_request(&mut stream, &request).await.unwrap();
            
            let response = recv_response(&mut stream).await.unwrap().unwrap();
            
            match response {
                SyncResponse::SegmentData { segment_number, entries } => {
                    assert_eq!(segment_number, segment);
                    assert!(entries.len() > 0);
                    println!("✓ Replica {} received {} entries", replica_id, entries.len());
                }
                _ => panic!("Replica {} got unexpected response", replica_id),
            }
        });
        
        handles.push(handle);
    }
    
    // Wait for all replicas to complete
    for handle in handles {
        handle.await.unwrap();
    }
    
    println!("✓ All 3 replicas successfully synced");
}

/// Test: Sequential segment requests on same connection
#[tokio::test]
async fn test_leader_sequential_requests() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("leader.wal");
    
    let mut wal = Wal::new(&path, 1024).unwrap();
    wal.set_segment_size_limit(5_000);
    
    // Create multiple segments
    for i in 0..100 {
        let entry = Entry {
            key: format!("key_{}", i).into_bytes(),
            value: vec![0u8; 100],
            timestamp: 1000 + i,
        };
        wal.append(&entry).unwrap();
    }
    wal.flush().unwrap();
    
    let segments = wal.list_closed_segments().unwrap();
    assert!(segments.len() >= 2, "Need at least 2 segments");
    
    let wal = Arc::new(RwLock::new(wal));
    
    // Start server
    let server = ReplicationServer::new(Arc::clone(&wal), 17883);
    tokio::spawn(async move {
        server.serve().await.unwrap();
    });
    
    sleep(Duration::from_millis(100)).await;
    
    // Connect once, request multiple segments
    let mut stream = TcpStream::connect("127.0.0.1:17883").await.unwrap();
    
    for segment in &segments[..2.min(segments.len())] {
        let request = SyncRequest::GetSegment { segment_number: *segment };
        send_request(&mut stream, &request).await.unwrap();
        
        let response = recv_response(&mut stream).await.unwrap().unwrap();
        
        match response {
            SyncResponse::SegmentData { segment_number, entries } => {
                assert_eq!(segment_number, *segment);
                assert!(entries.len() > 0);
                println!("✓ Received segment {} ({} entries)", segment_number, entries.len());
            }
            _ => panic!("Expected SegmentData"),
        }
    }
    
    println!("✓ Sequential requests on same connection work");
}