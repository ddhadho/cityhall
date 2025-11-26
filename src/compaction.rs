//! Compaction module for merging SSTables
//! 
//! Implements size-tiered compaction to:
//! - Reduce number of SSTables (faster reads)
//! - Reclaim space (remove old versions, duplicates)
//! - Improve data locality
//! 
//! # Algorithm: K-Way Merge
//! 
//! 1. Select N SSTables to compact (similar size)
//! 2. Open all SSTables, scan in sorted order
//! 3. Merge entries, keeping newest version of each key
//! 4. Write merged SSTable
//! 5. Delete old SSTables

use crate::{Result, StorageError};
use crate::sstable::{SsTableReader, SsTableWriter, DEFAULT_BLOCK_SIZE};
use std::path::{Path, PathBuf};
use std::collections::BinaryHeap;
use std::cmp::Ordering;

/// Entry from an SSTable with source tracking
#[derive(Debug, Clone)]
struct CompactionEntry {
    key: Vec<u8>,
    value: Vec<u8>,
    timestamp: u64,
    sstable_id: usize,  // Which SSTable this came from
}

/// Implement reverse ordering for min-heap
/// (BinaryHeap is max-heap, we want min-heap)
impl PartialEq for CompactionEntry {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for CompactionEntry {}

impl PartialOrd for CompactionEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CompactionEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering for min-heap
        other.key.cmp(&self.key)
            // If keys equal, prefer NEWER timestamp (higher = more recent)
            .then(self.timestamp.cmp(&other.timestamp))
    }
}

/// Compaction statistics
#[derive(Debug, Clone)]
pub struct CompactionStats {
    pub input_sstables: usize,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub entries_merged: usize,
    pub duplicates_removed: usize,
    pub duration_ms: u64,
}

/// Compact multiple SSTables into one
/// 
/// # Arguments
/// * `input_paths` - SSTables to compact
/// * `output_path` - Where to write merged SSTable
/// 
/// # Returns
/// Statistics about the compaction
pub fn compact_sstables(
    input_paths: &[PathBuf],
    output_path: PathBuf,
) -> Result<CompactionStats> {
    use std::time::Instant;
    let start = Instant::now();
    
    println!("🗜️  Compacting {} SSTables into {:?}", input_paths.len(), output_path);
    
    // Open all input SSTables
    let mut readers: Vec<SsTableReader> = Vec::new();
    let mut input_bytes = 0u64;
    
    for path in input_paths {
        let reader = SsTableReader::open(path.clone())?;
        input_bytes += std::fs::metadata(path)?.len();
        readers.push(reader);
    }
    
    // Perform k-way merge
    let (entries_merged, duplicates_removed) = merge_sstables(
        &mut readers,
        &output_path,
    )?;
    
    let output_bytes = std::fs::metadata(&output_path)?.len();
    let duration_ms = start.elapsed().as_millis() as u64;
    
    let stats = CompactionStats {
        input_sstables: input_paths.len(),
        input_bytes,
        output_bytes,
        entries_merged,
        duplicates_removed,
        duration_ms,
    };
    
    println!("✅ Compaction complete:");
    println!("   Input:  {} SSTables, {} bytes", stats.input_sstables, stats.input_bytes);
    println!("   Output: 1 SSTable, {} bytes", stats.output_bytes);
    println!("   Savings: {:.1}%", 
             (1.0 - stats.output_bytes as f64 / stats.input_bytes as f64) * 100.0);
    println!("   Entries: {} merged, {} duplicates removed", 
             stats.entries_merged, stats.duplicates_removed);
    println!("   Duration: {}ms", stats.duration_ms);
    
    Ok(stats)
}

/// Perform k-way merge of SSTables
fn merge_sstables(
    readers: &mut [SsTableReader],
    output_path: &Path,
) -> Result<(usize, usize)> {
    let mut writer = SsTableWriter::new(output_path.to_path_buf(), DEFAULT_BLOCK_SIZE)?;
    
    // Initialize heap with first entry from each SSTable
    let mut heap = BinaryHeap::new();
    let mut iterators: Vec<Vec<(Vec<u8>, Vec<u8>, u64)>> = Vec::new();
    
    for (i, reader) in readers.iter_mut().enumerate() {
        // Scan entire SSTable (from first to last key)
        match reader.scan(&[], &[0xFF; 1024]) {
            Ok(entries) => {
                if !entries.is_empty() {
                    // Push first entry to heap
                    let (key, value, timestamp) = &entries[0];
                    heap.push(CompactionEntry {
                        key: key.clone(),
                        value: value.clone(),
                        timestamp: *timestamp,
                        sstable_id: i,
                    });
                    iterators.push(entries);
                }
            }
            Err(e) => {
                eprintln!("Warning: Failed to scan SSTable {}: {}", i, e);
                iterators.push(Vec::new());
            }
        }
    }
    
    // Track position in each SSTable's entries
    let mut positions = vec![0usize; iterators.len()];
    
    let mut entries_merged = 0;
    let mut duplicates_removed = 0;
    let mut last_key: Option<Vec<u8>> = None;
    
    // K-way merge using min-heap
    while let Some(entry) = heap.pop() {
        // Check if this is a duplicate key
        let is_duplicate = last_key.as_ref().map_or(false, |k| k == &entry.key);
        
        if !is_duplicate {
            // Write unique entry
            writer.add(&entry.key, &entry.value, entry.timestamp)?;
            entries_merged += 1;
            last_key = Some(entry.key.clone());
        } else {
            // Skip duplicate (we already wrote the newest version)
            duplicates_removed += 1;

            if duplicates_removed <= 5 {  // Only show first few
                println!("   Skipping duplicate: {:?} @ t{} (older version)", 
                    String::from_utf8_lossy(&entry.key), entry.timestamp);
            }
        }
        
        // Get next entry from the same SSTable
        let sstable_id = entry.sstable_id;
        positions[sstable_id] += 1;
        
        if positions[sstable_id] < iterators[sstable_id].len() {
            let (key, value, timestamp) = &iterators[sstable_id][positions[sstable_id]];
            heap.push(CompactionEntry {
                key: key.clone(),
                value: value.clone(),
                timestamp: *timestamp,
                sstable_id,
            });
        }
    }
    
    writer.finish()?;
    
    Ok((entries_merged, duplicates_removed))
}

