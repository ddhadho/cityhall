//! Replication Server (Leader Side)
//!
//! Serves WAL segments to replica nodes over TCP
//!
//! ## Architecture
//! 
//! ```
//! Leader Process
//! ├── Main Thread: Accept writes, append to WAL
//! └── Replication Server Thread: Serve segments to replicas
//!     ├── Listen on port 7879
//!     ├── Accept replica connections
//!     └── Spawn handler per connection
//! ```

use crate::{Result, StorageError, Wal};
use std::sync::Arc;
use parking_lot::RwLock;
use tokio::net::{TcpListener, TcpStream};

use super::protocol::{
    send_response, recv_request,
    SyncRequest, SyncResponse,
};

/// Replication server that serves WAL segments to replicas
pub struct ReplicationServer {
    wal: Arc<RwLock<Wal>>,
    addr: String,
}

impl ReplicationServer {
    /// Create a new replication server
    /// 
    /// # Arguments
    /// * `wal` - Shared WAL instance (thread-safe)
    /// * `port` - Port to listen on (typically 7879)
    pub fn new(wal: Arc<RwLock<Wal>>, port: u16) -> Self {
        Self {
            wal,
            addr: format!("0.0.0.0:{}", port),
        }
    }
    
    /// Start serving replica requests
    /// 
    /// This function runs indefinitely, accepting connections and spawning
    /// handlers for each replica. Use `tokio::spawn()` to run in background.
    /// 
    /// # Example
    /// ```no_run
    /// let server = ReplicationServer::new(wal, 7879);
    /// tokio::spawn(async move {
    ///     server.serve().await.unwrap();
    /// });
    /// ```
    pub async fn serve(self) -> Result<()> {
        let listener = TcpListener::bind(&self.addr).await?;
        println!("🌐 Replication server listening on {}", self.addr);
        
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    println!("📡 Replica connected: {}", addr);
                    
                    let wal = Arc::clone(&self.wal);
                    
                    // Spawn handler for this replica
                    tokio::spawn(async move {
                        if let Err(e) = handle_replica(stream, wal).await {
                            eprintln!("❌ Error handling replica {}: {}", addr, e);
                        } else {
                            println!("✓ Replica {} disconnected", addr);
                        }
                    });
                }
                Err(e) => {
                    eprintln!("⚠️  Failed to accept connection: {}", e);
                }
            }
        }
    }
}

/// Handle a single replica connection
/// 
/// Processes requests until the connection is closed.
async fn handle_replica(
    mut stream: TcpStream,
    wal: Arc<RwLock<Wal>>,
) -> Result<()> {
    loop {
        // Read request from replica
        let request = match recv_request(&mut stream).await? {
            Some(req) => req,
            None => {
                // Connection closed gracefully
                return Ok(());
            }
        };
        
        // Process request and generate response
        let response = match request {
            SyncRequest::GetSegment { segment_number } => {
                handle_get_segment(segment_number, &wal)
            }
            SyncRequest::ListSegments => {
                handle_list_segments(&wal)
            }
        };
        
        // Send response back to replica
        send_response(&mut stream, &response).await?;
    }
}

/// Handle GetSegment request
/// 
/// Reads the requested segment from WAL and returns all entries.
fn handle_get_segment(segment_number: u64, wal: &Arc<RwLock<Wal>>) -> SyncResponse {
    let wal = wal.read();
    
    // Check if segment is available (closed, not active)
    if !wal.is_segment_available(segment_number) {
        println!("⚠️  Segment {} not available (still active or deleted)", segment_number);
        return SyncResponse::SegmentNotFound { segment_number };
    }
    
    // Read segment entries
    match wal.read_segment(segment_number) {
        Ok(entries) => {
            println!("📤 Sending segment {} ({} entries, ~{} KB)",
                     segment_number,
                     entries.len(),
                     estimate_size_kb(&entries));
            
            SyncResponse::SegmentData {
                segment_number,
                entries,
            }
        }
        Err(StorageError::NotFound(_)) => {
            println!("⚠️  Segment {} not found", segment_number);
            SyncResponse::SegmentNotFound { segment_number }
        }
        Err(e) => {
            eprintln!("❌ Error reading segment {}: {}", segment_number, e);
            SyncResponse::error(format!("Failed to read segment: {}", e))
        }
    }
}

