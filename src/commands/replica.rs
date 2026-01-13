//! Replica command implementation
//!
//! Starts CityHall in replica mode with:
//! - Replication agent syncing from leader
//! - Status monitoring and reporting

use cityhall::{Result, Wal};
use cityhall::replication::{ReplicationAgent, ReplicationConfig, ReplicaState};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use parking_lot::RwLock;
use tokio::signal;

/// Output format for status command
#[derive(Debug, Clone)]
pub enum OutputFormat {
    Text,
    Json,
    Compact,
}

/// Run CityHall in replica mode
pub async fn run_replica(
    leader: String,
    data_dir: PathBuf,
    sync_interval: u64,
    connect_timeout: u64,
    read_timeout: u64,
) -> Result<()> {
    println!("📥 Starting CityHall in REPLICA mode");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🎯 Leader: {}", leader);
    println!("📁 Data directory: {:?}", data_dir);
    println!("⏱️  Sync interval: {}s", sync_interval);
    println!("⏳ Connect timeout: {}s", connect_timeout);
    println!("⏳ Read timeout: {}s", read_timeout);
    println!();
    
    // Create data directory
    std::fs::create_dir_all(&data_dir)?;
    
    // Initialize WAL for replication
    let wal_path = data_dir.join("wal");
    let wal = Wal::new(&wal_path, 1024 * 1024)?; // 1MB buffer
    let wal = Arc::new(RwLock::new(wal));
    println!("✓ WAL initialized at {:?}", wal_path);
    
    // Create replica state path
    let state_path = data_dir.join("replica_state.json");
    
    // Generate replica ID (hostname or random)
    let replica_id = generate_replica_id();
    println!("✓ Replica ID: {}", replica_id);
    
    // Configure replication
    let config = ReplicationConfig {
        connect_timeout: Duration::from_secs(connect_timeout),
        read_timeout: Duration::from_secs(read_timeout),
        write_timeout: Duration::from_secs(10),
        sync_interval: Duration::from_secs(sync_interval),
    };
    
    // Create replication agent
    let mut agent = ReplicationAgent::with_config(
        leader.clone(),
        replica_id,
        state_path,
        Arc::clone(&wal),
        config,
    )?;
    
    println!("✓ Replication agent created");
    
    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("✅ Replica is running!");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🔄 Syncing from: {}", leader);
    println!("📴 Press Ctrl+C to stop");
    println!();
    
    // Start sync loop
    let sync_handle = tokio::spawn(async move {
        if let Err(e) = agent.run().await {
            eprintln!("❌ Sync loop error: {}", e);
        }
    });
    
    // Wait for shutdown signal
    tokio::select! {
        _ = signal::ctrl_c() => {
            println!();
            println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            println!("📴 Shutting down replica...");
        }
        _ = sync_handle => {
            println!("⚠️  Sync loop stopped unexpectedly");
        }
    }
    
    println!("✓ Replica stopped");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    
    Ok(())
}

/// Show replica status (health, metrics, sync state)
pub async fn show_replica_status(
    data_dir: PathBuf,
    verbose: bool,
    format: OutputFormat,
) -> Result<()> {
    match format {
        OutputFormat::Json => show_status_json(&data_dir, verbose).await,
        OutputFormat::Compact => show_status_compact(&data_dir).await,
        OutputFormat::Text => show_status_text(&data_dir, verbose).await,
    }
}

/// Show status in human-readable text format
async fn show_status_text(data_dir: &PathBuf, verbose: bool) -> Result<()> {
    println!("📊 Replica Status");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📁 Data directory: {:?}", data_dir);
    println!();
    
    // Load replica state
    let state_path = data_dir.join("replica_state.json");
    
    if !state_path.exists() {
        println!("⚠️  No replica state found");
        println!("   Has the replica been started?");
        println!("   Expected state file: {:?}", state_path);
        return Ok(());
    }
    
    let state = ReplicaState::load(&state_path)?;
    
    // Display basic state
    println!("═══ Replication State ═══");
    println!("Replica ID         : {}", state.replica_id);
    println!("Leader Address     : {}", state.leader_addr);
    println!("Last Synced Segment: {}", state.last_synced_segment);
    println!("Next Segment       : {}", state.next_segment_to_sync());
    println!("Total Segments     : {}", state.total_segments_synced);
    println!("Total Entries      : {}", state.total_entries_applied);
    
    // Time since last sync
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    
    let age_secs = now.saturating_sub(state.last_sync_time);
    println!();
    println!("═══ Sync Status ═══");
    println!("Last Sync          : {} seconds ago", age_secs);
    
    // Health indicator
    if age_secs > 300 {
        println!("Health             : 🔴 STALE (no sync in {}s)", age_secs);
    } else if age_secs > 60 {
        println!("Health             : 🟡 WARNING (no sync in {}s)", age_secs);
    } else {
        println!("Health             : 🟢 HEALTHY");
    }
    
    // Check WAL
    if verbose {
        println!();
        println!("═══ Local WAL ═══");
        let wal_path = data_dir.join("wal");
        if wal_path.exists() {
            match Wal::new(&wal_path, 1024) {
                Ok(wal) => {
                    let current_seg = wal.current_segment_number();
                    let segments = wal.list_closed_segments();
                    
                    println!("Current Segment    : {}", current_seg);
                    println!("Closed Segments    : {}", segments?.len());

                }
                Err(e) => {
                    println!("Error reading WAL  : {}", e);
                }
            }
        } else {
            println!("WAL directory      : Not found");
        }
    }
    
    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    
    Ok(())
}

