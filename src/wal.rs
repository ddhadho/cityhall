//! Segmented Write-Ahead Log
//!
//! Prevents unbounded WAL growth through segment rotation and cleanup.
//!
//! ## Architecture
//!
//! Instead of a single WAL file, uses multiple segments:
//! - wal_segments/000001.wal (100MB, old) ← deleted after flush
//! - wal_segments/000002.wal (100MB, old) ← deleted after flush
//! - wal_segments/000003.wal (50MB, active) ← current writes
//!
//! ## Rotation & Cleanup
//!
//! - **Rotation**: When segment reaches 100MB, start new segment
//! - **Cleanup**: After flush, delete all segments before flush point
//! - **Recovery**: Replay all segments in order

use crate::{Entry, OpType, Result, StorageError};
use bytes::{Buf, BufMut, BytesMut};
use crc32fast::Hasher;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const DEFAULT_SEGMENT_SIZE: usize = 100 * 1024 * 1024; // 100MB

pub struct Wal {
    dir: PathBuf,
    current_segment: WalSegment,
    segment_number: u64,
    segment_size_limit: usize,
    last_flushed_segment: u64,
}

struct WalSegment {
    file: File,
    #[allow(dead_code)]
    path: PathBuf,
    #[allow(dead_code)]
    segment_number: u64,
    buffer: BytesMut,
    buffer_capacity: usize,
    bytes_written: usize,
}

impl Wal {
    /// Create new segmented WAL
    pub fn new(path: impl AsRef<Path>, buffer_size: usize) -> Result<Self> {
        let dir = path
            .as_ref()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("wal_segments");

        std::fs::create_dir_all(&dir)?;

        let segment_number = Self::find_latest_segment_number(&dir)? + 1;
        let current_segment = WalSegment::new(&dir, segment_number, buffer_size)?;

        Ok(Self {
            dir,
            current_segment,
            segment_number,
            segment_size_limit: DEFAULT_SEGMENT_SIZE,
            last_flushed_segment: 0,
        })
    }

    /// Set segment size limit (useful for testing with smaller segments)
    pub fn set_segment_size_limit(&mut self, limit: usize) {
        self.segment_size_limit = limit;
    }

    fn find_latest_segment_number(dir: &Path) -> Result<u64> {
        let mut max_segment = 0;

        if dir.exists() {
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                if let Some(num) = Self::parse_segment_number(&entry.path()) {
                    max_segment = max_segment.max(num);
                }
            }
        }