/// Select SSTables for compaction (size-tiered strategy)
/// 
/// Selects N SSTables of similar size (within 50% of each other)
pub fn select_sstables_for_compaction(
    sstable_paths: &[PathBuf],
    min_sstables: usize,
) -> Result<Vec<PathBuf>> {
    if sstable_paths.len() < min_sstables {
        return Ok(Vec::new());  // Not enough SSTables to compact
    }
    
    // Get sizes
    let mut sstables_with_size: Vec<(PathBuf, u64)> = sstable_paths
        .iter()
        .filter_map(|path| {
            std::fs::metadata(path).ok().map(|m| (path.clone(), m.len()))
        })
        .collect();
    
    // Sort by size
    sstables_with_size.sort_by_key(|(_, size)| *size);
    
    // Find largest group of similar-sized SSTables
    let mut best_group = Vec::new();
    
    for i in 0..sstables_with_size.len() {
        let base_size = sstables_with_size[i].1;
        let mut group = vec![sstables_with_size[i].0.clone()];
        
        for j in (i + 1)..sstables_with_size.len() {
            let size = sstables_with_size[j].1;
            
            // Within 50% of base size?
            if size as f64 <= base_size as f64 * 1.5 {
                group.push(sstables_with_size[j].0.clone());
            } else {
                break;
            }
        }
        
        if group.len() >= min_sstables && group.len() > best_group.len() {
            best_group = group;
        }
    }
    
    Ok(best_group)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sstable::SsTableWriter;
    use tempfile::TempDir;
    
    #[test]
    fn test_compact_two_sstables() -> Result<()> {
        let temp_dir = TempDir::new()?;
        
        // Create SSTable 1
        let path1 = temp_dir.path().join("001.sst");
        let mut writer1 = SsTableWriter::new(path1.clone(), DEFAULT_BLOCK_SIZE)?;
        writer1.add(b"key1", b"value1_old", 100)?;
        writer1.add(b"key3", b"value3", 100)?;
        writer1.finish()?;
        
        // Create SSTable 2 (overlapping, newer)
        let path2 = temp_dir.path().join("002.sst");
        let mut writer2 = SsTableWriter::new(path2.clone(), DEFAULT_BLOCK_SIZE)?;
        writer2.add(b"key1", b"value1_new", 200)?;  // Newer version!
        writer2.add(b"key2", b"value2", 200)?;
        writer2.finish()?;
        
        // Compact
        let output = temp_dir.path().join("merged.sst");
        let stats = compact_sstables(&[path1, path2], output.clone())?;
        
        println!("Compaction stats: {:?}", stats);
        
        // Verify merged SSTable
        let mut reader = SsTableReader::open(output)?;
        
        assert_eq!(reader.get(b"key1")?, Some((b"value1_new".to_vec(), 200)));
        assert_eq!(reader.get(b"key2")?, Some((b"value2".to_vec(), 200)));
        assert_eq!(reader.get(b"key3")?, Some((b"value3".to_vec(), 100)));
        
        // Should have removed 1 duplicate (old key1)
        assert_eq!(stats.duplicates_removed, 1);
        assert_eq!(stats.entries_merged, 3);
        
        Ok(())
    }
    
    #[test]
    fn test_select_sstables() -> Result<()> {
        let temp_dir = TempDir::new()?;
        
        // Create SSTables of different sizes
        let paths: Vec<PathBuf> = (0..5)
            .map(|i| {
                let path = temp_dir.path().join(format!("{:03}.sst", i));
                let mut writer = SsTableWriter::new(path.clone(), DEFAULT_BLOCK_SIZE).unwrap();
                
                // Varying number of entries (different sizes)
                for j in 0..(i + 1) * 10 {
                    let key = format!("key{:04}", j);
                    writer.add(key.as_bytes(), b"value", j as u64).unwrap();
                }
                writer.finish().unwrap();
                path
            })
            .collect();
        
        // Select similar-sized SSTables
        let selected = select_sstables_for_compaction(&paths, 2)?;
        
        println!("Selected {} SSTables for compaction", selected.len());
        assert!(selected.len() >= 2);
        
        Ok(())
    }
}