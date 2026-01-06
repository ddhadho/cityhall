//! End-to-End Replication Integration Tests
//!
//! Tests the complete replication flow: Leader → Replica

use cityhall::{Entry, Wal};
use cityhall::replication::{ReplicationServer, ReplicationAgent};
use std::sync::Arc;
use parking_lot::RwLock;
use tempfile::tempdir;
use tokio::time::{sleep, Duration};

/// Test: Basic end-to-end replication
/// 
/// 1. Setup leader with data
/// 2. Start replication server
/// 3. Create replica
/// 4. Replica syncs one segment
/// 5. Verify replica has the data
#[tokio::test]
async fn test_basic_replication() {
    println!("\n=== Test: Basic Replication ===\n");
    
    // Setup leader
    let leader_dir = tempdir().unwrap();
    let leader_path = leader_dir.path().join("leader.wal");
    
    let mut leader_wal = Wal::new(&leader_path, 1024).unwrap();
    leader_wal.set_segment_size_limit(5_000);
    
    // Write data to leader
    println!("📝 Leader: Writing test data...");
    for i in 0..50 {
        let entry = Entry {
            key: format!("metric.cpu.{}", i).into_bytes(),
            value: vec![b'x'; 100], 
            timestamp: 1000 + i,
        };
        leader_wal.append(&entry).unwrap();
    }
    leader_wal.flush().unwrap();
    
    let segments = leader_wal.list_closed_segments().unwrap();
    println!("📊 Leader: Created {} closed segments", segments.len());
    assert!(segments.len() > 0, "Need at least one closed segment");
    
    let leader_wal = Arc::new(RwLock::new(leader_wal));
    
    // Start replication server
    println!("🌐 Starting replication server on port 16879...");
    let server = ReplicationServer::new(Arc::clone(&leader_wal), 16879);
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
    
    println!("📥 Creating replica agent...");
    let mut agent = ReplicationAgent::new(
        "127.0.0.1:16879".to_string(),
        "test-replica-1".to_string(),
        replica_state_path.clone(),
        Arc::clone(&replica_wal),
    ).unwrap();
    
    // Sync one segment
    println!("🔄 Replica: Syncing segment 1...");
    let synced = agent.sync_once().await.unwrap();
    assert!(synced, "Should have synced a segment");
    
    // Verify replica state updated
    assert_eq!(agent.state().last_synced_segment, 1);
    assert_eq!(agent.state().next_segment_to_sync(), 2);
    
    // Verify replica WAL has data
    let replica_size = replica_wal.read().size().unwrap();
    println!("📊 Replica WAL size: {} bytes", replica_size);
    assert!(replica_size > 0, "Replica should have data");
    
    println!("✅ Basic replication test passed!\n");
}

/// Test: Replica catches up after being offline
#[tokio::test]
async fn test_replica_catchup() {
    println!("\n=== Test: Replica Catch-Up ===\n");
    
    // Setup leader
    let leader_dir = tempdir().unwrap();
    let leader_path = leader_dir.path().join("leader.wal");
    
    let mut leader_wal = Wal::new(&leader_path, 1024).unwrap();
    leader_wal.set_segment_size_limit(3_000);

    
    // Write initial data
    println!("📝 Leader: Writing initial data (segments 1-2)...");
    for i in 0..40 {
        let entry = Entry {
            key: format!("key_{}", i).into_bytes(),
            value: vec![0u8; 150],
            timestamp: 1000 + i,
        };
        leader_wal.append(&entry).unwrap();
    }
    leader_wal.flush().unwrap();
    
    let initial_segments = leader_wal.list_closed_segments().unwrap();
    println!("📊 Leader: {} closed segments", initial_segments.len());
    
    let leader_wal = Arc::new(RwLock::new(leader_wal));
    
    // Start server
    let server = ReplicationServer::new(Arc::clone(&leader_wal), 16880);
    tokio::spawn(async move {
        server.serve().await.unwrap();
    });
    
    sleep(Duration::from_millis(200)).await;
    
    // Setup replica and sync segment 1
    let replica_dir = tempdir().unwrap();
    let replica_wal_path = replica_dir.path().join("replica.wal");
    let replica_state_path = replica_dir.path().join("replica_state.json");
    
    let replica_wal = Wal::new(&replica_wal_path, 1024).unwrap();
    let replica_wal = Arc::new(RwLock::new(replica_wal));
    
    let mut agent = ReplicationAgent::new(
        "127.0.0.1:16880".to_string(),
        "test-replica-2".to_string(),
        replica_state_path.clone(),
        Arc::clone(&replica_wal),
    ).unwrap();
    
    println!("🔄 Replica: Syncing segment 1...");
    agent.sync_once().await.unwrap();
    assert_eq!(agent.state().last_synced_segment, 1);
    
    // Leader writes more data while replica is "offline"
    println!("📝 Leader: Writing more data (segments 3-4) while replica offline...");
    {
        let mut wal = leader_wal.write();
        for i in 40..80 {
            let entry = Entry {
                key: format!("key_{}", i).into_bytes(),
                value: vec![0u8; 150],
                timestamp: 1000 + i,
            };
            wal.append(&entry).unwrap();
        }
        wal.flush().unwrap();
    }
    
    let new_segments = leader_wal.read().list_closed_segments().unwrap();
    println!("📊 Leader: Now has {} closed segments", new_segments.len());
    
    // Replica comes back online and catches up
    println!("🔄 Replica: Catching up...");
    loop {
        match agent.sync_once().await {
            Ok(true) => {
                println!("   Synced segment {}", agent.state().last_synced_segment);
            }
            Ok(false) => {
                println!("   Caught up!");
                break;
            }
            Err(e) => {
                panic!("Sync failed: {}", e);
            }
        }
    }
    
    // Verify replica caught up
    let last_synced = agent.state().last_synced_segment;
    println!("✓ Replica synced up to segment {}", last_synced);
    assert!(last_synced >= initial_segments.len() as u64);
    
    println!("✅ Catch-up test passed!\n");
}

