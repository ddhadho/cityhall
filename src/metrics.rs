// src/metrics.rs
//
// Production-grade metrics system for LSM storage engine
// Tracks operation counts, latencies, and system state

use parking_lot::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Global metrics singleton
static METRICS: once_cell::sync::Lazy<Arc<Metrics>> =
    once_cell::sync::Lazy::new(|| Arc::new(Metrics::new()));

pub fn metrics() -> Arc<Metrics> {
    Arc::clone(&METRICS)
}

/// Main metrics container
#[derive(Debug)]
pub struct Metrics {
    // === Operation Counters ===
    // Writes
    pub writes_total: Counter,
    pub writes_bytes: Counter, // NEW: Total bytes written

    // Reads
    pub reads_total: Counter,
    pub reads_hits: Counter,   // NEW: Found in memtable/sstable
    pub reads_misses: Counter, // NEW: Key not found

    // Flushes
    pub flushes_total: Counter,
    pub flush_duration: Histogram,

    // Compactions
    pub compactions_total: Counter,
    pub compaction_bytes_in: Counter,  // NEW: Input size
    pub compaction_bytes_out: Counter, // NEW: Output size
    pub compaction_duration: Histogram,

    // Bloom filter effectiveness
    pub bloom_filter_hits: Counter,
    pub bloom_filter_misses: Counter,
    pub bloom_filter_false_positives: Counter,

    // === Performance Metrics ===
    pub write_latency: Histogram,
    pub read_latency: Histogram,

    // === System State ===
    pub memtable_size_bytes: Gauge,
    pub memtable_entries: Gauge, // NEW: Number of entries
    pub immutable_count: Gauge,
    pub sstable_count: Gauge,
    pub disk_usage_bytes: Gauge, // NEW: Total disk usage
    pub wal_size_bytes: Gauge,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            writes_total: Counter::new(),
            writes_bytes: Counter::new(),
            reads_total: Counter::new(),
            reads_hits: Counter::new(),
            reads_misses: Counter::new(),
            flushes_total: Counter::new(),
            compactions_total: Counter::new(),
            compaction_bytes_in: Counter::new(),
            compaction_bytes_out: Counter::new(),

            bloom_filter_hits: Counter::new(),
            bloom_filter_misses: Counter::new(),
            bloom_filter_false_positives: Counter::new(),

            write_latency: Histogram::new(),
            read_latency: Histogram::new(),
            flush_duration: Histogram::new(),
            compaction_duration: Histogram::new(),

