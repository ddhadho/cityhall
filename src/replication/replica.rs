//! Replication Agent (Replica Side) - Enhanced with Error Handling
//!
//! Syncs WAL segments from leader to replica with robust error handling:
//! - Exponential backoff for retries
//! - Configurable timeouts
//! - Auto-reconnect on failures
//! - Connection state tracking

use crate::{Entry, Result, StorageError, Wal};
use std::path::PathBuf;
use std::sync::Arc;
use parking_lot::RwLock;
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout, Duration};

use super::health::ReplicaHealth;      
use super::metrics::ReplicationMetrics; 

use super::protocol::{
    send_request, recv_response,
    SyncRequest, SyncResponse,
};
use super::state::ReplicaState;
use super::backoff::ExponentialBackoff;

/// Configuration for replication agent
#[derive(Debug, Clone)]
pub struct ReplicationConfig {
    /// Timeout for TCP connection
    pub connect_timeout: Duration,
    
    /// Timeout for reading responses
    pub read_timeout: Duration,
    
    /// Timeout for writing requests
    pub write_timeout: Duration,
    
    /// Interval between sync attempts
    pub sync_interval: Duration,
}

impl Default for ReplicationConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(30),
            write_timeout: Duration::from_secs(10),
            sync_interval: Duration::from_secs(5),
        }
    }
}

/// Connection state for tracking replica health
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Never connected yet
    Disconnected,
    
    /// Currently connected and syncing
    Connected,
    
    /// Connection failed, retrying with backoff
    Retrying,
    
    /// Too many failures, unhealthy
    Unhealthy,
}

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
    
    /// Configuration (timeouts, intervals)
    config: ReplicationConfig,
    
    /// Exponential backoff for retries
    backoff: ExponentialBackoff,
    
    /// Current connection state
    connection_state: ConnectionState,
    
    /// Number of consecutive failures
    consecutive_failures: u32,
    
    /// Maximum consecutive failures before marking unhealthy
    max_consecutive_failures: u32,

    health: ReplicaHealth,
    metrics: ReplicationMetrics,
}

impl ReplicationAgent {
    /// Create a new replication agent with default configuration
    pub fn new(
        leader_addr: String,
        replica_id: String,
        state_path: PathBuf,
        wal: Arc<RwLock<Wal>>,
    ) -> Result<Self> {
        Self::with_config(
            leader_addr,
            replica_id,
            state_path,
            wal,
            ReplicationConfig::default(),
        )
    }
    
    /// Create a new replication agent with custom configuration
    pub fn with_config(
        leader_addr: String,
        replica_id: String,
        state_path: PathBuf,
        wal: Arc<RwLock<Wal>>,
        config: ReplicationConfig,
    ) -> Result<Self> {
        // Load or create state
        let state = ReplicaState::load_or_create(
            &state_path,
            replica_id,
            leader_addr.clone(),
        )?;
        
        println!("📥 Replica initialized with error handling:");
        println!("   ID: {}", state.replica_id);
        println!("   Leader: {}", state.leader_addr);
        println!("   Last synced segment: {}", state.last_synced_segment);
        println!("   Connect timeout: {:?}", config.connect_timeout);
        println!("   Sync interval: {:?}", config.sync_interval);
        
        Ok(Self {
            leader_addr,
            state_path,
            state,
            wal,
            config,
            backoff: ExponentialBackoff::new(),
            connection_state: ConnectionState::Disconnected,
            consecutive_failures: 0,
            max_consecutive_failures: 10,
            health: ReplicaHealth::new(),        
            metrics: ReplicationMetrics::new(),
        })
    }
    
    /// Run sync loop (runs indefinitely with error handling)
    pub async fn run(&mut self) -> Result<()> {
        println!("🔄 Starting sync loop with error handling");
        println!("   Backoff: 1s → 60s (exponential)");
        println!("   Max consecutive failures: {}", self.max_consecutive_failures);
        
        loop {
            match self.sync_with_error_handling().await {
                Ok(true) => {
                    // Successfully synced a segment
                    self.on_sync_success();
                }
                Ok(false) => {
                    // No new segments (not an error)
                    self.on_sync_success();
                    println!("⏸️  No new segments to sync");
                }
                Err(e) => {
                    // Error occurred - apply backoff
                    self.on_sync_failure(&e).await;
                }
            }
            
            // Wait before next sync
            sleep(self.config.sync_interval).await;
        }
    }
    
    /// Sync with comprehensive error handling and retries
    async fn sync_with_error_handling(&mut self) -> Result<bool> {
        self.connection_state = ConnectionState::Retrying;
        
        // Try to sync with timeout on connection
        match self.connect_with_timeout().await {
            Ok(mut stream) => {
                self.connection_state = ConnectionState::Connected;
                
                // Perform sync on this connection
                match self.sync_once_with_stream(&mut stream).await {
                    Ok(result) => Ok(result),
                    Err(e) => {
                        self.connection_state = ConnectionState::Retrying;
                        Err(e)
                    }
                }
            }
            Err(e) => {
                self.connection_state = ConnectionState::Retrying;
                Err(e)
            }
        }
    }
    
