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

use crate::{replication::ReplicaRegistry, Result, StorageError, Wal};
use parking_lot::RwLock;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};

use super::protocol::{recv_request, send_response, SyncRequest, SyncResponse};

/// Replication server that serves WAL segments to replicas
pub struct ReplicationServer {
    wal: Arc<RwLock<Wal>>,
    addr: String,
    registry: Arc<ReplicaRegistry>,
}

impl ReplicationServer {
    /// Create a new replication server
    ///
    /// # Arguments
    /// * `wal` - Shared WAL instance (thread-safe)
    /// * `port` - Port to listen on (typically 7879)
    pub fn new(wal: Arc<RwLock<Wal>>, registry: Arc<ReplicaRegistry>, port: u16) -> Self {
        let addr = format!("0.0.0.0:{}", port);

        Self {
            wal,
            addr,
            registry,
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
                    println!("Replica connected: {}", addr);

                    let wal = Arc::clone(&self.wal);
                    let registry = Arc::clone(&self.registry);
                    let peer_addr = addr.to_string();

                    // Spawn handler for this replica
                    tokio::spawn(async move {
                        if let Err(e) =
                            handle_replica(stream, wal, registry, peer_addr.clone()).await
                        {
                            eprintln!("Error handling replica {}: {}", addr, e);
                        } else {
                            println!("Replica {} disconnected", addr);
                        }
                    });
                }
                Err(e) => {
                    eprintln!("Failed to accept connection: {}", e);
                }
            }
        }
    }
}

/// Handle a single replica connection
///
/// Processes requests until the connection is closed.
///
async fn handle_replica(
    mut stream: TcpStream,
    wal: Arc<RwLock<Wal>>,
    registry: Arc<ReplicaRegistry>,
    peer_addr: String,
) -> Result<()> {
    let mut handshake_complete = false; // ← Track handshake state for THIS connection

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
            SyncRequest::Handshake {
                replica_id: rid,
                last_synced_segment,
            } => {
                if handshake_complete {
                    // Already handshaked on THIS connection
                    SyncResponse::Error {
                        message: "Handshake already completed on this connection".to_string(),
                    }
                } else {
                    // First handshake on this connection - process it
                    let current_segment = wal.read().current_segment_number();

                    // Register replica in registry
                    registry
                        .register(rid.clone(), peer_addr.clone(), last_synced_segment)
                        .await;

                    println!(
                        "🤝 Handshake with replica: {} (last synced: {})",
                        rid, last_synced_segment
                    );

                    // Mark handshake complete for this connection
                    handshake_complete = true;

                    // Send acknowledgment
                    SyncResponse::HandshakeAck {
                        leader_id: "leader-main".to_string(), // Or use hostname
                        current_segment,
                    }
                }
            }
            SyncRequest::GetSegment { segment_number } => {
                if !handshake_complete {
                    SyncResponse::Error {
                        message: "Must complete handshake first".to_string(),
                    }
                } else {
                    handle_get_segment(segment_number, &wal)
                }
            }
            SyncRequest::ListSegments => {
                if !handshake_complete {
                    SyncResponse::Error {
                        message: "Must complete handshake first".to_string(),
                    }
                } else {
                    handle_list_segments(&wal)
                }
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
        println!(
            "Segment {} not available (still active or deleted)",
            segment_number
        );
        return SyncResponse::SegmentNotFound { segment_number };
    }

    // Read segment entries
    match wal.read_segment(segment_number) {
        Ok(entries) => {
            println!(
                "Sending segment {} ({} entries, ~{} KB)",
                segment_number,
                entries.len(),
                estimate_size_kb(&entries)
            );

            SyncResponse::SegmentData {
                segment_number,
                entries,
            }
        }
        Err(StorageError::NotFound(_)) => {
            println!("Segment {} not found", segment_number);
            SyncResponse::SegmentNotFound { segment_number }
        }
        Err(e) => {
            eprintln!("Error reading segment {}: {}", segment_number, e);
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

            println!("Listing segments: {:?} (current: {})", segments, current);

            SyncResponse::SegmentList {
                segments,
                current_segment: current,
            }
        }
        Err(e) => {
            eprintln!("Error listing segments: {}", e);
            SyncResponse::error(format!("Failed to list segments: {}", e))
        }
    }
}

/// Estimate size of entries in KB (for logging)
fn estimate_size_kb(entries: &[crate::Entry]) -> usize {
    let bytes: usize = entries
        .iter()
        .map(|e| e.key.len() + e.value.len() + 8) // +8 for timestamp
        .sum();
    bytes / 1024
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replication::protocol::{recv_response, send_request};
    use crate::{replication::ReplicaRegistry, Wal};
    use tempfile::tempdir;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn test_handshake_flow() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");

        let wal = Wal::new(&wal_path, 1024).unwrap();
        let wal = Arc::new(RwLock::new(wal));

        // Corrected: Create registry first, then pass it to the server.
        let registry = Arc::new(ReplicaRegistry::new());
        let server = ReplicationServer::new(Arc::clone(&wal), Arc::clone(&registry), 17879);

        // Start server
        tokio::spawn(async move {
            let _ = server.serve().await;
        });

        sleep(Duration::from_millis(100)).await;

        // Connect as a fake replica
        let mut stream = TcpStream::connect("127.0.0.1:17879").await.unwrap();

        // Send handshake
        let handshake = SyncRequest::Handshake {
            replica_id: "test-replica".to_string(),
            last_synced_segment: 0,
        };
        send_request(&mut stream, &handshake).await.unwrap();

        // Receive ack
        let response = recv_response(&mut stream).await.unwrap().unwrap();

        match response {
            SyncResponse::HandshakeAck { leader_id, .. } => {
                // Corrected: Check that leader_id is not empty, as it's now the hostname.
                assert!(!leader_id.is_empty());
            }
            _ => panic!("Expected HandshakeAck"),
        }

        // Check registry
        // A small sleep might be needed for the server to process the registration asynchronously
        sleep(Duration::from_millis(50)).await;
        let replicas = registry.get_all().await;
        assert_eq!(replicas.len(), 1);
        assert_eq!(replicas[0].replica_id, "test-replica");
    }
}