            memtable_size_bytes: Gauge::new(),
            memtable_entries: Gauge::new(),
            immutable_count: Gauge::new(),
            sstable_count: Gauge::new(),
            disk_usage_bytes: Gauge::new(),
            wal_size_bytes: Gauge::new(),
        }
    }

    // === Computed Metrics ===

    /// Read hit rate (0.0 to 1.0)
    pub fn read_hit_rate(&self) -> f64 {
        let hits = self.reads_hits.get();
        let total = self.reads_total.get();
        if total == 0 {
            return 0.0;
        }
        hits as f64 / total as f64
    }

    /// Read miss rate (0.0 to 1.0)
    pub fn read_miss_rate(&self) -> f64 {
        1.0 - self.read_hit_rate()
    }

    /// Bloom filter hit rate (0.0 to 1.0)
    pub fn bloom_filter_hit_rate(&self) -> f64 {
        let hits = self.bloom_filter_hits.get();
        let misses = self.bloom_filter_misses.get();
        let total = hits + misses;
        if total == 0 {
            return 0.0;
        }
        hits as f64 / total as f64
    }

    /// Bloom filter false positive rate (0.0 to 1.0)
    pub fn bloom_filter_fp_rate(&self) -> f64 {
        let fps = self.bloom_filter_false_positives.get();
        let total_checks = self.bloom_filter_hits.get() + self.bloom_filter_misses.get();
        if total_checks == 0 {
            return 0.0;
        }
        fps as f64 / total_checks as f64
    }

    /// Compaction space savings (0.0 to 1.0)
    pub fn compaction_space_savings(&self) -> f64 {
        let bytes_in = self.compaction_bytes_in.get();
        let bytes_out = self.compaction_bytes_out.get();
        if bytes_in == 0 {
            return 0.0;
        }
        1.0 - (bytes_out as f64 / bytes_in as f64)
    }

    /// Write amplification factor
    pub fn write_amplification(&self) -> f64 {
        let logical_writes = self.writes_bytes.get();
        let physical_writes = self.compaction_bytes_out.get();
        if logical_writes == 0 {
            return 1.0;
        }
        physical_writes as f64 / logical_writes as f64
    }

    /// Format metrics for display
    pub fn summary(&self) -> String {
        format!(
            r#"Storage Engine Metrics
======================

Operations:
  Writes:      {:>12}  ({} MB)
  Reads:       {:>12}  (hits: {}, misses: {})
  Flushes:     {:>12}
  Compactions: {:>12}

Read Performance:
  Hit Rate:    {:>11.2}%
  Miss Rate:   {:>11.2}%

Bloom Filter:
  Hit Rate:    {:>11.2}%
  FP Rate:     {:>11.2}%
  Hits:        {:>12}
  Misses:      {:>12}
  False Pos:   {:>12}

Compaction:
  Input:       {:>9} MB
  Output:      {:>9} MB
  Space Saved: {:>11.2}%
  Write Amp:   {:>12.2}x

Latency (μs):
  Write p50:   {:>12.1}
  Write p99:   {:>12.1}
  Read p50:    {:>12.1}
  Read p99:    {:>12.1}
  Flush p50:   {:>12.1}
  Flush p99:   {:>12.1}

System State:
  MemTable:    {:>9} MB  ({} entries)
  Immutable:   {:>12}
  SSTables:    {:>12}
  WAL Size:    {:>9} MB
  Disk Usage:  {:>9} MB
"#,
            // Operations
            self.writes_total.get(),
            self.writes_bytes.get() / 1_048_576,
            self.reads_total.get(),
            self.reads_hits.get(),
            self.reads_misses.get(),
            self.flushes_total.get(),
            self.compactions_total.get(),
            // Read performance
            self.read_hit_rate() * 100.0,
            self.read_miss_rate() * 100.0,
            // Bloom filter
            self.bloom_filter_hit_rate() * 100.0,
            self.bloom_filter_fp_rate() * 100.0,
            self.bloom_filter_hits.get(),
            self.bloom_filter_misses.get(),
            self.bloom_filter_false_positives.get(),
            // Compaction
            self.compaction_bytes_in.get() / 1_048_576,
            self.compaction_bytes_out.get() / 1_048_576,
            self.compaction_space_savings() * 100.0,
            self.write_amplification(),
            // Latency
            self.write_latency.percentile(0.5).as_micros() as f64,
            self.write_latency.percentile(0.99).as_micros() as f64,
            self.read_latency.percentile(0.5).as_micros() as f64,
            self.read_latency.percentile(0.99).as_micros() as f64,
            self.flush_duration.percentile(0.5).as_micros() as f64,
            self.flush_duration.percentile(0.99).as_micros() as f64,
            // System state
            self.memtable_size_bytes.get() / 1_048_576,
            self.memtable_entries.get(),
            self.immutable_count.get(),
            self.sstable_count.get(),
            self.wal_size_bytes.get() / 1_048_576,
            self.disk_usage_bytes.get() / 1_048_576,
        )
    }

    /// Reset all metrics (useful for testing)
    pub fn reset(&self) {
        self.writes_total.reset();
        self.writes_bytes.reset();
        self.reads_total.reset();
        self.reads_hits.reset();
        self.reads_misses.reset();
        self.flushes_total.reset();
        self.compactions_total.reset();
        self.compaction_bytes_in.reset();
        self.compaction_bytes_out.reset();
        self.bloom_filter_hits.reset();
        self.bloom_filter_misses.reset();
        self.bloom_filter_false_positives.reset();
        self.write_latency.reset();
        self.read_latency.reset();
        self.flush_duration.reset();
        self.compaction_duration.reset();
        self.memtable_size_bytes.set(0);
        self.memtable_entries.set(0);
        self.immutable_count.set(0);
        self.sstable_count.set(0);
        self.disk_usage_bytes.set(0);
        self.wal_size_bytes.set(0);
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Atomic counter (lock-free)
#[derive(Debug)]
pub struct Counter {
    value: AtomicU64,
}

impl Counter {
    pub fn new() -> Self {
        Self {
            value: AtomicU64::new(0),
        }
    }

    pub fn inc(&self) {
        self.value.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add(&self, delta: u64) {
        self.value.fetch_add(delta, Ordering::Relaxed);
    }

    pub fn get(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }

    pub fn reset(&self) {
        self.value.store(0, Ordering::Relaxed);
    }
}

/// Gauge for tracking current state
#[derive(Debug)]
pub struct Gauge {
    value: AtomicU64,
}

impl Gauge {
    pub fn new() -> Self {
        Self {
            value: AtomicU64::new(0),
        }
    }

    pub fn set(&self, value: u64) {
        self.value.store(value, Ordering::Relaxed);
    }

    pub fn inc(&self) {
        self.value.fetch_add(1, Ordering::Relaxed);
    }

    pub fn dec(&self) {
        self.value.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn add(&self, delta: u64) {
        self.value.fetch_add(delta, Ordering::Relaxed);
    }

    pub fn sub(&self, delta: u64) {
        self.value.fetch_sub(delta, Ordering::Relaxed);
    }

    pub fn get(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }
}

/// Histogram for latency tracking
#[derive(Debug)]
pub struct Histogram {
    samples: RwLock<Vec<Duration>>,
    max_samples: usize,
}

impl Histogram {
    pub fn new() -> Self {
        Self::with_capacity(10_000)
    }

    pub fn with_capacity(max_samples: usize) -> Self {
        Self {
            samples: RwLock::new(Vec::with_capacity(max_samples)),
            max_samples,
        }
    }

    /// Record a latency sample
    pub fn observe(&self, duration: Duration) {
        let mut samples = self.samples.write();

        // Reservoir sampling to bound memory
        if samples.len() < self.max_samples {
            samples.push(duration);
        } else {
            // Replace random sample (simple reservoir sampling)
            let idx = fastrand::usize(..samples.len());
            samples[idx] = duration;
        }
    }

    /// Get percentile (0.0 to 1.0)
    pub fn percentile(&self, p: f64) -> Duration {
        let samples = self.samples.read();

        if samples.is_empty() {
            return Duration::ZERO;
        }

        let mut sorted: Vec<Duration> = samples.clone();
        sorted.sort();

        let idx = ((sorted.len() - 1) as f64 * p) as usize;
        sorted[idx]
    }

    pub fn count(&self) -> usize {
        self.samples.read().len()
    }

    pub fn reset(&self) {
        self.samples.write().clear();
    }
}

/// Timer helper for automatic latency tracking
pub struct Timer {
    start: Instant,
    histogram: Arc<Histogram>,
}

impl Timer {
    pub fn new(histogram: Arc<Histogram>) -> Self {
        Self {
            start: Instant::now(),
            histogram,
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        let duration = self.start.elapsed();
        self.histogram.observe(duration);
    }
}

/// Convenience macro for timing operations
#[macro_export]
macro_rules! time {
    ($histogram:expr, $block:expr) => {{
        let _timer = $crate::metrics::Timer::new(std::sync::Arc::new($histogram));
        $block
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_counter() {
        let counter = Counter::new();
        assert_eq!(counter.get(), 0);

        counter.inc();
        assert_eq!(counter.get(), 1);

        counter.add(99);
        assert_eq!(counter.get(), 100);

        counter.reset();
        assert_eq!(counter.get(), 0);
    }

    #[test]
    fn test_gauge() {
        let gauge = Gauge::new();
        assert_eq!(gauge.get(), 0);

        gauge.set(42);
        assert_eq!(gauge.get(), 42);

        gauge.inc();
        assert_eq!(gauge.get(), 43);

        gauge.dec();
        assert_eq!(gauge.get(), 42);

        gauge.add(10);
        assert_eq!(gauge.get(), 52);

        gauge.sub(2);
        assert_eq!(gauge.get(), 50);
    }

    #[test]
    fn test_histogram() {
        let hist = Histogram::new();

        // Add samples
        for i in 1..=100 {
            hist.observe(Duration::from_micros(i));
        }

        assert_eq!(hist.count(), 100);

        // Check percentiles
        let p50 = hist.percentile(0.5);
        let p99 = hist.percentile(0.99);

        assert!(p50.as_micros() >= 45 && p50.as_micros() <= 55);
        assert!(p99.as_micros() >= 95 && p99.as_micros() <= 100);
    }

    #[test]
    fn test_computed_metrics() {
        let metrics = Metrics::new();

        // Test read hit rate
        metrics.reads_total.add(100);
        metrics.reads_hits.add(80);
        metrics.reads_misses.add(20);
        assert_eq!(metrics.read_hit_rate(), 0.8);
        assert!((metrics.read_miss_rate() - 0.2).abs() < 0.0001);

        // Test compaction savings
        metrics.compaction_bytes_in.add(1000);
        metrics.compaction_bytes_out.add(300);
        assert_eq!(metrics.compaction_space_savings(), 0.7);
    }

    #[test]
    fn test_concurrent_counter() {
        let counter = Arc::new(Counter::new());
        let mut handles = vec![];

        for _ in 0..10 {
            let counter = Arc::clone(&counter);
            handles.push(thread::spawn(move || {
                for _ in 0..1000 {
                    counter.inc();
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(counter.get(), 10_000);
    }
}
