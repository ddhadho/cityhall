//! Leader command implementation
//!
//! Starts CityHall in leader mode with:
//! - Client server for writes (existing server code)
//! - Replication server for serving replicas
//! - Shared WAL for both write operations and replication

use cityhall::{Result, Wal, Entry};
use cityhall::replication::ReplicationServer;
use std::path::PathBuf;
use std::sync::Arc;
use parking_lot::RwLock;
use tokio::signal;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

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
    println!();
    
    // Create data directory
    std::fs::create_dir_all(&data_dir)?;
    
    // Initialize WAL - this is shared between client operations and replication
    let wal_path = data_dir.join("wal");
    let wal = Wal::new(&wal_path, wal_buffer_size)?;
    let wal = Arc::new(RwLock::new(wal));
    println!("✓ WAL initialized at {:?}", wal_path);
    
    // Start replication server
    let replication_wal = Arc::clone(&wal);
    let replication_server = ReplicationServer::new(replication_wal, replication_port);
    
    let replication_handle = tokio::spawn(async move {
        if let Err(e) = replication_server.serve().await {
            eprintln!("❌ Replication server error: {}", e);
        }
    });
    
    println!("✓ Replication server started on port {}", replication_port);
    
    // Start client server
    let client_addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&client_addr).await?;
    println!("✓ Client server started on {}", client_addr);
    
    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("✅ Leader is running!");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📝 Ready to accept writes on port {}", port);
    println!("🔄 Serving replicas on port {}", replication_port);
    println!("📴 Press Ctrl+C to stop");
    println!();
    
    // Spawn client connection handler
    let client_wal = Arc::clone(&wal);
    let client_handle = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    println!("🔗 Client connected: {}", addr);
                    let wal = Arc::clone(&client_wal);
                    
                    tokio::spawn(async move {
                        if let Err(e) = handle_client_connection(stream, wal).await {
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
    }
    
    println!("✓ Leader stopped");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    
    Ok(())
}

/// Handle a single client connection (PUT/GET commands)
/// 
/// This is a simple WAL-based write handler.
/// For production, you might want to integrate with your full StorageEngine.
async fn handle_client_connection(
    stream: TcpStream,
    wal: Arc<RwLock<Wal>>,
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
        let cmd = parts.get(0).unwrap_or(&"");
        
        let writer = reader.get_mut();
        
        match *cmd {
            "PUT" => {
                if let (Some(key), Some(value)) = (parts.get(1), parts.get(2)) {
                    // Create entry
                    let entry = Entry {
                        key: key.as_bytes().to_vec(),
                        value: value.as_bytes().to_vec(),
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs(),
                    };
                    
                    // Prepare response while holding lock, then drop it
                    let response = {
                        let mut wal_lock = wal.write();
                        match wal_lock.append(&entry) {
                            Ok(_) => {
                                match wal_lock.flush() {
                                    Ok(_) => "OK\n".to_string(),
                                    Err(e) => format!("ERROR: flush failed: {}\n", e),
                                }
                            }
                            Err(e) => format!("ERROR: {}\n", e),
                        }
                    }; // ← Lock dropped here
                    
                    // Now safe to await - no lock held
                    writer.write_all(response.as_bytes()).await?;
                } else {
                    writer.write_all(b"ERROR: invalid PUT format (use: PUT key value)\n").await?;
                }
            }
            "GET" => {
                // For simplicity, GET is not implemented in this WAL-only version
                // In production, you'd query your LSM-tree or memtable
                writer.write_all(b"ERROR: GET not implemented in WAL-only mode\n").await?;
            }
            
            "" => {
                // Ignore empty lines
            }
            
            _ => {
                writer.write_all(b"ERROR: unknown command (supported: PUT)\n").await?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    #[tokio::test]
    async fn test_leader_wal_initialization() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();
        
        // Test that we can initialize the WAL for leader
        let wal_path = data_dir.join("wal");
        let wal = Wal::new(&wal_path, 1024).unwrap();
        let wal = Arc::new(RwLock::new(wal));
        
        assert!(wal_path.exists());
        
        // Verify WAL is accessible and starts at segment 0
        let wal_lock = wal.read();
        assert_eq!(wal_lock.current_segment_number(), 0);
    }
    
    #[tokio::test]
    async fn test_wal_write_operation() {
        let temp_dir = TempDir::new().unwrap();
        let wal_path = temp_dir.path().join("test.wal");
        
        let wal = Wal::new(&wal_path, 1024).unwrap();
        let wal = Arc::new(RwLock::new(wal));
        
        // Test writing an entry
        let entry = Entry {
            key: b"test_key".to_vec(),
            value: b"test_value".to_vec(),
            timestamp: 1000,
        };
        
        let mut wal_lock = wal.write();
        assert!(wal_lock.append(&entry).is_ok());
        assert!(wal_lock.flush().is_ok());
    }
}