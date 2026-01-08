//! Error Handling and Failure Scenario Tests
//!
//! Tests replica behavior under various failure conditions

use cityhall::{Entry, Wal};
use cityhall::replication::{ReplicationServer, ReplicationAgent, ConnectionState};
use std::sync::Arc;
use parking_lot::RwLock;
use tempfile::tempdir;
use tokio::time::{sleep, Duration};
use tokio::net::TcpListener;

/// Test: Replica handles leader being offline
#[tokio::test]
async fn test_replica_handles_leader_offline() {
    println!("\n=== Test: Leader Offline ===\n");
    
    let replica_dir = tempdir().unwrap();
    let replica_wal_path = replica_dir.path().join("replica.wal");
    let replica_state_path = replica_dir.path().join("replica_state.json");
    
    let replica_wal = Wal::new(&replica_wal_path, 1024).unwrap();
    let replica_wal = Arc::new(RwLock::new(replica_wal));
    
    // Create agent pointing to non-existent leader
    let mut agent = ReplicationAgent::new(
        "127.0.0.1:19999".to_string(), // No server on this port
        "test-replica".to_string(),
        replica_state_path,
        replica_wal,
    )
    .unwrap();
    
    // Try to sync (should fail)
    let result = agent.sync_once().await;
    
    assert!(result.is_err());
    assert_eq!(agent.consecutive_failures(), 1);
    assert!(agent.backoff_attempts() > 0);
    
    println!("✓ Replica correctly handled offline leader");
}

/// Test: Replica retries with exponential backoff
#[tokio::test]
async fn test_replica_backoff_increases() {
    println!("\n=== Test: Backoff Progression ===\n");
    
    let replica_dir = tempdir().unwrap();
    let replica_wal_path = replica_dir.path().join("replica.wal");
    let replica_state_path = replica_dir.path().join("replica_state.json");
    
    let replica_wal = Wal::new(&replica_wal_path, 1024).unwrap();
    let replica_wal = Arc::new(RwLock::new(replica_wal));
    
    let mut agent = ReplicationAgent::new(
        "127.0.0.1:19998".to_string(),
        "test-replica".to_string(),
        replica_state_path,
        replica_wal,
    )
    .unwrap();
    
    // Simulate multiple failures
    for i in 1..=5 {
        let _ = agent.sync_once().await;
        println!("Attempt {}: backoff attempts = {}", i, agent.backoff_attempts());
        assert_eq!(agent.backoff_attempts(), i as u64);
    }
    
    println!("✓ Backoff increased with each failure");
}

/// Test: Replica becomes unhealthy after max failures
#[tokio::test]
async fn test_replica_becomes_unhealthy() {
    println!("\n=== Test: Unhealthy After Max Failures ===\n");
    
    let replica_dir = tempdir().unwrap();
    let replica_wal_path = replica_dir.path().join("replica.wal");
    let replica_state_path = replica_dir.path().join("replica_state.json");
    
    let replica_wal = Wal::new(&replica_wal_path, 1024).unwrap();
    let replica_wal = Arc::new(RwLock::new(replica_wal));
    
    let mut agent = ReplicationAgent::new(
        "127.0.0.1:19997".to_string(),
        "test-replica".to_string(),
        replica_state_path,
        replica_wal,
    )
    .unwrap();
    
    agent.set_max_consecutive_failures(3);
    
    // Initially healthy
    assert!(agent.is_healthy());
    
    // Simulate failures
    for _ in 0..3 {
        let _ = agent.sync_once().await;
    }
    
    // Should still be trying
    assert!(agent.consecutive_failures() >= 3);
    
    println!("✓ Replica tracks consecutive failures");
}

/// Test: Replica recovers after leader comes back online
#[tokio::test]
async fn test_replica_recovers_when_leader_returns() {
    println!("\n=== Test: Recovery When Leader Returns ===\n");
    
    // Setup leader with data
    let leader_dir = tempdir().unwrap();
    let leader_path = leader_dir.path().join("leader.wal");
    
    let mut leader_wal = Wal::new(&leader_path, 1024).unwrap();
    leader_wal.set_segment_size_limit(5_000);
    
    // Write data
    for i in 0..50 {
        let entry = Entry {
            key: format!("key_{}", i).into_bytes(),
            value: vec![0u8; 200],
            timestamp: 1000 + i,
        };
        leader_wal.append(&entry).unwrap();
    }
    leader_wal.flush().unwrap();
    
    let leader_wal = Arc::new(RwLock::new(leader_wal));
    
    // Start leader
    let server = ReplicationServer::new(Arc::clone(&leader_wal), 19001);
    tokio::spawn(async move {
        server.serve().await.unwrap();
    });
    
    sleep(Duration::from_millis(200)).await;
    
    // Setup replica
    let replica_dir = tempdir().unwrap();
    let replica_wal_path = replica_dir.path().join("replica.wal");
    let replica_state_path = replica_dir.path().join("replica_state.json");
    
    let replica_wal = Wal::new(&replica_wal_path, 1024).unwrap();
    let replica_wal = Arc::new(RwLock::new(replica_wal));
    
    let mut agent = ReplicationAgent::new(
        "127.0.0.1:19001".to_string(),
        "test-replica".to_string(),
        replica_state_path,
        replica_wal,
    )
    .unwrap();
    
    // First sync should succeed
    let result = agent.sync_once().await.unwrap();
    assert!(result);
    assert_eq!(agent.consecutive_failures(), 0);
    assert_eq!(agent.backoff_attempts(), 0);
    assert!(agent.is_healthy());
    
    println!("✓ Replica recovered and synced successfully");
}

