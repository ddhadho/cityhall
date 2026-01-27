//! Replication protocol message types
//!
//! Wire protocol for leader-replica communication
//!
//! ## Message Format
//! 
//! All messages are length-prefixed:
//! ```
//! [4 bytes: length (u32, little-endian)]
//! [N bytes: bincode-serialized payload]
//! ```

use crate::{Entry, Result, StorageError};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Request from replica to leader
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SyncRequest {
    /// NEW: Initial handshake with replica identification
    Handshake {
        replica_id: String,
        last_synced_segment: u64,
    },
    /// Request entries from a specific segment
    GetSegment { segment_number: u64 },
    
    /// Get list of available segments
    ListSegments,
}

/// Response from leader to replica
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncResponse {
    /// NEW: Handshake acknowledgment
    HandshakeAck {
        leader_id: String,
        current_segment: u64,
    },
    /// Segment data with entries
    SegmentData {
        segment_number: u64,
        entries: Vec<Entry>,
    },
    
    /// List of available segment numbers
    SegmentList {
        segments: Vec<u64>,
        current_segment: u64,
    },
    
    /// Segment not found (deleted or doesn't exist)
    SegmentNotFound {
        segment_number: u64,
    },
    
    /// Error occurred
    Error {
        message: String,
    },
}

impl SyncResponse {
    /// Create an error response
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
        }
    }
}

// ============================================================================
// Message Framing - Length-Prefixed Protocol
// ============================================================================

/// Send a SyncRequest over TCP
/// 
/// # Wire Format
/// ```
/// [4 bytes: length] [N bytes: bincode payload]
/// ```
pub async fn send_request(stream: &mut TcpStream, request: &SyncRequest) -> Result<()> {
    // Serialize message
    let payload = bincode::serialize(request)
        .map_err(|e| StorageError::InvalidFormat(format!("Serialize error: {}", e)))?;
    
    // Send length prefix
    let len = payload.len() as u32;
    stream.write_u32_le(len).await?;
    
    // Send payload
    stream.write_all(&payload).await?;
    stream.flush().await?;
    
    Ok(())
}

/// Receive a SyncRequest from TCP
/// 
/// Returns `Ok(None)` if connection is closed gracefully
pub async fn recv_request(stream: &mut TcpStream) -> Result<Option<SyncRequest>> {
    // Read length prefix
    let len = match stream.read_u32_le().await {
        Ok(l) => l as usize,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Ok(None); // Connection closed
        }
        Err(e) => return Err(e.into()),
    };
    
    // Validate length (prevent DoS)
    if len > 100 * 1024 * 1024 {
        return Err(StorageError::InvalidFormat(format!(
            "Message too large: {} bytes (max 100MB)",
            len
        )));
    }
    
    // Read payload
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    
    // Deserialize
    let request = bincode::deserialize(&buf)
        .map_err(|e| StorageError::InvalidFormat(format!("Deserialize error: {}", e)))?;
    
    Ok(Some(request))
}

/// Send a SyncResponse over TCP
pub async fn send_response(stream: &mut TcpStream, response: &SyncResponse) -> Result<()> {
    // Serialize message
    let payload = bincode::serialize(response)
        .map_err(|e| StorageError::InvalidFormat(format!("Serialize error: {}", e)))?;
    
    // Send length prefix
    let len = payload.len() as u32;
    stream.write_u32_le(len).await?;
    
    // Send payload
    stream.write_all(&payload).await?;
    stream.flush().await?;
    
    Ok(())
}

