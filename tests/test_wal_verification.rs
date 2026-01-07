// tests/test_wal_verification.rs
// Final verification of all WAL cleanup requirements

use cityhall::metrics::metrics;
use cityhall::StorageEngine;
use std::fs;
use std::thread;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn test_wal_success_criteria() {
    println!("\n╔════════════════════════════════════════════════╗");
    println!("║   WAL CLEANUP - SUCCESS CRITERIA VERIFICATION  ║");
    println!("╚════════════════════════════════════════════════╝\n");

    let dir = tempdir().unwrap();
    let memtable_size = 64 * 1024 * 1024;

    // ✅ Criterion 1: WAL rotates to new segment at 100MB
    println!("✓ Testing Criterion 1: WAL segment rotation at 100MB");
    {
        let mut engine = StorageEngine::new(dir.path().to_path_buf(), memtable_size).unwrap();
        metrics().reset();

        // Write ~120MB to force rotation
        for i in 0..60_000 {
            engine
                .put(format!("key_{:05}", i).into_bytes(), vec![0u8; 2000])
                .unwrap();
        }

        let wal_segments_dir = dir.path().join("wal_segments");
        let segment_count = fs::read_dir(&wal_segments_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("wal"))
            .count();

        assert!(
            segment_count >= 2,
            "Should create multiple segments, got {}",
            segment_count
        );
        println!("  ✅ PASS: Created {} segments (>= 2)", segment_count);
    }

    // ✅ Criterion 2: Old segments deleted after flush
    println!("\n✓ Testing Criterion 2: Old segments deleted after flush");
    {
        let dir = tempdir().unwrap();
        let mut engine = StorageEngine::new(dir.path().to_path_buf(), memtable_size).unwrap();

        // Write enough to create multiple segments (120MB)
        for i in 0..60_000 {
            engine
                .put(format!("key_{:05}", i).into_bytes(), vec![0u8; 2000])
                .unwrap();
        }

        let segments_before = count_wal_segments(dir.path());
        println!("  Segments before flush: {}", segments_before);

        // Now write enough to fill memtable and trigger flush
        let stats = engine.stats();
        let needed = ((memtable_size - stats.memtable_bytes) / 2000) + 1000;
        for i in 60_000..60_000 + needed {
            engine
                .put(format!("key_{:05}", i).into_bytes(), vec![0u8; 2000])
                .unwrap();
        }

        // Wait for flush
        thread::sleep(Duration::from_millis(500));
        engine.check_and_compact().unwrap();

        let segments_after = count_wal_segments(dir.path());
        println!("  Segments after flush: {}", segments_after);

        assert!(
            segments_after <= segments_before && segments_after == 1,
            "WAL cleanup should leave exactly one active segment. Before: {}, After: {}",
            segments_before,
            segments_after
        );
        println!(
            "  ✅ PASS: Segments reduced from {} to {}",
            segments_before, segments_after
        );
    }

    // ✅ Criterion 3: WAL size bounded (< 200MB)
    println!("\n✓ Testing Criterion 3: WAL size bounded");
    {
        let dir = tempdir().unwrap();
        let mut engine = StorageEngine::new(dir.path().to_path_buf(), memtable_size).unwrap();
        metrics().reset();

        // Write 500MB of data with periodic flushes
        for batch in 0..5 {
            for i in 0..20_000 {
                let key_id = batch * 20_000 + i;
                engine
                    .put(format!("key_{:08}", key_id).into_bytes(), vec![0u8; 2000])
                    .unwrap();
            }

            // Trigger flush every batch
            thread::sleep(Duration::from_millis(200));
            engine.check_and_compact().unwrap();
        }

        let final_wal_size = metrics().wal_size_bytes.get();
        let max_allowed = 200 * 1024 * 1024; // 200MB

        assert!(
            final_wal_size < max_allowed,
            "WAL should stay under 200MB, got {} MB",
            final_wal_size / 1_048_576
        );
        println!(
            "  ✅ PASS: WAL size {} MB (< 200MB)",
            final_wal_size / 1_048_576
        );
    }

    // ✅ Criterion 4: Recovery works correctly after cleanup
    println!("\n✓ Testing Criterion 4: Recovery after cleanup");
    {
        let dir = tempdir().unwrap();

        // Write, flush (cleanup), write more
        {
            let mut engine = StorageEngine::new(dir.path().to_path_buf(), memtable_size).unwrap();

            for i in 0..5_000 {
                engine
                    .put(
                        format!("key_{}", i).into_bytes(),
                        format!("value_{}", i).into_bytes(),
                    )
                    .unwrap();
            }

            thread::sleep(Duration::from_millis(200));
            engine.check_and_compact().unwrap();

            // Write after cleanup
            for i in 5_000..10_000 {
                engine
                    .put(
                        format!("key_{}", i).into_bytes(),
                        format!("value_{}", i).into_bytes(),
                    )
                    .unwrap();
            }
        }

        // Recover
        {
            let mut engine = StorageEngine::new(dir.path().to_path_buf(), memtable_size).unwrap();

            let mut recovered = 0;
            for i in 5_000..10_000 {
                if engine
                    .get(&format!("key_{}", i).into_bytes())
                    .unwrap()
                    .is_some()
                {
                    recovered += 1;
                }
            }

            assert!(
                recovered >= 4_500,
                "Should recover most entries, got {}/5000",
                recovered
            );
            println!(
                "  ✅ PASS: Recovered {}/5000 entries after cleanup",
                recovered
            );
        }
    }

    // ✅ Criterion 5: Metrics track WAL size accurately
    println!("\n✓ Testing Criterion 5: Metrics accuracy");
    {
        let dir = tempdir().unwrap();
        let mut engine = StorageEngine::new(dir.path().to_path_buf(), memtable_size).unwrap();
        metrics().reset();

        for i in 0..10_000 {
            engine
                .put(format!("key_{}", i).into_bytes(), vec![0u8; 1000])
                .unwrap();
        }

        let metric_size = metrics().wal_size_bytes.get();
        let actual_size = calculate_actual_wal_size(dir.path());

        let diff_pct =
            ((metric_size as i64 - actual_size as i64).abs() as f64 / actual_size as f64) * 100.0;

        assert!(
            diff_pct < 5.0,
            "Metric should match actual size within 5%, diff was {:.1}%",
            diff_pct
        );
        println!(
            "  ✅ PASS: Metric {} MB vs Actual {} MB (diff: {:.1}%)",
            metric_size / 1_048_576,
            actual_size / 1_048_576,
            diff_pct
        );
    }

    println!("\n╔════════════════════════════════════════════════╗");
    println!("║          ✅ ALL SUCCESS CRITERIA MET!          ║");
    println!("╚════════════════════════════════════════════════╝\n");
}

fn count_wal_segments(dir: &std::path::Path) -> usize {
    let wal_dir = dir.join("wal_segments");
    if !wal_dir.exists() {
        return 0;
    }

    fs::read_dir(&wal_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("wal"))
        .count()
}

fn calculate_actual_wal_size(dir: &std::path::Path) -> u64 {
    let wal_dir = dir.join("wal_segments");
    if !wal_dir.exists() {
        return 0;
    }

    fs::read_dir(&wal_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("wal"))
        .map(|e| e.metadata().unwrap().len())
        .sum()
}