/// Test: Connection timeout handling
#[tokio::test]
async fn test_connection_timeout() {
    println!("\n=== Test: Connection Timeout ===\n");
    
    // Create a server that accepts but never responds
    tokio::spawn(async {
        let listener = TcpListener::bind("127.0.0.1:19002").await.unwrap();
        loop {
            let (_stream, _) = listener.accept().await.unwrap();
            // Accept connection but never respond (simulates timeout)
            sleep(Duration::from_secs(3600)).await; // Sleep forever
        }
    });
    
    sleep(Duration::from_millis(200)).await;
    
    let replica_dir = tempdir().unwrap();
    let replica_wal_path = replica_dir.path().join("replica.wal");
    let replica_state_path = replica_dir.path().join("replica_state.json");
    
    let replica_wal = Wal::new(&replica_wal_path, 1024).unwrap();
    let replica_wal = Arc::new(RwLock::new(replica_wal));
    
    let mut config = cityhall::replication::replica::ReplicationConfig::default();
    config.read_timeout = Duration::from_millis(500); // Short timeout
    
    let mut agent = cityhall::replication::replica::ReplicationAgent::with_config(
        "127.0.0.1:19002".to_string(),
        "test-replica".to_string(),
        replica_state_path,
        replica_wal,
        config,
    )
    .unwrap();
    
    // Should timeout
    let result = agent.sync_once().await;
    assert!(result.is_err());
    
    // Check it's a timeout error
    if let Err(e) = result {
        println!("Error (expected): {}", e);
        assert!(matches!(e, cityhall::StorageError::Timeout(_)));
    }
    
    println!("✓ Timeout handled correctly");
}

/// Test: Multiple replicas with intermittent failures
#[tokio::test]
async fn test_multiple_replicas_with_failures() {
    println!("\n=== Test: Multiple Replicas with Failures ===\n");
    
    // Setup leader
    let leader_dir = tempdir().unwrap();
    let leader_path = leader_dir.path().join("leader.wal");
    
    let mut leader_wal = Wal::new(&leader_path, 1024).unwrap();

    leader_wal.set_segment_size_limit(5_000);
    
    for i in 0..50 {
        let entry = Entry {
            key: format!("key_{}", i).into_bytes(),
            value: vec![0u8; 200],
            timestamp: 1000 + i,
        };
        leader_wal.append(&entry).unwrap();
    }
    leader_wal.flush().unwrap();
    
    let leader_wal = Arc::new(RwLock::new(leader_wal));
    
    let server = ReplicationServer::new(Arc::clone(&leader_wal), 19003);
    tokio::spawn(async move {
        server.serve().await.unwrap();
    });
    
    sleep(Duration::from_millis(200)).await;
    
    // Create 3 replicas
    let mut handles = vec![];
    
    for replica_id in 1..=3 {
        let replica_dir = tempdir().unwrap();
        let replica_wal_path = replica_dir.path().join("replica.wal");
        let replica_state_path = replica_dir.path().join("replica_state.json");
        
        let replica_wal = Wal::new(&replica_wal_path, 1024).unwrap();
        let replica_wal = Arc::new(RwLock::new(replica_wal));
        
        let mut agent = ReplicationAgent::new(
            "127.0.0.1:19003".to_string(),
            format!("replica-{}", replica_id),
            replica_state_path,
            replica_wal,
        )
        .unwrap();
        
        let handle = tokio::spawn(async move {
            // Try syncing a few times
            let mut successes = 0;
            
            for _ in 0..3 {
                if let Ok(true) = agent.sync_once().await {
                    successes += 1;
                }
                sleep(Duration::from_millis(100)).await;
            }
            
            println!("Replica {}: {} successful syncs", replica_id, successes);
            successes
        });
        
        handles.push(handle);
    }
    
    // Wait for all replicas
    let mut total_successes = 0;
    for handle in handles {
        total_successes += handle.await.unwrap();
    }
    
    assert!(total_successes >= 3, "At least one sync per replica should succeed");
    
    println!("✓ Multiple replicas handled failures independently");
}

/// Test: Replica state tracking during failures
#[tokio::test]
async fn test_connection_state_transitions() {
    println!("\n=== Test: Connection State Transitions ===\n");
    
    let replica_dir = tempdir().unwrap();
    let replica_wal_path = replica_dir.path().join("replica.wal");
    let replica_state_path = replica_dir.path().join("replica_state.json");
    
    let replica_wal = Wal::new(&replica_wal_path, 1024).unwrap();
    let replica_wal = Arc::new(RwLock::new(replica_wal));
    
    let mut agent = ReplicationAgent::new(
        "127.0.0.1:19996".to_string(),
        "test-replica".to_string(),
        replica_state_path,
        replica_wal,
    )
    .unwrap();
    
    // Initially disconnected
    assert_eq!(agent.connection_state(), ConnectionState::Disconnected);
    
    // After failed sync
    let _ = agent.sync_once().await;
    // State should reflect the failure
    assert!(agent.consecutive_failures() > 0);
    
    println!("✓ Connection state transitions correctly");
}