        Ok(max_segment)
    }

    fn parse_segment_number(path: &Path) -> Option<u64> {
        path.file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse::<u64>().ok())
    }

    pub fn append(&mut self, entry: &Entry) -> Result<()> {
        self.append_operation(OpType::Put, entry)
    }

    pub fn append_delete(&mut self, key: &[u8], timestamp: u64) -> Result<()> {
        let entry = Entry {
            key: key.to_vec(),
            value: Vec::new(),
            timestamp,
        };
        self.append_operation(OpType::Delete, &entry)
    }

    fn append_operation(&mut self, op_type: OpType, entry: &Entry) -> Result<()> {
        // Check if rotation needed
        if self.current_segment.should_rotate(self.segment_size_limit) {
            self.rotate_segment()?;
        }

        let record = encode_record(op_type, entry)?;
        self.current_segment.append(&record)?;

        Ok(())
    }

    fn rotate_segment(&mut self) -> Result<()> {
        self.current_segment.flush()?;

        self.segment_number += 1;
        self.current_segment = WalSegment::new(
            &self.dir,
            self.segment_number,
            self.current_segment.buffer_capacity,
        )?;

        println!("📝 Rotated WAL to segment {:06}", self.segment_number);

        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        self.current_segment.flush()
    }

    /// Mark current segment as flushed to SSTable
    pub fn mark_flushed(&mut self) -> Result<()> {
        self.last_flushed_segment = self.segment_number;
        println!("✓ Marked segment {:06} as flushed", self.segment_number);
        Ok(())
    }

    /// Delete old segments before flush point
    /// Delete old segments before the minimum of flush point and replica positions
    ///
    /// # Arguments
    /// * `min_replica_segment` - Minimum segment needed by any replica (0 if no replicas)
    ///
    /// This method ensures segments are only deleted when BOTH:
    /// 1. They've been flushed to SSTable (persisted)
    /// 2. All replicas have synced them (no data loss)
    pub fn cleanup_old_segments(&mut self, min_replica_segment: u64) -> Result<()> {
        let mut deleted_count = 0;
        let mut reclaimed_bytes = 0u64;

        // Don't delete segments that:
        // 1. Haven't been flushed to SSTable yet, OR
        // 2. Are still needed by replicas
        let safe_to_delete = if min_replica_segment > 0 {
            // If replicas exist, respect both flush point and replica position
            self.last_flushed_segment.min(min_replica_segment)
        } else {
            // No replicas, only consider flush point
            self.last_flushed_segment
        };

        if safe_to_delete == 0 {
            println!("⏸️  No segments safe to delete (safe_to_delete = 0)");
            return Ok(());
        }

        println!(
            "🗑️  Cleanup policy: delete segments < {} (flush: {}, min_replica: {})",
            safe_to_delete, self.last_flushed_segment, min_replica_segment
        );

        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();

            if let Some(segment_num) = Self::parse_segment_number(&path) {
                if segment_num < safe_to_delete {
                    let size = entry.metadata()?.len();
                    std::fs::remove_file(&path)?;
                    deleted_count += 1;
                    reclaimed_bytes += size;

                    println!(
                        "🗑️  Deleted WAL segment {:06}.wal ({} bytes)",
                        segment_num, size
                    );
                }
            }
        }

        if deleted_count > 0 {
            println!(
                "✅ WAL cleanup: {} segments, {} MB reclaimed",
                deleted_count,
                reclaimed_bytes / 1_048_576
            );
        } else {
            println!("ℹ️  No segments deleted (all needed)");
        }

        Ok(())
    }

    pub fn recover(path: impl AsRef<Path>) -> Result<Vec<Entry>> {
        let dir = path
            .as_ref()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("wal_segments");

        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut segments = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();

            if let Some(segment_num) = Self::parse_segment_number(&path) {
                segments.push((segment_num, path));
            }
        }

        segments.sort_by_key(|(num, _)| *num);

        let mut entries = Vec::new();
        for (segment_num, path) in segments {
            println!("♻️  Recovering from segment {:06}.wal", segment_num);

            let segment_entries = Self::recover_segment(&path)?;
            entries.extend(segment_entries);
        }

        if !entries.is_empty() {
            println!("✅ Recovered {} entries from WAL", entries.len());
        }

        Ok(entries)
    }

    fn recover_segment(path: &Path) -> Result<Vec<Entry>> {
        let mut file = File::open(path)?;
        let mut entries = Vec::new();

        loop {
            match read_record(&mut file) {
                Ok(Some((op_type, entry))) => match op_type {
                    OpType::Put => entries.push(entry),
                    OpType::Delete => entries.push(entry),
                },
                Ok(None) => break,
                Err(StorageError::Corruption(msg)) => {
                    eprintln!("WAL corruption detected: {}", msg);
                    break;
                }
                Err(e) => return Err(e),
            }
        }

        Ok(entries)
    }

    pub fn size(&self) -> Result<u64> {
        Ok(self.total_size())
    }

    pub fn file_size(&self) -> u64 {
        self.total_size()
    }

    fn total_size(&self) -> u64 {
        let mut total = 0;

        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                if entry.path().extension().and_then(|s| s.to_str()) == Some("wal") {
                    total += entry.metadata().map(|m| m.len()).unwrap_or(0);
                }
            }
        }

        total
    }

    /// Get list of closed (non-active) segments available for replication
    pub fn list_closed_segments(&self) -> Result<Vec<u64>> {
        let mut segments = Vec::new();

        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();

            if let Some(num) = Self::parse_segment_number(&path) {
                // Only return closed segments (not the current active one)
                if num < self.segment_number {
                    segments.push(num);
                }
            }
        }

        segments.sort_unstable();
        Ok(segments)
    }

    /// Read all entries from a specific segment
    ///
    /// # Arguments
    /// * `segment_number` - The segment to read
    ///
    /// # Returns
    /// * `Ok(Vec<Entry>)` - All entries from the segment
    /// * `Err` - If segment doesn't exist or is corrupted
    pub fn read_segment(&self, segment_number: u64) -> Result<Vec<Entry>> {
        let path = self.dir.join(format!("{:06}.wal", segment_number));

        if !path.exists() {
            return Err(StorageError::NotFound(format!(
                "Segment {} not found",
                segment_number
            )));
        }

        Self::recover_segment(&path)
    }

    /// Get current active segment number
    pub fn current_segment_number(&self) -> u64 {
        self.segment_number
    }

    /// Get directory path for WAL segments
    pub fn segment_dir(&self) -> &Path {
        &self.dir
    }

    /// Check if a segment exists and is closed (safe to read)
    pub fn is_segment_available(&self, segment_number: u64) -> bool {
        segment_number < self.segment_number
            && self.dir.join(format!("{:06}.wal", segment_number)).exists()
    }
}

