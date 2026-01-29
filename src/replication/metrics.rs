//! Replication Metrics
//!
//! Tracks operational metrics for replication monitoring

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Serialize, Deserialize};
use crate::replication::registry::{ReplicaRegistry, ConnectionState};
use std::time::Instant;

/// Replication metrics for observability
#[derive(Debug)]
pub struct ReplicationMetrics {
    // Connection metrics
    connection_attempts: AtomicU64,
    connection_successes: AtomicU64,
    connection_failures: AtomicU64,
    
    // Sync metrics
    sync_attempts: AtomicU64,
    sync_successes: AtomicU64,
    sync_failures: AtomicU64,
    
    // Data metrics
    total_segments_synced: AtomicU64,
    total_entries_synced: AtomicU64,
    total_bytes_synced: AtomicU64,
    
    // Timing
    last_sync_timestamp: AtomicU64,
}

impl ReplicationMetrics {
    /// Create new metrics tracker
    pub fn new() -> Self {
        Self {
            connection_attempts: AtomicU64::new(0),
            connection_successes: AtomicU64::new(0),
            connection_failures: AtomicU64::new(0),
            sync_attempts: AtomicU64::new(0),
            sync_successes: AtomicU64::new(0),
            sync_failures: AtomicU64::new(0),
            total_segments_synced: AtomicU64::new(0),
            total_entries_synced: AtomicU64::new(0),
            total_bytes_synced: AtomicU64::new(0),
            last_sync_timestamp: AtomicU64::new(0),
        }
    }
    
    // Connection metrics
    
    pub fn record_connection_attempt(&self) {
        self.connection_attempts.fetch_add(1, Ordering::Relaxed);
    }
    
    pub fn record_connection_success(&self) {
        self.connection_successes.fetch_add(1, Ordering::Relaxed);
    }
    
    pub fn record_connection_failure(&self) {
        self.connection_failures.fetch_add(1, Ordering::Relaxed);
    }
    
    pub fn connection_attempts(&self) -> u64 {
        self.connection_attempts.load(Ordering::Relaxed)
    }
    
    pub fn connection_successes(&self) -> u64 {
        self.connection_successes.load(Ordering::Relaxed)
    }
    
    pub fn connection_failures(&self) -> u64 {
        self.connection_failures.load(Ordering::Relaxed)
    }
    
    pub fn connection_success_rate(&self) -> f64 {
        let attempts = self.connection_attempts();
        if attempts == 0 {
            return 1.0;
        }
        self.connection_successes() as f64 / attempts as f64
    }
    
    // Sync metrics
    
    pub fn record_sync_attempt(&self) {
        self.sync_attempts.fetch_add(1, Ordering::Relaxed);
    }
    
    pub fn record_sync_success(&self, entries: u64, bytes: u64) {
        self.sync_successes.fetch_add(1, Ordering::Relaxed);
        self.total_segments_synced.fetch_add(1, Ordering::Relaxed);
        self.total_entries_synced.fetch_add(entries, Ordering::Relaxed);
        self.total_bytes_synced.fetch_add(bytes, Ordering::Relaxed);
        
        // Update timestamp
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        self.last_sync_timestamp.store(now, Ordering::Relaxed);
    }
    
    pub fn record_sync_failure(&self) {
        self.sync_failures.fetch_add(1, Ordering::Relaxed);
    }
    
    pub fn sync_attempts(&self) -> u64 {
        self.sync_attempts.load(Ordering::Relaxed)
    }
    
    pub fn sync_successes(&self) -> u64 {
        self.sync_successes.load(Ordering::Relaxed)
    }
    
    pub fn sync_failures(&self) -> u64 {
        self.sync_failures.load(Ordering::Relaxed)
    }
    
    pub fn sync_success_rate(&self) -> f64 {
        let attempts = self.sync_attempts();
        if attempts == 0 {
            return 1.0;
        }
        self.sync_successes() as f64 / attempts as f64
    }
    
    // Data metrics
    
    pub fn total_segments_synced(&self) -> u64 {
        self.total_segments_synced.load(Ordering::Relaxed)
    }
    
    pub fn total_entries_synced(&self) -> u64 {
        self.total_entries_synced.load(Ordering::Relaxed)
    }
    
    pub fn total_bytes_synced(&self) -> u64 {
        self.total_bytes_synced.load(Ordering::Relaxed)
    }
    
