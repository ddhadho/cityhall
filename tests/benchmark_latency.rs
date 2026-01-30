use cityhall::replication::{ReplicationAgent, ReplicationServer};
use cityhall::{Entry, Result, StorageEngine, Wal};
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::time::sleep;

#[test]
#[ignore]
fn benchmark_write_latency() -> Result<()> {
    println!("\n=== Write Latency Benchmark ===\n");

    let temp_dir = TempDir::new()?;
    let path = temp_dir.path().to_path_buf();
    let wal_path = path.join("test.wal");
    let wal = Wal::new(&wal_path, 1024)?;
    let wal = Arc::new(RwLock::new(wal));
    let mut engine = StorageEngine::new(path, 200, wal)?.with_compaction(false);

    let mut latencies = Vec::new();

    // Write 1000 entries, measure each
    for i in 0..1000 {
        let key = format!("key{:06}", i);
        let value = vec![0u8; 100];

        let start = Instant::now();
        engine.put(key.into_bytes(), value)?;
        let duration = start.elapsed();

        latencies.push(duration.as_micros());
    }

    // Calculate stats
    latencies.sort();
    let p50 = latencies[latencies.len() / 2];
    let p99 = latencies[latencies.len() * 99 / 100];
    let p999 = latencies[latencies.len() * 999 / 1000];
    let max = latencies[latencies.len() - 1];

    println!("Write Latency Distribution:");
    println!("  p50:  {}μs", p50);
    println!("  p99:  {}μs", p99);
    println!("  p99.9: {}μs", p999);
    println!("  max:  {}μs", max);
    println!("\n✅ p99 < 1000μs (1ms): {}", p99 < 1000);

    Ok(())
}

#[tokio::test]
#[ignore]
async fn benchmark_replication_throughput() -> Result<()> {
    println!("\n=== Replication Throughput Benchmark ===\n");

    const NUM_ENTRIES: u64 = 10_000;
    const PORT: u16 = 17890;

    // 1. Setup leader and write a large number of entries
    let leader_dir = TempDir::new()?;
    let leader_wal_path = leader_dir.path().join("leader.wal");
    let mut leader_wal = Wal::new(&leader_wal_path, 1024 * 1024)?; // 1MB buffer
    leader_wal.set_segment_size_limit(10 * 1024 * 1024); // 10MB segments

    for i in 0..NUM_ENTRIES {
        let entry = Entry {
            key: format!("throughput.test.key.{}", i).into_bytes(),
            value: vec![b'x'; 256],
            timestamp: i,
        };
        leader_wal.append(&entry)?;
    }
    leader_wal.flush()?;
    let leader_wal = Arc::new(RwLock::new(leader_wal));

    // 2. Start replication server
    let server = ReplicationServer::new(Arc::clone(&leader_wal), PORT);
    let server_handle = tokio::spawn(async move {
        server.serve().await.unwrap();
    });
    sleep(Duration::from_millis(100)).await; // Give server time to start

    // 3. Setup replica
    let replica_dir = TempDir::new()?;
    let replica_wal_path = replica_dir.path().join("replica.wal");
    let replica_state_path = replica_dir.path().join("replica_state.json");
    let replica_wal = Wal::new(&replica_wal_path, 1024 * 1024)?;
    let replica_wal = Arc::new(RwLock::new(replica_wal));
    let mut agent = ReplicationAgent::new(
        format!("127.0.0.1:{}", PORT),
        "benchmark-replica".to_string(),
        replica_state_path,
        replica_wal,
    )?;

    // 4. Measure time to catch up
    let start_time = Instant::now();

    loop {
        match agent.sync_once().await? {
            true => { /* Continue syncing */ }
            false => break, // Caught up
        }
    }
    let elapsed = start_time.elapsed();

    // 5. Calculate and print throughput
    let total_entries = agent.state().total_entries_applied;
    let throughput = (total_entries as f64) / elapsed.as_secs_f64();

    println!("Replication Catch-Up Stats:");
    println!("  Total Entries: {}", total_entries);
    println!("  Time to Sync:  {:?}", elapsed);
    println!("  Throughput:    {:.2} entries/sec", throughput);

    assert_eq!(total_entries, NUM_ENTRIES);
    server_handle.abort();

    Ok(())
}