/// Receive a SyncResponse from TCP
/// 
/// Returns `Ok(None)` if connection is closed gracefully
pub async fn recv_response(stream: &mut TcpStream) -> Result<Option<SyncResponse>> {
    // Read length prefix
    let len = match stream.read_u32_le().await {
        Ok(l) => l as usize,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Ok(None); // Connection closed
        }
        Err(e) => return Err(e.into()),
    };
    
    // Validate length
    if len > 100 * 1024 * 1024 {
        return Err(StorageError::InvalidFormat(format!(
            "Message too large: {} bytes (max 100MB)",
            len
        )));
    }
    
    // Read payload
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    
    // Deserialize
    let response = bincode::deserialize(&buf)
        .map_err(|e| StorageError::InvalidFormat(format!("Deserialize error: {}", e)))?;
    
    Ok(Some(response))
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Calculate size of a serialized message (for testing/metrics)
pub fn message_size<T: Serialize>(msg: &T) -> Result<usize> {
    bincode::serialize(msg)
        .map(|bytes| bytes.len())
        .map_err(|e| StorageError::InvalidFormat(format!("Serialize error: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_sync_request_serialization() {
        let request = SyncRequest::GetSegment { segment_number: 42 };
        
        // Serialize
        let encoded = bincode::serialize(&request).unwrap();
        assert!(encoded.len() > 0);
        
        // Deserialize
        let decoded: SyncRequest = bincode::deserialize(&encoded).unwrap();
        assert_eq!(request, decoded);
    }
    
    #[test]
    fn test_sync_request_list_segments() {
        let request = SyncRequest::ListSegments;
        
        let encoded = bincode::serialize(&request).unwrap();
        let decoded: SyncRequest = bincode::deserialize(&encoded).unwrap();
        
        assert_eq!(request, decoded);
    }
    
    #[test]
    fn test_sync_response_segment_data() {
        let entries = vec![
            Entry {
                key: b"key1".to_vec(),
                value: b"value1".to_vec(),
                timestamp: 1000,
            },
            Entry {
                key: b"key2".to_vec(),
                value: b"value2".to_vec(),
                timestamp: 2000,
            },
        ];
        
        let response = SyncResponse::SegmentData {
            segment_number: 5,
            entries: entries.clone(),
        };
        
        let encoded = bincode::serialize(&response).unwrap();
        let decoded: SyncResponse = bincode::deserialize(&encoded).unwrap();
        
        match decoded {
            SyncResponse::SegmentData { segment_number, entries: decoded_entries } => {
                assert_eq!(segment_number, 5);
                assert_eq!(decoded_entries.len(), 2);
                assert_eq!(decoded_entries[0].key, b"key1");
            }
            _ => panic!("Wrong response type"),
        }
    }
    
    #[test]
    fn test_sync_response_segment_list() {
        let response = SyncResponse::SegmentList {
            segments: vec![1, 2, 3, 4],
            current_segment: 5,
        };
        
        let encoded = bincode::serialize(&response).unwrap();
        let decoded: SyncResponse = bincode::deserialize(&encoded).unwrap();
        
        match decoded {
            SyncResponse::SegmentList { segments, current_segment } => {
                assert_eq!(segments, vec![1, 2, 3, 4]);
                assert_eq!(current_segment, 5);
            }
            _ => panic!("Wrong response type"),
        }
    }
    
    #[test]
    fn test_sync_response_error() {
        let response = SyncResponse::error("Test error message");
        
        let encoded = bincode::serialize(&response).unwrap();
        let decoded: SyncResponse = bincode::deserialize(&encoded).unwrap();
        
        match decoded {
            SyncResponse::Error { message } => {
                assert_eq!(message, "Test error message");
            }
            _ => panic!("Wrong response type"),
        }
    }
    
    #[test]
    fn test_message_size_calculation() {
        let request = SyncRequest::GetSegment { segment_number: 42 };
        let size = message_size(&request).unwrap();
        
        assert!(size > 0);
        assert!(size < 100); // Should be small
    }
    
    #[test]
    fn test_large_message_size() {
        // Create response with many entries
        let entries: Vec<Entry> = (0..1000)
            .map(|i| Entry {
                key: format!("key_{}", i).into_bytes(),
                value: vec![0u8; 1000], // 1KB per entry
                timestamp: i,
            })
            .collect();
        
        let response = SyncResponse::SegmentData {
            segment_number: 1,
            entries,
        };
        
        let size = message_size(&response).unwrap();
        println!("Message size: {} bytes ({} MB)", size, size / 1_048_576);
        
        // Should be roughly 1MB (1000 entries * 1KB each)
        assert!(size > 900_000);
        assert!(size < 2_000_000);
    }
}