/// Handle ListSegments request
/// 
/// Returns list of closed segments available for replication.
fn handle_list_segments(wal: &Arc<RwLock<Wal>>) -> SyncResponse {
    let wal = wal.read();
    
    match wal.list_closed_segments() {
        Ok(segments) => {
            let current = wal.current_segment_number();
            
            println!("📋 Listing segments: {:?} (current: {})", segments, current);
            
            SyncResponse::SegmentList {
                segments,
                current_segment: current,
            }
        }
        Err(e) => {
            eprintln!("❌ Error listing segments: {}", e);
            SyncResponse::error(format!("Failed to list segments: {}", e))
        }
    }
}

/// Estimate size of entries in KB (for logging)
fn estimate_size_kb(entries: &[crate::Entry]) -> usize {
    let bytes: usize = entries.iter()
        .map(|e| e.key.len() + e.value.len() + 8) // +8 for timestamp
        .sum();
    bytes / 1024
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Entry, Wal};
    use tempfile::tempdir;
    use tokio::time::{sleep, Duration};
    
    #[tokio::test]
    async fn test_replication_server_start() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.wal");
        
        let wal = Wal::new(&path, 1024).unwrap();
        let wal = Arc::new(RwLock::new(wal));
        
        let server = ReplicationServer::new(Arc::clone(&wal), 18879);
        
        // Start server in background
        let handle = tokio::spawn(async move {
            // Run for a short time then cancel
            tokio::select! {
                _ = server.serve() => {}
                _ = sleep(Duration::from_millis(500)) => {}
            }
        });
        
        // Give server time to start
        sleep(Duration::from_millis(100)).await;
        
        // Try to connect (should succeed)
        let result = TcpStream::connect("127.0.0.1:18879").await;
        assert!(result.is_ok(), "Should be able to connect to server");
        
        // Cleanup
        handle.abort();
    }
    
    #[tokio::test]
    async fn test_handle_get_segment() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.wal");
        
        let mut wal = Wal::new(&path, 1024).unwrap();
        
        // Force small segments for testing
        wal.set_segment_size_limit(5_000);
        
        // Write data to create segments
        for i in 0..50 {
            let entry = Entry {
                key: format!("key_{}", i).into_bytes(),
                value: vec![0u8; 200],
                timestamp: 1000 + i,
            };
            wal.append(&entry).unwrap();
        }
        wal.flush().unwrap();
        
        let wal = Arc::new(RwLock::new(wal));
        
        // Get first closed segment
        let segments = wal.read().list_closed_segments().unwrap();
        assert!(segments.len() > 0, "Should have closed segments");
        
        let first_segment = segments[0];
        
        // Test handle_get_segment
        let response = handle_get_segment(first_segment, &wal);
        
        match response {
            SyncResponse::SegmentData { segment_number, entries } => {
                assert_eq!(segment_number, first_segment);
                assert!(entries.len() > 0, "Should have entries");
            }
            _ => panic!("Expected SegmentData response"),
        }
    }
    
    #[tokio::test]
    async fn test_handle_get_segment_not_found() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.wal");
        
        let wal = Wal::new(&path, 1024).unwrap();
        let wal = Arc::new(RwLock::new(wal));
        
        // Request non-existent segment
        let response = handle_get_segment(999, &wal);
        
        match response {
            SyncResponse::SegmentNotFound { segment_number } => {
                assert_eq!(segment_number, 999);
            }
            _ => panic!("Expected SegmentNotFound response"),
        }
    }
    
    #[tokio::test]
    async fn test_handle_list_segments() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.wal");
        
        let mut wal = Wal::new(&path, 1024).unwrap();
        wal.set_segment_size_limit(5_000);
        
        // Write data to create segments
        for i in 0..50 {
            let entry = Entry {
                key: format!("key_{}", i).into_bytes(),
                value: vec![0u8; 200],
                timestamp: 1000 + i,
            };
            wal.append(&entry).unwrap();
        }
        wal.flush().unwrap();
        
        let current = wal.current_segment_number();
        let wal = Arc::new(RwLock::new(wal));
        
        // Test handle_list_segments
        let response = handle_list_segments(&wal);
        
        match response {
            SyncResponse::SegmentList { segments, current_segment } => {
                assert!(segments.len() > 0, "Should have segments");
                assert_eq!(current_segment, current);
            }
            _ => panic!("Expected SegmentList response"),
        }
    }
    
    #[test]
    fn test_estimate_size_kb() {
        let entries = vec![
            Entry {
                key: vec![0u8; 100],
                value: vec![0u8; 900],
                timestamp: 1000,
            },
            Entry {
                key: vec![0u8; 100],
                value: vec![0u8; 900],
                timestamp: 2000,
            },
        ];
        
        let size_kb = estimate_size_kb(&entries);
        
        // Each entry is ~1KB, so 2 entries = ~2KB
        assert!(size_kb >= 1 && size_kb <= 3);
    }
}