    async fn connect_with_timeout(&self) -> Result<TcpStream> {
        self.metrics.record_connection_attempt();  // NEW
        
        let connect_future = TcpStream::connect(&self.leader_addr);
        
        match timeout(self.config.connect_timeout, connect_future).await {
            Ok(Ok(stream)) => {
                self.metrics.record_connection_success();  
                Ok(stream)
            }
            Ok(Err(e)) => {
                self.metrics.record_connection_failure(); 
                Err(StorageError::ConnectionFailed(format!(
                    "Failed to connect to {}: {}",
                    self.leader_addr, e
                )))
            }
            Err(_) => {
                self.metrics.record_connection_failure();  
                Err(StorageError::Timeout(format!(
                    "Connection timeout after {:?}",
                    self.config.connect_timeout
                )))
            }
        }
    }
    
    /// Perform single sync operation with an established connection
    async fn sync_once_with_stream(&mut self, stream: &mut TcpStream) -> Result<bool> {
        let next_segment = self.state.next_segment_to_sync();

        self.metrics.record_sync_attempt(); 
        
        println!("🔄 Syncing segment {} from {}", next_segment, self.leader_addr);
        
        // Send request with timeout
        let request = SyncRequest::GetSegment {
            segment_number: next_segment,
        };
        
        timeout(
            self.config.write_timeout,
            send_request(stream, &request)
        )
        .await
        .map_err(|_| StorageError::Timeout("Write timeout".into()))??;
        
        // Receive response with timeout
        let response = timeout(
            self.config.read_timeout,
            recv_response(stream)
        )
        .await
        .map_err(|_| StorageError::Timeout("Read timeout".into()))??
        .ok_or(StorageError::ConnectionClosed)?;
        
        // Process response
        match response {
            SyncResponse::SegmentData { segment_number, entries } => {
                println!(
                    "📥 Received segment {} ({} entries, ~{} KB)",
                    segment_number,
                    entries.len(),
                    estimate_size_kb(&entries)
                );
                
                // Apply entries to local WAL
                self.apply_entries(&entries)?;
                
                // Update state
                self.state.update_synced_segment(segment_number, entries.len() as u64)?;
                self.state.save(&self.state_path)?;
                
                println!("✓ Synced segment {} successfully", segment_number);
                
                Ok(true)
            }
            SyncResponse::SegmentNotFound { segment_number } => {
                println!(
                    "⚠️  Segment {} not found on leader (may be deleted or still active)",
                    segment_number
                );
                Ok(false)
            }
            SyncResponse::Error { message } => {
                Err(StorageError::LeaderError(message))
            }
            _ => {
                Err(StorageError::UnexpectedResponse)
            }
        }
    }
    
    /// Handle successful sync
    fn on_sync_success(&mut self) {
        self.backoff.reset();
        self.consecutive_failures = 0;
        self.connection_state = ConnectionState::Connected;

        self.health.record_success();
        // metrics updated in sync_once_with_stream
    }
    
