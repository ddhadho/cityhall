use cityhall::replication::ReplicaState;
use tempfile::tempdir;

#[test]
fn test_replica_state_lifecycle() {
    let dir = tempdir().unwrap();
    let state_path = dir.path().join("replica.json");

    // Phase 1: Create new state
    let mut state = ReplicaState::new("replica-test".to_string(), "127.0.0.1:7879".to_string());

    assert_eq!(state.next_segment_to_sync(), 1);

    // Phase 2: Sync some segments
    state.update_synced_segment(1, 50).unwrap();
    state.update_synced_segment(2, 75).unwrap();
    state.save(&state_path).unwrap();

    // Phase 3: Load from disk
    let loaded = ReplicaState::load(&state_path).unwrap();
    assert_eq!(loaded.last_synced_segment, 2);
    assert_eq!(loaded.total_entries_applied, 125);
    assert_eq!(loaded.next_segment_to_sync(), 3);
}

#[test]
fn test_replica_state_validates_sequence() {
    let mut state = ReplicaState::new("r1".to_string(), "addr".to_string());

    // Can sync 1, 2, 3 in order
    assert!(state.update_synced_segment(1, 10).is_ok());
    assert!(state.update_synced_segment(2, 20).is_ok());
    assert!(state.update_synced_segment(3, 30).is_ok());

    // Cannot skip backwards
    assert!(state.update_synced_segment(2, 10).is_err());

    // Cannot skip the same
    assert!(state.update_synced_segment(3, 10).is_err());
}
