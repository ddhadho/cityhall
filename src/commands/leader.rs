//! Leader command implementation
//!
//! Starts CityHall in leader mode with:
//! - Client server for writes/reads (using full StorageEngine)
//! - Replication server for serving replicas
//! - Shared WAL between StorageEngine and replication

use cityhall::replication::metrics::ReplicationMetrics;
use cityhall::replication::registry::ReplicaRegistry;
use cityhall::replication::ReplicationServer;
use cityhall::Result;
use cityhall::{http_server, StorageEngine};
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::signal;
use tokio::sync::RwLock;

/// Default MemTable size: 4MB
const DEFAULT_MEMTABLE_SIZE: usize = 4 * 1024 * 1024;

/// Run CityHall in leader mode
pub async fn run_leader(
    data_dir: PathBuf,
    port: u16,
    replication_port: u16,
    wal_buffer_size: usize,
) -> Result<()> {
    println!("🏙️  Starting CityHall in LEADER mode");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📁 Data directory: {:?}", data_dir);
    println!("🌐 Client port: {}", port);
    println!("🔄 Replication port: {}", replication_port);
    println!("💾 WAL buffer: {} bytes", wal_buffer_size);
    println!("📊 MemTable size: {} MB", DEFAULT_MEMTABLE_SIZE / 1_048_576);
    println!();

    let start_time = std::time::Instant::now();

    // Create data directory
    std::fs::create_dir_all(&data_dir)?;

    // Initialize WAL
    let wal_path = data_dir.join("wal");
    let wal = cityhall::Wal::new(&wal_path, wal_buffer_size)?;
    let wal = Arc::new(parking_lot::RwLock::new(wal));
    println!("✓ WAL initialized at {:?}", wal_path);

    // Create StorageEngine with shared WAL
    let storage_engine =
        StorageEngine::new(data_dir.clone(), DEFAULT_MEMTABLE_SIZE, Arc::clone(&wal))?;
    let storage = Arc::new(Mutex::new(storage_engine));
    println!("✓ StorageEngine initialized");

    // Initialize Replica Registry
    let replica_registry = Arc::new(ReplicaRegistry::new());
    println!("✓ Replica Registry initialized");

    // NEW: Create replication metrics
    let replication_metrics = Arc::new(ReplicationMetrics::new());
    println!("✓ Replication Metrics initialized");

    // Initialize shared current WAL segment (for dashboard)
    let current_wal_segment = Arc::new(RwLock::new(wal.read().current_segment_number())); // NEW
    println!(
        "✓ Current WAL segment tracker initialized (current: {})",
        *current_wal_segment.read().await
    );

    // Get WAL for replication (same instance used by StorageEngine)
    let replication_wal = {
        let engine = storage.lock();
        engine.get_wal()
    };

    // Start replication server
    // MODIFIED: Pass replica_registry to ReplicationServer::new
    let replication_server = ReplicationServer::new(
        replication_wal,
        Arc::clone(&replica_registry),
        replication_port,
    );
    let replication_handle = tokio::spawn(async move {
        if let Err(e) = replication_server.serve().await {
            eprintln!("❌ Replication server error: {}", e);
        }
    });
    println!("✓ Replication server started on port {}", replication_port);

    // Start Dashboard HTTP server
    let dashboard_handle = tokio::spawn(http_server::start_dashboard_server(
        Arc::clone(&replication_metrics),
        Arc::clone(&replica_registry),
        Arc::clone(&current_wal_segment),
        start_time,
    ));
    println!("✓ Dashboard HTTP server started");

    // Update current WAL segment periodically (for dashboard)
    let wal_clone = Arc::clone(&wal);
    let segment_tracker = Arc::clone(&current_wal_segment);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            let current = wal_clone.read().current_segment_number();
            *segment_tracker.write().await = current;
        }
    });
    println!("✓ WAL segment tracker background task started");

    // Start client server
    let client_addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&client_addr).await?;
    println!("✓ Client server started on {}", client_addr);

    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("✅ Leader is running!");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📝 Ready to accept reads/writes on port {}", port);
    println!("🔄 Serving replicas on port {}", replication_port);
    println!("📴 Press Ctrl+C to stop");
    println!();

    // Spawn client connection handler
    let client_handle = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    println!("🔗 Client connected: {}", addr);
                    let storage = Arc::clone(&storage);

                    tokio::spawn(async move {
                        if let Err(e) = handle_client_connection(stream, storage).await {
                            eprintln!("❌ Client connection error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    eprintln!("❌ Failed to accept client connection: {}", e);
                }
            }
        }
    });

    // Start dashboard with all parameters
    tokio::spawn(http_server::start_dashboard_server(
        Arc::clone(&replication_metrics),
        Arc::clone(&replica_registry),
        Arc::clone(&current_wal_segment),
        start_time, // NEW: Pass start time
    ));

    // Wait for shutdown signal
    tokio::select! {
        _ = signal::ctrl_c() => {
            println!();
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!("📴 Shutting down leader...");
        }
        _ = replication_handle => {
            println!("⚠️  Replication server stopped unexpectedly");
        }
        _ = client_handle => {
            println!("⚠️  Client server stopped unexpectedly");
        }
        _ = dashboard_handle => { // NEW
            println!("⚠️  Dashboard HTTP server stopped unexpectedly");
        }
    }

    println!("✓ Leader stopped");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    Ok(())
}