/// Test: Multiple replicas sync simultaneously
#[tokio::test]
async fn test_multiple_replicas() {
    println!("\n=== Test: Multiple Replicas ===\n");
    
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
    
    // Start server
    let server = ReplicationServer::new(Arc::clone(&leader_wal), 16881);
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
            "127.0.0.1:16881".to_string(),
            format!("replica-{}", replica_id),
            replica_state_path,
            replica_wal,
        ).unwrap();
        
        let handle = tokio::spawn(async move {
            println!("🔄 Replica {}: Syncing...", replica_id);
            
            let synced = agent.sync_once().await.unwrap();
            assert!(synced);
            
            println!("✓ Replica {}: Synced segment {}", 
                     replica_id, 
                     agent.state().last_synced_segment);
            
            agent.state().last_synced_segment
        });
        
        handles.push(handle);
    }
    
    // Wait for all replicas
    for handle in handles {
        let segment = handle.await.unwrap();
        assert_eq!(segment, 1);
    }
    
    println!("✅ Multiple replicas test passed!\n");
}

/// Test: List segments from replica
#[tokio::test]
async fn test_replica_list_segments() {
    println!("\n=== Test: Replica List Segments ===\n");
    
    let leader_dir = tempdir().unwrap();
    let leader_path = leader_dir.path().join("leader.wal");
    
    let mut leader_wal = Wal::new(&leader_path, 1024).unwrap();
    leader_wal.set_segment_size_limit(3_000);

    
    for i in 0..60 {
        let entry = Entry {
            key: format!("key_{}", i).into_bytes(),
            value: vec![0u8; 100],
            timestamp: 1000 + i,
        };
        leader_wal.append(&entry).unwrap();
    }
    leader_wal.flush().unwrap();
    
    let expected_segments = leader_wal.list_closed_segments().unwrap();
    let expected_current = leader_wal.current_segment_number();
    
    let leader_wal = Arc::new(RwLock::new(leader_wal));
    
    let server = ReplicationServer::new(Arc::clone(&leader_wal), 16882);
    tokio::spawn(async move {
        server.serve().await.unwrap();
    });
    
    sleep(Duration::from_millis(200)).await;
    
    // Create replica
    let replica_dir = tempdir().unwrap();
    let replica_wal_path = replica_dir.path().join("replica.wal");
    let replica_state_path = replica_dir.path().join("replica_state.json");
    
    let replica_wal = Wal::new(&replica_wal_path, 1024).unwrap();
    let replica_wal = Arc::new(RwLock::new(replica_wal));
    
    let agent = ReplicationAgent::new(
        "127.0.0.1:16882".to_string(),
        "test-replica".to_string(),
        replica_state_path,
        replica_wal,
    ).unwrap();
    
    // List segments
    let (segments, current) = agent.list_segments().await.unwrap();
    
    println!("📋 Available segments: {:?}", segments);
    println!("📊 Current segment: {}", current);
    
    assert_eq!(segments, expected_segments);
    assert_eq!(current, expected_current);
    
    println!("✅ List segments test passed!\n");
}

/// Test: Replica handles segment not found
#[tokio::test]
async fn test_replica_segment_not_found() {
    println!("\n=== Test: Segment Not Found ===\n");
    
    let leader_dir = tempdir().unwrap();
    let leader_path = leader_dir.path().join("leader.wal");
    
    let leader_wal = Wal::new(&leader_path, 1024).unwrap();
    let leader_wal = Arc::new(RwLock::new(leader_wal));
    
    let server = ReplicationServer::new(Arc::clone(&leader_wal), 16883);
    tokio::spawn(async move {
        server.serve().await.unwrap();
    });
    
    sleep(Duration::from_millis(200)).await;
    
    let replica_dir = tempdir().unwrap();
    let replica_wal_path = replica_dir.path().join("replica.wal");
    let replica_state_path = replica_dir.path().join("replica_state.json");
    
    let replica_wal = Wal::new(&replica_wal_path, 1024).unwrap();
    let replica_wal = Arc::new(RwLock::new(replica_wal));
    
    let mut agent = ReplicationAgent::new(
        "127.0.0.1:16883".to_string(),
        "test-replica".to_string(),
        replica_state_path,
        replica_wal,
    ).unwrap();
    
    // Try to sync (leader has no segments)
    let result = agent.sync_once().await.unwrap();
    
    // Should return false (nothing to sync)
    assert!(!result);
    
    // State should not have changed
    assert_eq!(agent.state().last_synced_segment, 0);
    
    println!("✅ Segment not found test passed!\n");
}