//! Replica connection registry
//!
//! Tracks active replica connections for monitoring and metrics

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::Instant;

/// Registry of all connected replicas
#[derive(Clone)]
pub struct ReplicaRegistry {
    replicas: Arc<RwLock<HashMap<String, ReplicaInfo>>>,
}

/// Information about a connected replica
#[derive(Clone, Debug)]
pub struct ReplicaInfo {
    /// Unique replica identifier
    pub replica_id: String,

    /// Last segment this replica requested
    pub last_segment_requested: u64,

    /// Connection state
    pub connection_state: ConnectionState,

    /// When was the last activity from this replica
    pub last_heartbeat: Instant,

    /// Network address of the replica
    pub remote_addr: String,

    /// Total bytes sent to this replica (approximate)
    pub bytes_sent: u64,

    /// When the replica first connected
    pub connected_at: Instant,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConnectionState {
    /// Just connected, handshake received
    Connected,

    /// Actively syncing segments
    Syncing,

    /// No recent activity
    Idle,

    /// Disconnected
    Offline,
}

impl ReplicaRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            replicas: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new replica connection
    pub async fn register(
        &self,
        replica_id: String,
        remote_addr: String,
        last_synced_segment: u64,
    ) {
        let mut replicas = self.replicas.write().await;

        let info = ReplicaInfo {
            replica_id: replica_id.clone(),
            last_segment_requested: last_synced_segment,
            connection_state: ConnectionState::Connected,
            last_heartbeat: Instant::now(),
            remote_addr,
            bytes_sent: 0,
            connected_at: Instant::now(),
        };

        replicas.insert(replica_id.clone(), info);

        println!(
            "Registered replica: {} (at segment {})",
            replica_id, last_synced_segment
        );
    }

    /// Update replica's last requested segment
    pub async fn update_segment(&self, replica_id: &str, segment: u64, bytes: u64) {
        let mut replicas = self.replicas.write().await;

        if let Some(info) = replicas.get_mut(replica_id) {
            info.last_segment_requested = segment;
            info.last_heartbeat = Instant::now();
            info.connection_state = ConnectionState::Syncing;
            info.bytes_sent += bytes;
        }
    }

    /// Mark replica as idle (still connected but no recent requests)
    pub async fn mark_idle(&self, replica_id: &str) {
        let mut replicas = self.replicas.write().await;

        if let Some(info) = replicas.get_mut(replica_id) {
            if info.connection_state != ConnectionState::Offline {
                info.connection_state = ConnectionState::Idle;
            }
        }
    }

    /// Mark replica as offline (disconnected)
    pub async fn mark_offline(&self, replica_id: &str) {
        let mut replicas = self.replicas.write().await;

        if let Some(info) = replicas.get_mut(replica_id) {
            info.connection_state = ConnectionState::Offline;
            println!("Replica offline: {}", replica_id);
        }
    }

    /// Get all replica information
    pub async fn get_all(&self) -> Vec<ReplicaInfo> {
        let replicas = self.replicas.read().await;
        replicas.values().cloned().collect()
    }

    /// Get specific replica info
    pub async fn get(&self, replica_id: &str) -> Option<ReplicaInfo> {
        let replicas = self.replicas.read().await;
        replicas.get(replica_id).cloned()
    }

    /// Get count of connected replicas (excluding offline)
    pub async fn connected_count(&self) -> usize {
        let replicas = self.replicas.read().await;
        replicas
            .values()
            .filter(|r| r.connection_state != ConnectionState::Offline)
            .count()
    }

    /// Remove old offline replicas (cleanup)
    pub async fn cleanup_offline(&self, max_age_secs: u64) {
        let mut replicas = self.replicas.write().await;

        let now = Instant::now();
        replicas.retain(|_, info| {
            if info.connection_state == ConnectionState::Offline {
                let age = now.duration_since(info.last_heartbeat).as_secs();
                age < max_age_secs
            } else {
                true
            }
        });
    }
}

impl Default for ReplicaRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_replica() {
        let registry = ReplicaRegistry::new();

        registry
            .register("replica-1".to_string(), "127.0.0.1:5000".to_string(), 5)
            .await;

        let replicas = registry.get_all().await;
        assert_eq!(replicas.len(), 1);
        assert_eq!(replicas[0].replica_id, "replica-1");
        assert_eq!(replicas[0].last_segment_requested, 5);
    }

    #[tokio::test]
    async fn test_update_segment() {
        let registry = ReplicaRegistry::new();

        registry
            .register("replica-1".to_string(), "127.0.0.1:5000".to_string(), 0)
            .await;

        registry.update_segment("replica-1", 10, 5000).await;

        let info = registry.get("replica-1").await.unwrap();
        assert_eq!(info.last_segment_requested, 10);
        assert_eq!(info.bytes_sent, 5000);
        assert_eq!(info.connection_state, ConnectionState::Syncing);
    }

    #[tokio::test]
    async fn test_mark_offline() {
        let registry = ReplicaRegistry::new();

        registry
            .register("replica-1".to_string(), "127.0.0.1:5000".to_string(), 0)
            .await;

        registry.mark_offline("replica-1").await;

        let info = registry.get("replica-1").await.unwrap();
        assert_eq!(info.connection_state, ConnectionState::Offline);
    }

    #[tokio::test]
    async fn test_connected_count() {
        let registry = ReplicaRegistry::new();

        registry
            .register("replica-1".to_string(), "addr1".to_string(), 0)
            .await;
        registry
            .register("replica-2".to_string(), "addr2".to_string(), 0)
            .await;
        registry
            .register("replica-3".to_string(), "addr3".to_string(), 0)
            .await;

        assert_eq!(registry.connected_count().await, 3);

        registry.mark_offline("replica-2").await;

        assert_eq!(registry.connected_count().await, 2);
    }
}