/// Show status in compact format
async fn show_status_compact(data_dir: &PathBuf) -> Result<()> {
    let state_path = data_dir.join("replica_state.json");
    
    if !state_path.exists() {
        println!("STATUS: NO_STATE");
        return Ok(());
    }
    
    let state = ReplicaState::load(&state_path)?;
    
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let age = now.saturating_sub(state.last_sync_time);
    
    let health = if age > 300 {
        "STALE"
    } else if age > 60 {
        "WARNING"
    } else {
        "HEALTHY"
    };
    
    println!(
        "STATUS: {} | SEGMENT: {} | ENTRIES: {} | AGE: {}s",
        health, state.last_synced_segment, state.total_entries_applied, age
    );
    
    Ok(())
}

/// Show status in JSON format
async fn show_status_json(data_dir: &PathBuf, verbose: bool) -> Result<()> {
    let state_path = data_dir.join("replica_state.json");
    
    if !state_path.exists() {
        println!("{{\"status\":\"no_state\",\"message\":\"No replica state found\"}}");
        return Ok(());
    }
    
    let state = ReplicaState::load(&state_path)?;
    
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let age = now.saturating_sub(state.last_sync_time);
    
    let health = if age > 300 {
        "stale"
    } else if age > 60 {
        "warning"
    } else {
        "healthy"
    };
    
    // Simple JSON output (could use serde_json if available)
    print!("{{");
    print!("\"replica_id\":\"{}\",", state.replica_id);
    print!("\"leader_addr\":\"{}\",", state.leader_addr);
    print!("\"last_synced_segment\":{},", state.last_synced_segment);
    print!("\"next_segment\":{},", state.next_segment_to_sync());
    print!("\"total_segments_synced\":{},", state.total_segments_synced);
    print!("\"total_entries_applied\":{},", state.total_entries_applied);
    print!("\"last_sync_time\":{},", state.last_sync_time);
    print!("\"age_seconds\":{},", age);
    print!("\"health\":\"{}\"", health);
    
    if verbose {
        let wal_path = data_dir.join("wal");
        if wal_path.exists() {
            if let Ok(wal) = Wal::new(&wal_path, 1024) {
                print!(",\"wal\":{{");
                print!("\"current_segment\":{},", wal.current_segment_number());
                print!("\"closed_segments\":[");
                let segments = wal.list_closed_segments();
                for (i, seg) in segments.iter().enumerate() {
                    if i > 0 { print!(","); }
                   print!("{:#?}", seg);
                }
                print!("]");
                print!("}}");
            }
        }
    }
    
    println!("}}");
    
    Ok(())
}

/// Generate a unique replica ID
fn generate_replica_id() -> String {
    // Try to get hostname first
    if let Ok(hostname) = hostname::get() {
        if let Ok(hostname_str) = hostname.into_string() {
            return format!("replica-{}", hostname_str);
        }
    }
    
    // Fall back to random ID with timestamp
    use std::time::SystemTime;
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    
    format!("replica-{}-{}", ts, fastrand::u32(..))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    #[tokio::test]
    async fn test_replica_id_generation() {
        let id1 = generate_replica_id();
        let id2 = generate_replica_id();
        
        assert!(id1.starts_with("replica-"));
        assert!(id2.starts_with("replica-"));
    }
    
    #[tokio::test]
    async fn test_status_no_state() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();
        
        // Should handle missing state gracefully
        let result = show_replica_status(data_dir, false, OutputFormat::Text).await;
        assert!(result.is_ok());
    }
}