    pub fn total_mb_synced(&self) -> f64 {
        self.total_bytes_synced() as f64 / 1_048_576.0
    }
    
    pub fn last_sync_timestamp(&self) -> u64 {
        self.last_sync_timestamp.load(Ordering::Relaxed)
    }
    
    pub fn seconds_since_last_sync(&self) -> Option<u64> {
        let last = self.last_sync_timestamp();
        if last == 0 {
            return None;
        }
        
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        Some(now.saturating_sub(last))
    }
    
    /// Get a formatted metrics report
    pub fn report(&self) -> String {
        let conn_rate = (self.connection_success_rate() * 100.0).round() as u32;
        let sync_rate = (self.sync_success_rate() * 100.0).round() as u32;
        
        let last_sync = match self.seconds_since_last_sync() {
            Some(s) => format!("{} seconds ago", s),
            None => "never".to_string(),
        };
        
        format!(
            "=== Replication Metrics ===
Connections:
  Attempts: {}
  Successes: {}
  Failures: {}
  Success rate: {}%

Syncs:
  Attempts: {}
  Successes: {}
  Failures: {}
  Success rate: {}%

Data:
  Segments synced: {}
  Entries synced: {}
  Data synced: {:.2} MB
  Last sync: {}",
            self.connection_attempts(),
            self.connection_successes(),
            self.connection_failures(),
            conn_rate,
            self.sync_attempts(),
            self.sync_successes(),
            self.sync_failures(),
            sync_rate,
            self.total_segments_synced(),
            self.total_entries_synced(),
            self.total_mb_synced(),
            last_sync
        )
    }

    /// Convert to dashboard-friendly format
    /// 
    /// This is called by the HTTP server to expose metrics to the web dashboard
    pub async fn to_dashboard_metrics(
        &self,
        replica_registry: &ReplicaRegistry,
        current_wal_segment: u64,
        start_time: Instant,  // Pass this in since we don't track it
    ) -> DashboardMetrics {
        // Get all replicas from registry
        let replicas_info = replica_registry.get_all().await;
        
        // Transform each replica into dashboard format
        let replicas = replicas_info.iter().map(|info| {
            let lag = current_wal_segment as i64 - info.last_segment_requested as i64;
            let last_seen = info.last_heartbeat.elapsed().as_secs();
            
            ReplicaDashboardInfo {
                replica_id: info.replica_id.clone(),
                status: format!("{:?}", info.connection_state),
                last_segment: info.last_segment_requested,
                lag_segments: lag,
                last_seen_ago_secs: last_seen,
                bytes_sent: info.bytes_sent,
            }
        }).collect();
        
        // Count connected replicas
        let connected_count = replicas_info.iter()
            .filter(|r| r.connection_state != ConnectionState::Offline)
            .count();
        
        // Calculate throughput
        let uptime_secs = start_time.elapsed().as_secs();
        let throughput = if uptime_secs > 0 {
            self.total_entries_synced() as f64 / uptime_secs as f64
        } else {
            0.0
        };
        
        DashboardMetrics {
            node_type: "Leader".to_string(),
            node_id: "leader-1".to_string(),
            uptime_seconds: uptime_secs,
            total_entries: self.total_entries_synced(),
            wal_segments: current_wal_segment,
            replicas,
            connected_count,
            throughput_entries_per_sec: throughput,
            bytes_synced_last_minute: self.total_bytes_synced(),
            last_updated: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        }
    }
}

