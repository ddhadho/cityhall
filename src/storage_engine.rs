use crate::{Entry, Result, Wal, MemTable, StorageError};
use crate::sstable::{SsTableWriter, SsTableReader};
use crate::metrics::metrics;
use crate::compaction::{compact_sstables, select_sstables_for_compaction, CompactionStats};
use std::path::PathBuf;
use std::time::{SystemTime, Instant, Duration};
use std::sync::atomic::{AtomicU64, Ordering};
use crossbeam::channel::{self, Sender, Receiver};
use std::thread;

const DEFAULT_BLOCK_SIZE: usize = 16 * 1024;

/// Message for background flush thread
enum FlushMessage {
    Flush {
        memtable: MemTable,
        path: PathBuf,
        sstable_id: u64,
    },
    Shutdown,
}

/// Result from background flush
struct FlushResult {
    sstable_id: u64,
    path: PathBuf,
}

pub struct StorageEngine {
    wal: Wal,
    memtable: MemTable,
    immutable_memtable: Option<MemTable>,
    sstables: Vec<SsTableReader>,
    wal_path: PathBuf,
    data_dir: PathBuf,
    memtable_max_size: usize,
    sstable_counter: AtomicU64,

    flush_tx: Option<Sender<FlushMessage>>,
    flush_rx: Option<Receiver<FlushResult>>,
    _flush_thread: Option<thread::JoinHandle<()>>,
    background_flush_enabled: bool,

    compaction_enabled: bool,
    last_compaction_check: Instant,
}

#[derive(Debug)]
pub struct EngineStats {
    pub memtable_entries: usize,
    pub memtable_bytes: usize,
    pub num_sstables: usize,
    pub immutable_memtable_entries: usize,
}

impl StorageEngine {
    /// Create new StorageEngine with background flush enabled
    pub fn new(dir: PathBuf, memtable_max_size: usize) -> Result<Self> {
        Self::new_with_config(dir, memtable_max_size, true)
    }

