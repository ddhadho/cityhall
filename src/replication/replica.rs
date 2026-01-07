//! Replication Agent (Replica Side)
//!
//! Syncs WAL segments from leader to replica
//!
//! ## Architecture
//! 
//! ```
//! Replica Process
//! ├── Main Thread: Serve read requests (from local WAL)
//! └── Replication Agent Thread: Sync from leader
//!     ├── Load ReplicaState (last synced segment)
//!     ├── Request next segment from leader
//!     ├── Apply entries to local WAL
//!     └── Update and persist ReplicaState
//! ```

use crate::{Entry, Result, StorageError, Wal};
use std::path::PathBuf;
use std::sync::Arc;
use parking_lot::RwLock;
use tokio::net::TcpStream;
use tokio::time::{sleep, Duration};

use super::protocol::{
    send_request, recv_response,
    SyncRequest, SyncResponse,
};
use super::state::ReplicaState;

/// Replication agent that syncs segments from leader
pub struct ReplicationAgent {
    /// Leader address (e.g., "127.0.0.1:7879")
    leader_addr: String,
    
    /// Path to replica state file
    state_path: PathBuf,
    
    /// Current replica state (in-memory)
    state: ReplicaState,
    
    /// Local WAL (shared with main thread)
    wal: Arc<RwLock<Wal>>,
    
    /// Sync interval (how often to poll leader)
    sync_interval: Duration,
}

impl ReplicationAgent {
    /// Create a new replication agent
    /// 
    /// # Arguments
    /// * `leader_addr` - Leader address (e.g., "10.0.0.1:7879")
    /// * `replica_id` - Unique identifier for this replica
    /// * `state_path` - Path to persist replica state
    /// * `wal` - Shared WAL instance
    pub fn new(
        leader_addr: String,
        replica_id: String,
        state_path: PathBuf,
        wal: Arc<RwLock<Wal>>,
    ) -> Result<Self> {
        // Load or create state
        let state = ReplicaState::load_or_create(
            &state_path,
            replica_id,
            leader_addr.clone(),
        )?;
        
        println!("📥 Replica initialized:");
        println!("   ID: {}", state.replica_id);
        println!("   Leader: {}", state.leader_addr);
        println!("   Last synced segment: {}", state.last_synced_segment);
        
        Ok(Self {
            leader_addr,
            state_path,
            state,
            wal,
            sync_interval: Duration::from_secs(5), // Default: sync every 5 seconds
        })
    }
    
    /// Set custom sync interval
    pub fn set_sync_interval(&mut self, interval: Duration) {
        self.sync_interval = interval;
    }
    
    /// Run sync loop (runs indefinitely)
    /// 
    /// This function continuously syncs segments from the leader.
    /// Use `tokio::spawn()` to run in background.
    /// 
    /// # Example
    /// ```no_run
    /// let mut agent = ReplicationAgent::new(...)?;
    /// tokio::spawn(async move {
    ///     agent.run().await.unwrap();
    /// });
    /// ```
    pub async fn run(&mut self) -> Result<()> {
        println!("🔄 Starting sync loop (interval: {:?})", self.sync_interval);
        
        loop {
            match self.sync_once().await {
                Ok(true) => {
                    // Successfully synced a segment
                    println!("✓ Sync successful");
                }
                Ok(false) => {
                    // No new segments available
                    println!("⏸️  No new segments to sync");
                }
                Err(e) => {
                    eprintln!("❌ Sync error: {}", e);
                    // Continue trying (don't crash on errors)
                }
            }
            
            // Wait before next sync
            sleep(self.sync_interval).await;
        }
    }
    
    /// Perform a single sync operation
    /// 
    /// Returns:
    /// - `Ok(true)` if a segment was synced
    /// - `Ok(false)` if no new segments available
    /// - `Err` on failure
    pub async fn sync_once(&mut self) -> Result<bool> {
        let next_segment = self.state.next_segment_to_sync();
        
        println!("🔄 Attempting to sync segment {}...", next_segment);
        
        // Connect to leader
        let mut stream = TcpStream::connect(&self.leader_addr).await
            .map_err(|e| {
                StorageError::Io(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    format!("Failed to connect to leader {}: {}", self.leader_addr, e)
                ))
            })?;
        
        // Request segment
        let request = SyncRequest::GetSegment {
            segment_number: next_segment,
        };
        send_request(&mut stream, &request).await?;
        
        // Receive response
        let response = recv_response(&mut stream).await?
            .ok_or_else(|| StorageError::InvalidFormat("Connection closed".into()))?;
        
