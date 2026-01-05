use cityhall::{Entry, Wal};
use tempfile::tempdir;

#[test]
fn test_wal_list_closed_segments() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    let mut wal = Wal::new(&path, 1024).unwrap();
    wal.set_segment_size_limit(10_000);
    // Initially, no closed segments
    let closed = wal.list_closed_segments().unwrap();
    assert_eq!(closed.len(), 0);

    // Write enough to create multiple segments
    for i in 0..100 {
        let entry = Entry {
            key: format!("key_{}", i).into_bytes(),
            value: vec![0u8; 200],
            timestamp: 1000 + i,
        };
        wal.append(&entry).unwrap();
    }

    wal.flush().unwrap();

    // Now should have closed segments
    let closed = wal.list_closed_segments().unwrap();
    assert!(
        closed.len() > 0,
        "Should have closed segments after rotation"
    );

    // Current segment should NOT be in closed list
    let current = wal.current_segment_number();
    assert!(
        !closed.contains(&current),
        "Active segment should not be in closed list"
    );
}

#[test]
fn test_wal_read_specific_segment() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    let mut wal = Wal::new(&path, 1024).unwrap();
    wal.set_segment_size_limit(10_000);
    // Write entries
    for i in 0..50 {
        let entry = Entry {
            key: format!("test_key_{}", i).into_bytes(),
            value: format!("test_value_{}", i).into_bytes(),
            timestamp: 2000 + i,
        };
        wal.append(&entry).unwrap();
    }

    wal.flush().unwrap();

    // Read first closed segment
    let closed_segments = wal.list_closed_segments().unwrap();
    if let Some(&first_segment) = closed_segments.first() {
        let entries = wal.read_segment(first_segment).unwrap();
        assert!(entries.len() > 0, "Segment should contain entries");

        // Verify entry content
        assert!(entries[0].key.starts_with(b"test_key_"));
    }
}

#[test]
fn test_wal_cleanup_respects_replica_position() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    let mut wal = Wal::new(&path, 1024).unwrap();
    wal.set_segment_size_limit(5_000); // Correct - calling the method
                                       // Create several segments
    for i in 0..100 {
        let entry = Entry {
            key: format!("key_{}", i).into_bytes(),
            value: vec![0u8; 100],
            timestamp: 3000 + i,
        };
        wal.append(&entry).unwrap();
    }

    wal.flush().unwrap();

    let segments_before = wal.list_closed_segments().unwrap();
    println!("Segments before cleanup: {:?}", segments_before);

    // Mark as flushed (all segments can be deleted from SSTable perspective)
    wal.mark_flushed().unwrap();

    // But replica is at segment 3, so cleanup should keep 3 and later
    let min_replica_segment = 3;
    wal.cleanup_old_segments(min_replica_segment).unwrap();

    let segments_after = wal.list_closed_segments().unwrap();
    println!("Segments after cleanup: {:?}", segments_after);

    // Should still have segment 3 and later
    assert!(
        segments_after.contains(&min_replica_segment),
        "Should keep segment {} for replica",
        min_replica_segment
    );

    // Should have deleted segments before 3
    for &seg in &segments_before {
        if seg < min_replica_segment {
            assert!(
                !segments_after.contains(&seg),
                "Should have deleted segment {}",
                seg
            );
        }
    }
}

#[test]
fn test_read_nonexistent_segment() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.wal");

    let wal = Wal::new(&path, 1024).unwrap();

    // Try to read segment that doesn't exist
    let result = wal.read_segment(999);
    assert!(result.is_err(), "Should error on nonexistent segment");
}