    /// Full constructor with configurable background flush
    pub fn new_with_config(
        dir: PathBuf,
        memtable_max_size: usize,
        background_flush: bool,
    ) -> Result<Self> {
        std::fs::create_dir_all(&dir)?;

        let wal_path = dir.join("data.wal");
        let wal = Wal::new(&wal_path, 8192)?;

        let entries = Wal::recover(&wal_path)?;
        let mut memtable = MemTable::new(memtable_max_size);
        for entry in entries {
            memtable.put(entry.key, entry.value, entry.timestamp)?;
        }

        // Load SSTables
        let mut sstables = Vec::new();
        let mut max_sstable_id = 0u64;
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("sst") {
                if let Ok(reader) = SsTableReader::open(path.clone()) {
                    if let Some(stem) = path.file_stem() {
                        if let Some(id_str) = stem.to_str() {
                            if let Ok(id) = id_str.parse::<u64>() {
                                max_sstable_id = max_sstable_id.max(id);
                            }
                        }
                    }
                    sstables.push(reader);
                }
            }
        }
        sstables.sort_by_key(|r| {
            r.info()
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0)
        });

        // Setup background flush
        let (flush_tx, flush_rx, flush_thread) = if background_flush {
            let (tx, rx_internal) = channel::unbounded();
            let (result_tx, rx) = channel::unbounded();
            let thread = Some(Self::spawn_flush_thread(rx_internal, result_tx));
            (Some(tx), Some(rx), thread)
        } else {
            (None, None, None)
        };

        Ok(StorageEngine {
            wal,
            memtable,
            immutable_memtable: None,
            sstables,
            wal_path,
            data_dir: dir,
            memtable_max_size,
            sstable_counter: AtomicU64::new(max_sstable_id + 1),
            flush_tx,
            flush_rx,
            _flush_thread: flush_thread,
            background_flush_enabled: background_flush,
            compaction_enabled: true,
            last_compaction_check: Instant::now(),
        })
    }

    /// Enable/disable compaction
    pub fn with_compaction(mut self, enabled: bool) -> Self {
        self.compaction_enabled = enabled;
        self
    }

    fn spawn_flush_thread(rx: Receiver<FlushMessage>, result_tx: Sender<FlushResult>) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            while let Ok(msg) = rx.recv() {
                match msg {
                    FlushMessage::Flush { memtable, path, sstable_id } => {
                        if let Err(e) = Self::flush_memtable_to_disk(memtable, &path) {
                            eprintln!("Background flush FAILED: {}", e);
                        } else {
                            let _ = result_tx.send(FlushResult { sstable_id, path });
                        }
                    }
                    FlushMessage::Shutdown => break,
                }
            }
        })
    }

    fn flush_memtable_to_disk(memtable: MemTable, path: &PathBuf) -> Result<()> {
        let start = Instant::now();
        
        if memtable.is_empty() { return Ok(()); }
        let mut writer = SsTableWriter::new(path.clone(), DEFAULT_BLOCK_SIZE)?;
        for (key, value, timestamp) in memtable.entries_with_timestamps() {
            writer.add(&key, &value, timestamp)?;
        }
        writer.finish()?;
        
        // Track flush
        metrics().flushes_total.inc();
        metrics().flush_duration.observe(start.elapsed());
        metrics().sstable_count.inc();
        
        // Note: disk usage updated by caller
        
        Ok(())
    }

    pub fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
        let start = Instant::now();

        // Increment write counters
        metrics().writes_total.inc();
        metrics().writes_bytes.add((key.len() + value.len()) as u64);

        // Existing logic
        let timestamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs();
        let entry = Entry { key: key.clone(), value: value.clone(), timestamp };
        self.wal.append(&entry)?;
        self.check_and_compact()?;

        let is_full = self.memtable.put(key, value, timestamp)?;
        if is_full {
            if self.background_flush_enabled {
                self.trigger_background_flush()?;
            } else {
                self.flush_memtable_sync()?;
            }
        }

        // Update memtable metrics
        metrics().memtable_size_bytes.set(self.memtable.size_bytes() as u64);
        metrics().memtable_entries.set(self.memtable.len() as u64);

        // Record latency
        metrics().write_latency.observe(start.elapsed());

        Ok(())
    }

    fn trigger_background_flush(&mut self) -> Result<()> {
        let max_wait = 100;
        let mut waited = 0;
        while self.immutable_memtable.is_some() && waited < max_wait {
            self.check_flush_completion()?;
            if self.immutable_memtable.is_some() {
                thread::sleep(Duration::from_millis(1));
                waited += 1;
            }
        }
        if self.immutable_memtable.is_some() {
            return self.flush_memtable_sync();
        }

        let old_memtable = std::mem::replace(&mut self.memtable, MemTable::new(self.memtable_max_size));
        let sstable_id = self.sstable_counter.fetch_add(1, Ordering::SeqCst);
        let sstable_path = self.data_dir.join(format!("{:06}.sst", sstable_id));

        let entries_clone = old_memtable.entries_with_timestamps().clone();
        let mut immutable = MemTable::new(self.memtable_max_size);
        for (k, v, ts) in entries_clone { let _ = immutable.put(k, v, ts); }
        self.immutable_memtable = Some(immutable);

        if let Some(ref tx) = self.flush_tx {
            tx.send(FlushMessage::Flush { memtable: old_memtable, path: sstable_path, sstable_id })?;
        }
        Ok(())
    }

    fn check_flush_completion(&mut self) -> Result<()> {
        if let Some(ref rx) = self.flush_rx {
            while let Ok(result) = rx.try_recv() {
                let reader = SsTableReader::open(result.path)?;
                self.sstables.push(reader);
                self.immutable_memtable = None;
                self.update_disk_usage();
            }
        }
        Ok(())
    }

    fn flush_memtable_sync(&mut self) -> Result<()> {
        if self.memtable.is_empty() { return Ok(()); }
        let sstable_id = self.sstable_counter.fetch_add(1, Ordering::SeqCst);
        let sstable_path = self.data_dir.join(format!("{:06}.sst", sstable_id));
        let memtable_to_flush = std::mem::replace(&mut self.memtable, MemTable::new(self.memtable_max_size));
        Self::flush_memtable_to_disk(memtable_to_flush, &sstable_path)?;
        self.sstables.push(SsTableReader::open(sstable_path)?);
        self.update_disk_usage();
        Ok(())
    }

    pub fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>> {  // Note: &mut self
        let start = Instant::now();

        // Track total read operation
        metrics().reads_total.inc();

        // Check active memtable
        if let Some(value) = self.memtable.get(key) {
            metrics().reads_hits.inc();
            metrics().read_latency.observe(start.elapsed());
            return Ok(Some(value));
        }

        // Check immutable memtable
        if let Some(immut) = &self.immutable_memtable {
            if let Some(value) = immut.get(key) {
                metrics().reads_hits.inc();
                metrics().read_latency.observe(start.elapsed());
                return Ok(Some(value));
            }
        }

        // Check SSTables (bloom filter check is inside sstable.get())
        for sstable in &mut self.sstables {  // &mut for get()
            match sstable.get(key) {
                Ok(Some((value, _timestamp))) => {
                    metrics().reads_hits.inc();
                    metrics().read_latency.observe(start.elapsed());
                    return Ok(Some(value));
                }
                Ok(None) => {
                    // Bloom filter said "maybe" but key wasn't found
                    metrics().bloom_filter_false_positives.inc();
                    continue;
                }
                Err(crate::StorageError::CorruptedData(msg)) => {
                    eprintln!("Warning: corrupted SSTable, skipping: {}", msg);
                    continue;
                }
                Err(e) => return Err(e),
            }
        }

        // Key not found
        metrics().reads_misses.inc();
        metrics().read_latency.observe(start.elapsed());
        Ok(None)
    }

    pub fn scan(&mut self, start: &[u8], end: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>, u64)>> {
        let mut results = self.memtable.scan_with_timestamps(start, end);
        if let Some(ref imm) = self.immutable_memtable {
            results.extend(imm.scan_with_timestamps(start, end));
        }
        for sstable in self.sstables.iter_mut() {
            results.extend(sstable.scan(start, end)?);
        }
        results.sort_by(|a,b| a.0.cmp(&b.0).then(b.2.cmp(&a.2)));
        results.dedup_by(|a,b| a.0 == b.0);
        Ok(results)
    }

    pub fn stats(&self) -> EngineStats {
        EngineStats {
            memtable_entries: self.memtable.len(),
            memtable_bytes: self.memtable.size_bytes(),
            num_sstables: self.sstables.len(),
            immutable_memtable_entries: self.immutable_memtable.as_ref().map(|m| m.len()).unwrap_or(0),
        }
    }

    fn update_disk_usage(&self) {
        let mut total = 0u64;
        
        for sstable in &self.sstables {
            // Get file size from filesystem
            let info = sstable.info();
            if let Ok(metadata) = std::fs::metadata(&info.path) {
                total += metadata.len();
            }
        }
        
        // Add WAL size
        if let Ok(metadata) = std::fs::metadata(&self.wal_path) {
            total += metadata.len();
        }
        
        metrics().disk_usage_bytes.set(total);
    }

    /// Compaction logic (updated)
    pub fn maybe_compact(&mut self) -> Result<()> {
        if !self.compaction_enabled {
            return Ok(());
        }

        let now = Instant::now();
        if now.duration_since(self.last_compaction_check) < Duration::from_secs(1) {
            return Ok(());
        }
        self.last_compaction_check = now;

        let sstable_paths: Vec<PathBuf> = std::fs::read_dir(&self.data_dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("sst"))
            .collect();

        println!("🔍 Compaction check: {} SSTables found", sstable_paths.len());

        if sstable_paths.len() < 4 {
            println!("⏭️  Skipping: need 4+ SSTables (have {})", sstable_paths.len());
            return Ok(());
        }

        let to_compact = select_sstables_for_compaction(&sstable_paths, 4)?;

        if to_compact.is_empty() {
            println!("⏭️  No suitable SSTables selected for compaction");
            return Ok(());
        }

        println!("🔧 Compaction needed: {} SSTables selected", to_compact.len());

        self.compact_sstables_sync(to_compact)?;

        Ok(())
    }

    fn compact_sstables_sync(&mut self, input_paths: Vec<PathBuf>) -> Result<()> {
        println!("🗜️  Starting compaction of {} SSTables", input_paths.len());

        let sstable_id = self.sstable_counter.fetch_add(1, Ordering::SeqCst);
        let output_path = self.data_dir.join(format!("{:06}_compacted.sst", sstable_id));

        let stats = compact_sstables(&input_paths, output_path.clone())?;
        let new_reader = SsTableReader::open(output_path)?;

        let before_count = self.sstables.len();
        self.sstables.retain(|reader| !input_paths.contains(&reader.info().path));
        let after_count = self.sstables.len();
        println!("📊 Removed {} old SSTables from list", before_count - after_count);

        self.sstables.push(new_reader);
        println!("➕ Added compacted SSTable to list");

        for path in &input_paths {
            match std::fs::remove_file(path) {
                Ok(_) => println!("🗑️  Deleted: {:?}", path.file_name()),
                Err(e) => eprintln!("⚠️  Failed to delete {:?}: {}", path, e),
            }
        }

        println!("✅ Compaction complete: {} → 1 SSTable, saved {}%", 
                 stats.input_sstables,
                 ((stats.input_bytes - stats.output_bytes) * 100 / stats.input_bytes));

        Ok(())
    }

    /// Force compaction (for testing)
    pub fn force_compact(&mut self) -> Result<()> {
        self.last_compaction_check = Instant::now() - Duration::from_secs(10);
        self.maybe_compact()
    }

    pub fn check_and_compact(&mut self) -> Result<()> {
        self.check_flush_completion()?;
        self.maybe_compact()?;
        Ok(())
    }

    pub fn sstable_count(&self) -> usize {
        self.sstables.len()
    }

    pub fn memtable_size(&self) -> usize {
        self.memtable.size_bytes()
    }

    /// Get formatted metrics summary
    pub fn metrics(&self) -> String {
        self.update_disk_usage();
        crate::metrics::metrics().summary()
    }
    
    /// Print metrics to stdout
    pub fn print_metrics(&self) {
        println!("{}", self.metrics());
    }

}

impl Drop for StorageEngine {
    fn drop(&mut self) {
        if let Some(ref tx) = self.flush_tx {
            let _ = tx.send(FlushMessage::Shutdown);
        }
    }
}
