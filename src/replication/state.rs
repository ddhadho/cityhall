//! Replica state persistence
//!
//! Tracks which WAL segments have been successfully synced

use crate::{Result, StorageError};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::SystemTime;

/// Persistent state for a replica node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicaState {
    /// Unique identifier for this replica
    pub replica_id: String,

    /// Address of the leader node
    pub leader_addr: String,

    /// Last WAL segment successfully synced and applied
    ///
    /// Replica has all entries from segments <= this number
    pub last_synced_segment: u64,

    /// The leader's current (active) WAL segment number, as reported in handshake.
    /// This helps the replica understand the leader's progress.
    pub leader_current_segment: u64,

    /// Timestamp of last successful sync
    pub last_sync_time: u64, // Unix timestamp in seconds

    /// Total number of segments synced (metric)
    pub total_segments_synced: u64,

    /// Total number of entries applied (metric)
    pub total_entries_applied: u64,
}

impl ReplicaState {
    /// Create new replica state
    pub fn new(replica_id: String, leader_addr: String) -> Self {
        Self {
            replica_id,
            leader_addr,
            last_synced_segment: 0,    // Start from segment 1
            leader_current_segment: 0, // Initialized to 0, updated by handshake
            last_sync_time: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            total_segments_synced: 0,
            total_entries_applied: 0,
        }
    }

    /// Load state from file, or create new if doesn't exist
    pub fn load_or_create(path: &Path, replica_id: String, leader_addr: String) -> Result<Self> {
        if path.exists() {
            Self::load(path)
        } else {
            let state = Self::new(replica_id, leader_addr);
            state.save(path)?;
            Ok(state)
        }
    }

    /// Load state from JSON file
    pub fn load(path: &Path) -> Result<Self> {
        let mut file = File::open(path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;

        let state: ReplicaState = serde_json::from_str(&contents)
            .map_err(|e| StorageError::InvalidFormat(format!("Failed to parse state: {}", e)))?;

        println!(
            "📥 Loaded replica state: segment {}, {} entries applied",
            state.last_synced_segment, state.total_entries_applied
        );

        Ok(state)
    }

    /// Save state to JSON file (atomic write)
    pub fn save(&self, path: &Path) -> Result<()> {
        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Write to temporary file first
        let tmp_path = path.with_extension("tmp");

        let json = serde_json::to_string_pretty(self)
            .map_err(|e| StorageError::InvalidFormat(format!("Failed to serialize: {}", e)))?;

        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_path)?;

        file.write_all(json.as_bytes())?;
        file.sync_all()?; // Ensure data is on disk

        // Atomic rename
        std::fs::rename(&tmp_path, path)?;

        println!(
            "💾 Saved replica state: segment {}",
            self.last_synced_segment
        );

        Ok(())
    }

    /// Update after successfully syncing a segment
    pub fn update_synced_segment(&mut self, segment_number: u64, entries_count: u64) -> Result<()> {
        if segment_number <= self.last_synced_segment {
            return Err(StorageError::InvalidFormat(format!(
                "Cannot sync segment {} - already at segment {}",
                segment_number, self.last_synced_segment
            )));
        }

        self.last_synced_segment = segment_number;
        self.last_sync_time = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        self.total_segments_synced += 1;
        self.total_entries_applied += entries_count;

        Ok(())
    }

    /// Get next segment number to sync
    pub fn next_segment_to_sync(&self) -> u64 {
        self.last_synced_segment + 1
    }

    /// Check if state is stale (no sync in last N seconds)
    pub fn is_stale(&self, threshold_seconds: u64) -> bool {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        now - self.last_sync_time > threshold_seconds
    }

    /// Get human-readable status
    pub fn status(&self) -> String {
        let age = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - self.last_sync_time;

        format!(
            "Replica: {}\nLeader: {}\nLast Synced Segment: {}\nLast Sync: {} seconds ago\nTotal Segments: {}\nTotal Entries: {}",
            self.replica_id,
            self.leader_addr,
            self.last_synced_segment,
            age,
            self.total_segments_synced,
            self.total_entries_applied
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_state_new() {
        let state = ReplicaState::new("replica-1".to_string(), "127.0.0.1:7879".to_string());

        assert_eq!(state.replica_id, "replica-1");
        assert_eq!(state.leader_addr, "127.0.0.1:7879");
        assert_eq!(state.last_synced_segment, 0);
        assert_eq!(state.next_segment_to_sync(), 1);
    }

    #[test]
    fn test_state_persistence() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("replica_state.json");

        // Create and save
        let mut state = ReplicaState::new("test-replica".to_string(), "10.0.0.1:7879".to_string());

        state.update_synced_segment(5, 100).unwrap();
        state.save(&path).unwrap();

        // Load back
        let loaded = ReplicaState::load(&path).unwrap();

        assert_eq!(loaded.replica_id, "test-replica");
        assert_eq!(loaded.last_synced_segment, 5);
        assert_eq!(loaded.total_entries_applied, 100);
    }

    #[test]
    fn test_update_segment() {
        let mut state = ReplicaState::new("replica-1".to_string(), "127.0.0.1:7879".to_string());

        // Update to segment 1
        state.update_synced_segment(1, 50).unwrap();
        assert_eq!(state.last_synced_segment, 1);
        assert_eq!(state.total_entries_applied, 50);

        // Update to segment 2
        state.update_synced_segment(2, 75).unwrap();
        assert_eq!(state.last_synced_segment, 2);
        assert_eq!(state.total_entries_applied, 125);

        // Cannot go backwards
        assert!(state.update_synced_segment(1, 10).is_err());
    }

    #[test]
    fn test_load_or_create() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.json");

        // First call creates new
        let state1 =
            ReplicaState::load_or_create(&path, "r1".to_string(), "addr".to_string()).unwrap();

        assert_eq!(state1.last_synced_segment, 0);

        // Modify and save
        let mut state2 = state1.clone();
        state2.update_synced_segment(3, 200).unwrap();
        state2.save(&path).unwrap();

        // Second call loads existing
        let state3 = ReplicaState::load_or_create(
            &path,
            "different".to_string(), // Different ID, but loads existing
            "different".to_string(),
        )
        .unwrap();

        assert_eq!(state3.last_synced_segment, 3);
        assert_eq!(state3.replica_id, "r1"); // Kept original ID
    }

    #[test]
    fn test_atomic_save() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.json");

        let state = ReplicaState::new("r1".to_string(), "addr".to_string());
        state.save(&path).unwrap();

        // Should not have .tmp file after successful save
        let tmp_path = path.with_extension("tmp");
        assert!(!tmp_path.exists());

        // Should have actual file
        assert!(path.exists());
    }

    #[test]
    fn test_is_stale() {
        let mut state = ReplicaState::new("r1".to_string(), "addr".to_string());

        // Just created, not stale
        assert!(!state.is_stale(60));

        // Manually set old timestamp
        state.last_sync_time = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 120; // 120 seconds ago

        assert!(state.is_stale(60)); // Stale (> 60 seconds)
        assert!(!state.is_stale(180)); // Not stale (< 180 seconds)
    }
}
