//! Server command implementation
//!
//! Starts CityHall in standalone server mode with:
//! - Client server for writes/reads (using full StorageEngine)
//! - Shared WAL between StorageEngine and compaction
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

/// Run CityHall in server mode
pub async fn run_server(
    data_dir: PathBuf,
    port: u16,
    wal_buffer_size: usize,
) -> Result<()> {
    println!("🏙️  Starting CityHall");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📁 Data directory: {:?}", data_dir);
    println!("🌐 Client port:    {}", port);
    println!("💾 WAL buffer:     {} bytes", wal_buffer_size);
    println!("📊 MemTable size:  {} MB", DEFAULT_MEMTABLE_SIZE / 1_048_576);
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

    // Initialize shared current WAL segment (for dashboard)
    let current_wal_segment = Arc::new(RwLock::new(wal.read().current_segment_number()));
    println!(
        "✓ WAL segment tracker initialized (current: {})",
        *current_wal_segment.read().await
    );

    // Start Dashboard HTTP server
    let dashboard_handle = tokio::spawn(http_server::start_dashboard_server(
        Arc::clone(&current_wal_segment),
        start_time,
    ));
    println!("✓ Dashboard started at http://localhost:8080/dashboard");

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

    // Start client TCP server
    let client_addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&client_addr).await?;
    println!("✓ TCP server listening on {}", client_addr);

    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("✅ CityHall is running");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("   Supported commands: PUT, GET");
    println!("   Press Ctrl+C to stop");
    println!();

    // Spawn client connection handler
    let _client_handle = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    println!("🔗 Client connected: {}", addr);
                    let storage = Arc::clone(&storage);
                    tokio::spawn(async move {
                        if let Err(e) = handle_client_connection(stream, storage).await {
                            eprintln!("❌ Client error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    eprintln!("❌ Failed to accept connection: {}", e);
                }
            }
        }
    });

    // Wait for shutdown signal
    tokio::select! {
        _ = signal::ctrl_c() => {
            println!();
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!("📴 Shutting down...");
        }
        _ = dashboard_handle => {
            println!("⚠️  Dashboard stopped unexpectedly");
        }
    }

    println!("✓ Server stopped");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    Ok(())
}

/// Handle a single client connection
///
/// Supported commands:
///   PUT <key> <value>  — write a key-value pair
///   GET <key>          — read a value by key
///   DELETE <key>       — not yet implemented (tombstone support pending)
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
            return Ok(());
        }

        let parts: Vec<&str> = line.trim().splitn(3, ' ').collect();
        let cmd = parts.first().map(|s| s.to_uppercase());
        let writer = reader.get_mut();

        match cmd.as_deref() {
            Some("PUT") => {
                if let (Some(key), Some(value)) = (parts.get(1), parts.get(2)) {
                    let key = key.as_bytes().to_vec();
                    let value = value.as_bytes().to_vec();

                    let result = {
                        let mut engine = storage.lock();
                        engine.put(key.clone(), value.clone())
                    };

                    match result {
                        Ok(_) => {
                            writer.write_all(b"OK\n").await?;
                        }
                        Err(e) => {
                            writer
                                .write_all(format!("ERROR {}\n", e).as_bytes())
                                .await?;
                            eprintln!("❌ PUT failed: {}", e);
                        }
                    }
                } else {
                    writer
                        .write_all(b"ERROR usage: PUT <key> <value>\n")
                        .await?;
                }
            }

            Some("GET") => {
                if let Some(key) = parts.get(1) {
                    let result = {
                        let mut engine = storage.lock();
                        engine.get(key.as_bytes())
                    };

                    match result {
                        Ok(Some(value)) => {
                            let display = String::from_utf8_lossy(&value);
                            writer
                                .write_all(format!("VALUE {}\n", display).as_bytes())
                                .await?;
                        }
                        Ok(None) => {
                            writer.write_all(b"NOT_FOUND\n").await?;
                        }
                        Err(e) => {
                            writer
                                .write_all(format!("ERROR {}\n", e).as_bytes())
                                .await?;
                            eprintln!("❌ GET failed: {}", e);
                        }
                    }
                } else {
                    writer.write_all(b"ERROR usage: GET <key>\n").await?;
                }
            }

            // DELETE is defined in the CLI and client but requires tombstone
            // support in the storage engine. Tracked in the project roadmap.
            Some("DELETE") => {
                writer
                    .write_all(b"ERROR DELETE not yet implemented (tombstone support pending)\n")
                    .await?;
            }

            Some("") | None => {
                // ignore empty lines
            }

            _ => {
                writer
                    .write_all(b"ERROR unknown command-supported: PUT, GET, DELETE\n")
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
    async fn test_server_initialization() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();

        let wal_path = data_dir.join("wal");
        let wal = cityhall::Wal::new(&wal_path, 1024).unwrap();
        let wal = Arc::new(parking_lot::RwLock::new(wal));

        let storage_engine =
            StorageEngine::new(data_dir.clone(), DEFAULT_MEMTABLE_SIZE, Arc::clone(&wal))
                .unwrap();

        let storage = Arc::new(Mutex::new(storage_engine));
        let mut engine = storage.lock();
        assert!(engine.put(b"test".to_vec(), b"value".to_vec()).is_ok());
        assert_eq!(engine.get(b"test").unwrap(), Some(b"value".to_vec()));
    }

    #[tokio::test]
    async fn test_put_and_get() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();

        let wal_path = data_dir.join("wal");
        let wal = cityhall::Wal::new(&wal_path, 1024).unwrap();
        let wal = Arc::new(parking_lot::RwLock::new(wal));

        let storage_engine =
            StorageEngine::new(data_dir.clone(), DEFAULT_MEMTABLE_SIZE, wal).unwrap();
        let storage = Arc::new(Mutex::new(storage_engine));

        let mut engine = storage.lock();
        engine.put(b"k1".to_vec(), b"v1".to_vec()).unwrap();
        engine.put(b"k2".to_vec(), b"v2".to_vec()).unwrap();

        assert_eq!(engine.get(b"k1").unwrap(), Some(b"v1".to_vec()));
        assert_eq!(engine.get(b"k2").unwrap(), Some(b"v2".to_vec()));
        assert_eq!(engine.get(b"k3").unwrap(), None);
    }
}