impl WalSegment {
    fn new(dir: &Path, segment_number: u64, buffer_capacity: usize) -> Result<Self> {
        let path = dir.join(format!("{:06}.wal", segment_number));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;

        Ok(Self {
            file,
            path,
            segment_number,
            buffer: BytesMut::with_capacity(buffer_capacity),
            buffer_capacity,
            bytes_written: 0,
        })
    }

    fn should_rotate(&self, size_limit: usize) -> bool {
        self.bytes_written >= size_limit
    }

    fn append(&mut self, record: &[u8]) -> Result<()> {
        self.buffer.extend_from_slice(record);
        self.bytes_written += record.len();

        if self.buffer.len() >= self.buffer_capacity {
            self.flush()?;
        }

        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if !self.buffer.is_empty() {
            self.file.write_all(&self.buffer)?;
            self.file.sync_all()?;
            self.buffer.clear();
        }
        Ok(())
    }
}

impl Drop for Wal {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

fn encode_record(op_type: OpType, entry: &Entry) -> Result<Vec<u8>> {
    let mut data = BytesMut::new();

    data.put_u64_le(entry.timestamp);

    if entry.key.len() > u16::MAX as usize {
        return Err(StorageError::InvalidFormat("Key too long".into()));
    }
    data.put_u16_le(entry.key.len() as u16);
    data.put_slice(&entry.key);

    if entry.value.len() > u32::MAX as usize {
        return Err(StorageError::InvalidFormat("Value too long".into()));
    }
    data.put_u32_le(entry.value.len() as u32);
    data.put_slice(&entry.value);

    let data_len = data.len();
    if data_len > u16::MAX as usize {
        return Err(StorageError::InvalidFormat("Record too large".into()));
    }

    let mut hasher = Hasher::new();
    hasher.update(&(data_len as u16).to_le_bytes());
    hasher.update(&[op_type as u8]);
    hasher.update(&data);
    let checksum = hasher.finalize();

    let mut record = BytesMut::with_capacity(4 + 2 + 1 + data_len);
    record.put_u32_le(checksum);
    record.put_u16_le(data_len as u16);
    record.put_u8(op_type as u8);
    record.put_slice(&data);

    Ok(record.to_vec())
}

fn read_record(file: &mut File) -> Result<Option<(OpType, Entry)>> {
    let mut header = [0u8; 7];
    match file.read_exact(&mut header) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Ok(None);
        }
        Err(e) => return Err(e.into()),
    }

    let mut buf = &header[..];
    let checksum = buf.get_u32_le();
    let length = buf.get_u16_le();
    let type_byte = buf.get_u8();

    let op_type = OpType::from_u8(type_byte)
        .ok_or_else(|| StorageError::InvalidFormat(format!("Invalid op type: {}", type_byte)))?;

    let mut data = vec![0u8; length as usize];
    file.read_exact(&mut data)?;

    let mut hasher = Hasher::new();
    hasher.update(&length.to_le_bytes());
    hasher.update(&[type_byte]);
    hasher.update(&data);
    let computed = hasher.finalize();

    if computed != checksum {
        return Err(StorageError::Corruption(format!(
            "Checksum mismatch: expected {}, got {}",
            checksum, computed
        )));
    }

    let mut buf = &data[..];

    let timestamp = buf.get_u64_le();

    let key_len = buf.get_u16_le() as usize;
    if buf.remaining() < key_len {
        return Err(StorageError::Corruption("Truncated key".into()));
    }
    let key = buf[..key_len].to_vec();
    buf.advance(key_len);

    let value_len = buf.get_u32_le() as usize;
    if buf.remaining() < value_len {
        return Err(StorageError::Corruption("Truncated value".into()));
    }
    let value = buf[..value_len].to_vec();

    let entry = Entry {
        key,
        value,
        timestamp,
    };

    Ok(Some((op_type, entry)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_wal_basic() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.wal");

        let mut wal = Wal::new(&path, 1024).unwrap();

        let entry = Entry {
            key: b"test_key".to_vec(),
            value: b"test_value".to_vec(),
            timestamp: 1234567890,
        };

        wal.append(&entry).unwrap();
        wal.flush().unwrap();

        assert!(wal.size().unwrap() > 0);
    }

    #[test]
    fn test_wal_recovery() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.wal");

        {
            let mut wal = Wal::new(&path, 1024).unwrap();

            for i in 0..100 {
                let entry = Entry {
                    key: format!("key_{}", i).into_bytes(),
                    value: format!("value_{}", i).into_bytes(),
                    timestamp: 1000 + i,
                };
                wal.append(&entry).unwrap();
            }

            wal.flush().unwrap();
        }

        let entries = Wal::recover(&path).unwrap();

        assert_eq!(entries.len(), 100);
        assert_eq!(entries[0].key, b"key_0");
        assert_eq!(entries[99].value, b"value_99");
    }

    #[test]
    fn test_wal_cleanup() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.wal");

        let mut wal = Wal::new(&path, 1024).unwrap();

        // Force small segment size for testing
        wal.segment_size_limit = 50_000; // 50KB instead of 100MB

        println!("\n=== Testing WAL Cleanup ===\n");

        // Phase 1: Write enough to create multiple segments
        println!("Phase 1: Writing data to trigger rotation...");
        for i in 0..100 {
            let entry = Entry {
                key: format!("key_{}", i).into_bytes(),
                value: vec![0u8; 1000], // 1KB per entry
                timestamp: 1000 + i,
            };
            wal.append(&entry).unwrap();
        }

        wal.flush().unwrap();

        let size_before = wal.size().unwrap();
        let segment_before = wal.segment_number;

        println!("Before cleanup:");
        println!("  Segments created: {}", segment_before);
        println!("  Total size: {} bytes", size_before);

        // Phase 2: Mark as flushed and cleanup
        println!("\nPhase 2: Marking flushed and cleaning up...");
        wal.mark_flushed().unwrap();
        wal.cleanup_old_segments(0).unwrap();

        let size_after = wal.size().unwrap();

        println!("\nAfter cleanup:");
        println!("  Total size: {} bytes", size_after);
        println!(
            "  Reduction: {} bytes ({:.1}%)",
            size_before.saturating_sub(size_after),
            (size_before.saturating_sub(size_after) as f64 / size_before as f64) * 100.0
        );

        // Should have cleaned up old segments
        assert!(
            size_after < size_before,
            "WAL should shrink after cleanup: {} >= {}",
            size_after,
            size_before
        );

        println!("\n✅ WAL cleanup working!");
    }

    #[test]
    fn test_wal_cleanup_respects_replica_position() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.wal");

        let mut wal = Wal::new(&path, 1024).unwrap();
        wal.segment_size_limit = 5_000; // Very small for multiple segments

        println!("\n=== Testing Replica-Aware Cleanup ===\n");

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
        println!("Total segments: {}", segments_before.len());

        // Mark all as flushed (from SSTable perspective, all can be deleted)
        wal.mark_flushed().unwrap();
        println!("Marked segment {} as flushed", wal.segment_number);

        // But replica is at segment 3, so we should keep 3 and later
        let min_replica_segment = 2;
        println!("Min replica segment: {}", min_replica_segment);

        wal.cleanup_old_segments(min_replica_segment).unwrap();

        let segments_after = wal.list_closed_segments().unwrap();
        println!("\nSegments after cleanup: {:?}", segments_after);

        // Verify: Should still have segment 3 and later
        assert!(
            segments_after.contains(&min_replica_segment),
            "Should keep segment {} for replica",
            min_replica_segment
        );

        // Verify: Should have deleted segments before 3
        for &seg in &segments_before {
            if seg < min_replica_segment {
                assert!(
                    !segments_after.contains(&seg),
                    "Should have deleted segment {}",
                    seg
                );
                println!("✓ Correctly deleted segment {}", seg);
            } else {
                assert!(
                    segments_after.contains(&seg),
                    "Should have kept segment {}",
                    seg
                );
                println!("✓ Correctly kept segment {}", seg);
            }
        }

        println!("\n✅ Replica-aware cleanup working!");
    }
}