        match response {
            SyncResponse::SegmentData { segment_number, entries } => {
                println!("📥 Received segment {} ({} entries, ~{} KB)",
                         segment_number,
                         entries.len(),
                         estimate_size_kb(&entries));
                
                // Apply entries to local WAL
                self.apply_entries(&entries)?;
                
                // Update state
                self.state.update_synced_segment(segment_number, entries.len() as u64)?;
                self.state.save(&self.state_path)?;
                
                println!("✓ Synced segment {} successfully", segment_number);
                
                Ok(true) // Successfully synced
            }
            SyncResponse::SegmentNotFound { segment_number } => {
                println!("⚠️  Segment {} not found on leader (may be deleted or still active)",
                         segment_number);
                
                // Check if we should retry or give up
                if segment_number > 1 {
                    println!("   Segment may have been compacted. Consider full resync.");
                }
                
                Ok(false) // Nothing to sync
            }
            SyncResponse::Error { message } => {
                Err(StorageError::InvalidFormat(format!("Leader error: {}", message)))
            }
            _ => {
                Err(StorageError::InvalidFormat("Unexpected response type".into()))
            }
        }
    }
    
    /// Apply entries to local WAL
    fn apply_entries(&self, entries: &[Entry]) -> Result<()> {
        let mut wal = self.wal.write();
        
        for entry in entries {
            // Check if this is a delete operation (empty value)
            if entry.value.is_empty() {
                wal.append_delete(&entry.key, entry.timestamp)?;
            } else {
                wal.append(entry)?;
            }
        }
        
        wal.flush()?;
        
        Ok(())
    }
    
    /// Get current replica state (for status/metrics)
    pub fn state(&self) -> &ReplicaState {
        &self.state
    }
    
    /// Request list of available segments from leader
    pub async fn list_segments(&self) -> Result<(Vec<u64>, u64)> {
        let mut stream = TcpStream::connect(&self.leader_addr).await?;
        
        let request = SyncRequest::ListSegments;
        send_request(&mut stream, &request).await?;
        
        let response = recv_response(&mut stream).await?
            .ok_or_else(|| StorageError::InvalidFormat("Connection closed".into()))?;
        
        match response {
            SyncResponse::SegmentList { segments, current_segment } => {
                Ok((segments, current_segment))
            }
            SyncResponse::Error { message } => {
                Err(StorageError::InvalidFormat(format!("Leader error: {}", message)))
            }
            _ => {
                Err(StorageError::InvalidFormat("Unexpected response".into()))
            }
        }
    }
}

/// Estimate size of entries in KB (for logging)
fn estimate_size_kb(entries: &[Entry]) -> usize {
    let bytes: usize = entries.iter()
        .map(|e| e.key.len() + e.value.len() + 8)
        .sum();
    bytes / 1024
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    
    #[test]
    fn test_replica_agent_creation() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("replica.wal");
        let state_path = dir.path().join("replica_state.json");
        
        let wal = Wal::new(&wal_path, 1024).unwrap();
        let wal = Arc::new(RwLock::new(wal));
        
        let agent = ReplicationAgent::new(
            "127.0.0.1:7879".to_string(),
            "test-replica".to_string(),
            state_path.clone(),
            wal,
        ).unwrap();
        
        assert_eq!(agent.state.replica_id, "test-replica");
        assert_eq!(agent.state.leader_addr, "127.0.0.1:7879");
        assert_eq!(agent.state.next_segment_to_sync(), 1);
        
        // State should be persisted
        assert!(state_path.exists());
    }
    
    #[test]
    fn test_replica_agent_loads_existing_state() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("replica.wal");
        let state_path = dir.path().join("replica_state.json");
        
        let wal = Wal::new(&wal_path, 1024).unwrap();
        let wal = Arc::new(RwLock::new(wal));
        
        // Create agent and sync some segments
        {
            let mut agent = ReplicationAgent::new(
                "127.0.0.1:7879".to_string(),
                "test-replica".to_string(),
                state_path.clone(),
                Arc::clone(&wal),
            ).unwrap();
            
            // Manually update state (simulate syncing)
            agent.state.update_synced_segment(5, 100).unwrap();
            agent.state.save(&state_path).unwrap();
        }
        
        // Create new agent (should load existing state)
        let agent = ReplicationAgent::new(
            "127.0.0.1:7879".to_string(),
            "different-id".to_string(), // Different ID, but loads existing
            state_path,
            wal,
        ).unwrap();
        
        assert_eq!(agent.state.last_synced_segment, 5);
        assert_eq!(agent.state.next_segment_to_sync(), 6);
    }
    
    #[test]
    fn test_apply_entries() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("replica.wal");
        let state_path = dir.path().join("replica_state.json");
        
        let wal = Wal::new(&wal_path, 1024).unwrap();
        let wal = Arc::new(RwLock::new(wal));
        
        let agent = ReplicationAgent::new(
            "127.0.0.1:7879".to_string(),
            "test-replica".to_string(),
            state_path,
            Arc::clone(&wal),
        ).unwrap();
        
        // Create test entries
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
        
        // Apply entries
        agent.apply_entries(&entries).unwrap();
        
        // Verify entries were written
        let wal_size = wal.read().size().unwrap();
        assert!(wal_size > 0, "WAL should contain data");
    }
    
    #[test]
    fn test_estimate_size_kb() {
        let entries = vec![
            Entry {
                key: vec![0u8; 512],
                value: vec![0u8; 512],
                timestamp: 1000,
            },
        ];
        
        let size = estimate_size_kb(&entries);
        assert_eq!(size, 1); // 1KB
    }
    
    #[test]
    fn test_set_sync_interval() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("replica.wal");
        let state_path = dir.path().join("replica_state.json");
        
        let wal = Wal::new(&wal_path, 1024).unwrap();
        let wal = Arc::new(RwLock::new(wal));
        
        let mut agent = ReplicationAgent::new(
            "127.0.0.1:7879".to_string(),
            "test-replica".to_string(),
            state_path,
            wal,
        ).unwrap();
        
        // Default is 5 seconds
        assert_eq!(agent.sync_interval, Duration::from_secs(5));
        
        // Change interval
        agent.set_sync_interval(Duration::from_secs(10));
        assert_eq!(agent.sync_interval, Duration::from_secs(10));
    }
}