    /// Handle sync failure with backoff
    async fn on_sync_failure(&mut self, error: &StorageError) {
        self.consecutive_failures += 1;

        self.health.record_failure(error);
        self.metrics.record_sync_failure();
        
        // Check if we've exceeded max failures
        if self.consecutive_failures >= self.max_consecutive_failures {
            self.connection_state = ConnectionState::Unhealthy;
            eprintln!(
                "❌ Replica UNHEALTHY: {} consecutive failures (max: {})",
                self.consecutive_failures,
                self.max_consecutive_failures
            );
        } else {
            self.connection_state = ConnectionState::Retrying;
        }
        
        // Calculate backoff
        let wait = self.backoff.next();
        
        eprintln!(
            "❌ Sync error (attempt {}): {}",
            self.backoff.attempts(),
            error
        );
        eprintln!(
            "   Retrying in {:?} (backoff: {:?})",
            wait,
            self.backoff.current()
        );
        
        // Apply backoff (but don't block the normal sync interval)
        // The caller will handle the sync_interval sleep
        if wait > self.config.sync_interval {
            sleep(wait - self.config.sync_interval).await;
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
    
    /// Perform a single sync operation (public API for manual sync)
    /// 
    /// This method tracks failures and updates connection state,
    /// making it suitable for testing and manual sync triggers.
    pub async fn sync_once(&mut self) -> Result<bool> {
        self.connection_state = ConnectionState::Retrying;
        
        match self.connect_with_timeout().await {
            Ok(mut stream) => {
                match self.sync_once_with_stream(&mut stream).await {
                    Ok(result) => {
                        // Success - reset failure tracking
                        self.on_sync_success();
                        Ok(result)
                    }
                    Err(e) => {
                        // Sync failed - track failure
                        self.consecutive_failures += 1;
                        if self.consecutive_failures >= self.max_consecutive_failures {
                            self.connection_state = ConnectionState::Unhealthy;
                        } else {
                            self.connection_state = ConnectionState::Retrying;
                        }
                        self.backoff.next(); // Increment backoff attempts
                        Err(e)
                    }
                }
            }
            Err(e) => {
                // Connection failed - track failure
                self.consecutive_failures += 1;
                if self.consecutive_failures >= self.max_consecutive_failures {
                    self.connection_state = ConnectionState::Unhealthy;
                } else {
                    self.connection_state = ConnectionState::Retrying;
                }
                self.backoff.next(); // Increment backoff attempts
                Err(e)
            }
        }
    }
    
    /// Get current replica state (for status/metrics)
    pub fn state(&self) -> &ReplicaState {
        &self.state
    }
    
    /// Get current connection state
    pub fn connection_state(&self) -> ConnectionState {
        self.connection_state
    }
    
    /// Get number of consecutive failures
    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }
    
    /// Check if replica is healthy
    pub fn is_healthy(&self) -> bool {
        self.connection_state != ConnectionState::Unhealthy
    }
    
    /// Get current backoff attempts
    pub fn backoff_attempts(&self) -> u64 {
        self.backoff.attempts()
    }
    
    /// Request list of available segments from leader
    pub async fn list_segments(&self) -> Result<(Vec<u64>, u64)> {
        let mut stream = self.connect_with_timeout().await?;
        
        let request = SyncRequest::ListSegments;
        
        timeout(
            self.config.write_timeout,
            send_request(&mut stream, &request)
        )
        .await
        .map_err(|_| StorageError::Timeout("Write timeout".into()))??;
        
        let response = timeout(
            self.config.read_timeout,
            recv_response(&mut stream)
        )
        .await
        .map_err(|_| StorageError::Timeout("Read timeout".into()))??
        .ok_or(StorageError::ConnectionClosed)?;
        
        match response {
            SyncResponse::SegmentList { segments, current_segment } => {
                Ok((segments, current_segment))
            }
            SyncResponse::Error { message } => {
                Err(StorageError::LeaderError(message))
            }
            _ => {
                Err(StorageError::UnexpectedResponse)
            }
        }
    }

    pub fn set_max_consecutive_failures(&mut self, n: u32) {
        self.max_consecutive_failures = n;
    }
    
    /// Get health status
    pub fn health(&self) -> &ReplicaHealth {
        &self.health
    }
    
    /// Get metrics
    pub fn metrics(&self) -> &ReplicationMetrics {
        &self.metrics
    }
    
    /// Get formatted status report
    pub fn status_report(&self) -> String {
        format!(
            "=== Replica Status ===
{}

{}

State: {:?}
Consecutive failures: {}
Backoff attempts: {}
Last synced segment: {}",
            self.health.status_report(),
            self.metrics.report(),
            self.connection_state,
            self.consecutive_failures,
            self.backoff.attempts(),
            self.state.last_synced_segment
        )
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
    fn test_replica_agent_creation_with_config() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("replica.wal");
        let state_path = dir.path().join("replica_state.json");
        
        let wal = Wal::new(&wal_path, 1024).unwrap();
        let wal = Arc::new(RwLock::new(wal));
        
        let config = ReplicationConfig {
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(60),
            write_timeout: Duration::from_secs(20),
            sync_interval: Duration::from_secs(3),
        };
        
        let agent = ReplicationAgent::with_config(
            "127.0.0.1:7879".to_string(),
            "test-replica".to_string(),
            state_path,
            wal,
            config.clone(),
        )
        .unwrap();
        
        assert_eq!(agent.config.connect_timeout, Duration::from_secs(10));
        assert_eq!(agent.config.sync_interval, Duration::from_secs(3));
        assert_eq!(agent.connection_state, ConnectionState::Disconnected);
        assert_eq!(agent.consecutive_failures, 0);
    }
    
    #[test]
    fn test_connection_state_tracking() {
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
        )
        .unwrap();
        
        // Initially disconnected
        assert_eq!(agent.connection_state(), ConnectionState::Disconnected);
        assert!(agent.is_healthy());
        
        // Simulate failures
        for i in 1..=5 {
            agent.consecutive_failures = i;
            assert!(agent.is_healthy()); // Still healthy
        }
        
        // Exceed threshold
        agent.consecutive_failures = agent.max_consecutive_failures;
        agent.connection_state = ConnectionState::Unhealthy;
        assert!(!agent.is_healthy());
        assert_eq!(agent.connection_state(), ConnectionState::Unhealthy);
    }
    
    #[test]
    fn test_backoff_reset_on_success() {
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
        )
        .unwrap();
        
        // Simulate some failures
        agent.backoff.next();
        agent.backoff.next();
        agent.consecutive_failures = 2;
        
        assert!(agent.backoff_attempts() > 0);
        
        // Success resets everything
        agent.on_sync_success();
        
        assert_eq!(agent.backoff_attempts(), 0);
        assert_eq!(agent.consecutive_failures, 0);
        assert_eq!(agent.connection_state, ConnectionState::Connected);
    }
    
    #[test]
    fn test_default_config() {
        let config = ReplicationConfig::default();
        
        assert_eq!(config.connect_timeout, Duration::from_secs(5));
        assert_eq!(config.read_timeout, Duration::from_secs(30));
        assert_eq!(config.write_timeout, Duration::from_secs(10));
        assert_eq!(config.sync_interval, Duration::from_secs(5));
    }
}