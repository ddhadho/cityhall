// tests/test_wal_diagnostic.rs
// Run this to diagnose the WAL issue

use cityhall::StorageEngine;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_wal_diagnostic() {
    let dir = tempdir().unwrap();
    let memtable_size = 256 * 1024 * 1024; // Large memtable
    let mut engine = StorageEngine::new(dir.path().to_path_buf(), memtable_size).unwrap();

    println!("\n=== WAL Diagnostic Test ===\n");

    println!("Test directory: {:?}", dir.path());
    println!("\nDirectory contents before writes:");
    for entry in fs::read_dir(dir.path()).unwrap() {
        let entry = entry.unwrap();
        println!("  {:?}", entry.path());
    }

    // Write some data
    println!("\nWriting 1000 entries...");
    for i in 0..1000 {
        engine
            .put(
                format!("key_{}", i).into_bytes(),
                vec![0u8; 2000], // 2KB each
            )
            .unwrap();
    }

    println!("\nDirectory contents after writes:");
    for entry in fs::read_dir(dir.path()).unwrap() {
        let entry = entry.unwrap();
        let metadata = entry.metadata().unwrap();
        println!("  {:?} - {} bytes", entry.path(), metadata.len());
    }

    // Check wal_segments directory
    let wal_segments_dir = dir.path().join("wal_segments");
    if wal_segments_dir.exists() {
        println!("\nWAL segments directory contents:");
        for entry in fs::read_dir(&wal_segments_dir).unwrap() {
            let entry = entry.unwrap();
            let metadata = entry.metadata().unwrap();
            println!(
                "  {:?} - {} bytes ({} MB)",
                entry.file_name(),
                metadata.len(),
                metadata.len() / 1_048_576
            );
        }
    } else {
        println!("\n⚠️  wal_segments directory does NOT exist!");
    }

    // Check if there's a data.wal file (old single-file WAL)
    let old_wal = dir.path().join("data.wal");
    if old_wal.exists() {
        let metadata = fs::metadata(&old_wal).unwrap();
        println!(
            "\n⚠️  Found OLD single-file WAL: data.wal - {} bytes ({} MB)",
            metadata.len(),
            metadata.len() / 1_048_576
        );
    }

    // Try to get WAL size through engine
    println!("\nEngine stats:");
    let stats = engine.stats();
    println!("  Memtable entries: {}", stats.memtable_entries);
    println!("  Memtable bytes: {} MB", stats.memtable_bytes / 1_048_576);
    println!("  SSTables: {}", stats.num_sstables);

    println!("\n=== End Diagnostic ===\n");
}