/// Handle a single client connection (PUT/GET commands)
///
/// Now uses the full StorageEngine for both writes and reads!
async fn handle_client_connection(
    stream: TcpStream,
    storage: Arc<Mutex<StorageEngine>>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line).await?;

        if bytes_read == 0 {
            // Connection closed
            return Ok(());
        }

        let parts: Vec<&str> = line.trim().splitn(3, ' ').collect();
        let cmd = parts.first().unwrap_or(&"");

        let writer = reader.get_mut();

        match *cmd {
            "PUT" => {
                if let (Some(key), Some(value)) = (parts.get(1), parts.get(2)) {
                    let key = key.as_bytes().to_vec();
                    let value = value.as_bytes().to_vec();

                    // Use StorageEngine.put() - handles WAL + MemTable + flush
                    let result = {
                        let mut engine = storage.lock();
                        engine.put(key.clone(), value.clone())
                    };

                    match result {
                        Ok(_) => {
                            writer.write_all(b"OK\n").await?;
                            println!("✓ PUT: {} bytes", key.len() + value.len());
                        }
                        Err(e) => {
                            let response = format!("ERROR: {}\n", e);
                            writer.write_all(response.as_bytes()).await?;
                            eprintln!("❌ PUT failed: {}", e);
                        }
                    }
                } else {
                    writer
                        .write_all(b"ERROR: invalid PUT format (use: PUT key value)\n")
                        .await?;
                }
            }

            "GET" => {
                if let Some(key) = parts.get(1) {
                    let key = key.as_bytes();

                    // Use StorageEngine.get() - checks MemTable + SSTables
                    let result = {
                        let mut engine = storage.lock();
                        engine.get(key)
                    };

                    match result {
                        Ok(Some(value)) => {
                            // Try to convert to UTF-8, otherwise use lossy conversion
                            let display = String::from_utf8_lossy(&value);
                            writer.write_all(display.as_bytes()).await?;
                            writer.write_all(b"\n").await?;
                            println!("✓ GET: found {} bytes", value.len());
                        }
                        Ok(None) => {
                            writer.write_all(b"NOT_FOUND\n").await?;
                            println!("⚠️  GET: key not found");
                        }
                        Err(e) => {
                            let response = format!("ERROR: {}\n", e);
                            writer.write_all(response.as_bytes()).await?;
                            eprintln!("❌ GET failed: {}", e);
                        }
                    }
                } else {
                    writer
                        .write_all(b"ERROR: invalid GET format (use: GET key)\n")
                        .await?;
                }
            }

            "" => {
                // Ignore empty lines
            }

            _ => {
                writer
                    .write_all(b"ERROR: unknown command (supported: PUT, GET)\n")
                    .await?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_leader_initialization() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();

        // Initialize WAL
        let wal_path = data_dir.join("wal");
        let wal = cityhall::Wal::new(&wal_path, 1024).unwrap();
        let wal = Arc::new(parking_lot::RwLock::new(wal));

        // Create StorageEngine
        let storage_engine =
            StorageEngine::new(data_dir.clone(), DEFAULT_MEMTABLE_SIZE, Arc::clone(&wal)).unwrap();

        let storage = Arc::new(Mutex::new(storage_engine));

        // Verify we can perform operations
        let mut engine = storage.lock();
        assert!(engine.put(b"test".to_vec(), b"value".to_vec()).is_ok());
        assert_eq!(engine.get(b"test").unwrap(), Some(b"value".to_vec()));
    }

    #[tokio::test]
    async fn test_wal_shared_with_storage_engine() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();

        // Initialize WAL
        let wal_path = data_dir.join("wal");
        let wal = cityhall::Wal::new(&wal_path, 1024).unwrap();
        let wal = Arc::new(parking_lot::RwLock::new(wal));

        // Create StorageEngine with shared WAL
        let storage_engine =
            StorageEngine::new(data_dir.clone(), DEFAULT_MEMTABLE_SIZE, Arc::clone(&wal)).unwrap();

        let storage = Arc::new(Mutex::new(storage_engine));

        // Get WAL from storage engine
        let _wal_from_engine = {
            let engine = storage.lock();
            engine.get_wal()
        };

        // Verify they point to the same WAL
        // (In Rust, Arc pointers are equal if they point to same allocation)
        assert_eq!(Arc::strong_count(&wal), 3); // original + storage_engine + wal_from_engine
    }

    #[tokio::test]
    async fn test_storage_engine_put_and_get() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();

        let wal_path = data_dir.join("wal");
        let wal = cityhall::Wal::new(&wal_path, 1024).unwrap();
        let wal = Arc::new(parking_lot::RwLock::new(wal));

        let storage_engine = StorageEngine::new(data_dir, DEFAULT_MEMTABLE_SIZE, wal).unwrap();

        let storage = Arc::new(Mutex::new(storage_engine));

        // Test PUT
        {
            let mut engine = storage.lock();
            engine.put(b"key1".to_vec(), b"value1".to_vec()).unwrap();
            engine.put(b"key2".to_vec(), b"value2".to_vec()).unwrap();
        }

        // Test GET
        {
            let mut engine = storage.lock();
            assert_eq!(engine.get(b"key1").unwrap(), Some(b"value1".to_vec()));
            assert_eq!(engine.get(b"key2").unwrap(), Some(b"value2".to_vec()));
            assert_eq!(engine.get(b"key3").unwrap(), None);
        }
    }
}