/// Dashboard-friendly metrics format
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DashboardMetrics {
    pub node_type: String,
    pub node_id: String,
    pub uptime_seconds: u64,
    pub total_entries: u64,
    pub wal_segments: u64,
    pub replicas: Vec<ReplicaDashboardInfo>,
    pub connected_count: usize,
    pub throughput_entries_per_sec: f64,
    pub bytes_synced_last_minute: u64,
    pub last_updated: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ReplicaDashboardInfo {
    pub replica_id: String,
    pub status: String,
    pub last_segment: u64,
    pub lag_segments: i64,
    pub last_seen_ago_secs: u64,
    pub bytes_sent: u64,
}

impl Default for ReplicationMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;
    
    #[test]
    fn test_connection_metrics() {
        let metrics = ReplicationMetrics::new();
        
        assert_eq!(metrics.connection_attempts(), 0);
        assert_eq!(metrics.connection_success_rate(), 1.0);
        
        // Record some connections
        metrics.record_connection_attempt();
        metrics.record_connection_success();
        
        metrics.record_connection_attempt();
        metrics.record_connection_failure();
        
        metrics.record_connection_attempt();
        metrics.record_connection_success();
        
        assert_eq!(metrics.connection_attempts(), 3);
        assert_eq!(metrics.connection_successes(), 2);
        assert_eq!(metrics.connection_failures(), 1);
        assert_eq!(metrics.connection_success_rate(), 2.0 / 3.0);
    }
    
    #[test]
    fn test_sync_metrics() {
        let metrics = ReplicationMetrics::new();
        
        assert_eq!(metrics.sync_attempts(), 0);
        assert_eq!(metrics.sync_success_rate(), 1.0);
        
        // Record syncs
        metrics.record_sync_attempt();
        metrics.record_sync_success(100, 10240); // 100 entries, 10KB
        
        metrics.record_sync_attempt();
        metrics.record_sync_success(200, 20480); // 200 entries, 20KB
        
        metrics.record_sync_attempt();
        metrics.record_sync_failure();
        
        assert_eq!(metrics.sync_attempts(), 3);
        assert_eq!(metrics.sync_successes(), 2);
        assert_eq!(metrics.sync_failures(), 1);
        assert_eq!(metrics.sync_success_rate(), 2.0 / 3.0);
        
        assert_eq!(metrics.total_segments_synced(), 2);
        assert_eq!(metrics.total_entries_synced(), 300);
        assert_eq!(metrics.total_bytes_synced(), 30720);
    }
    
    #[test]
    fn test_data_metrics() {
        let metrics = ReplicationMetrics::new();
        
        // Sync 1MB of data
        metrics.record_sync_success(1000, 1_048_576);
        
        assert_eq!(metrics.total_segments_synced(), 1);
        assert_eq!(metrics.total_entries_synced(), 1000);
        assert_eq!(metrics.total_mb_synced(), 1.0);
    }
    
    #[test]
    fn test_last_sync_timestamp() {
        let metrics = ReplicationMetrics::new();
        
        // Initially no sync
        assert!(metrics.seconds_since_last_sync().is_none());
        
        // Record sync
        metrics.record_sync_success(10, 1024);
        
        // Should have recent timestamp
        let elapsed = metrics.seconds_since_last_sync().unwrap();
        assert!(elapsed < 2);
        
        // Wait a bit
        sleep(Duration::from_millis(100));
        
        // Time should increase
        let elapsed2 = metrics.seconds_since_last_sync().unwrap();
        assert!(elapsed2 >= elapsed);
    }
    
    #[test]
    fn test_metrics_report() {
        let metrics = ReplicationMetrics::new();
        
        // Add some data
        metrics.record_connection_attempt();
        metrics.record_connection_success();
        
        metrics.record_sync_attempt();
        metrics.record_sync_success(500, 51200);
        
        let report = metrics.report();
        
        assert!(report.contains("Attempts: 1"));
        assert!(report.contains("Successes: 1"));
        assert!(report.contains("Success rate: 100%"));
        assert!(report.contains("Segments synced: 1"));
        assert!(report.contains("Entries synced: 500"));
    }
    
    #[test]
    fn test_zero_attempts_success_rate() {
        let metrics = ReplicationMetrics::new();
        
        // With zero attempts, success rate should be 100%
        assert_eq!(metrics.connection_success_rate(), 1.0);
        assert_eq!(metrics.sync_success_rate(), 1.0);
    }
    
    #[test]
    fn test_concurrent_updates() {
        use std::sync::Arc;
        use std::thread;
        
        let metrics = Arc::new(ReplicationMetrics::new());
        let mut handles = vec![];
        
        // Spawn 10 threads, each recording 100 syncs
        for _ in 0..10 {
            let m = Arc::clone(&metrics);
            let handle = thread::spawn(move || {
                for _ in 0..100 {
                    m.record_sync_attempt();
                    m.record_sync_success(10, 1024);
                }
            });
            handles.push(handle);
        }
        
        // Wait for all threads
        for handle in handles {
            handle.join().unwrap();
        }
        
        // Should have 1000 attempts and successes
        assert_eq!(metrics.sync_attempts(), 1000);
        assert_eq!(metrics.sync_successes(), 1000);
        assert_eq!(metrics.total_entries_synced(), 10000);
    }
}