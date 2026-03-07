# CityHall Performance Benchmarks

Performance results for the CityHall LSM-tree storage engine.

## Environment

| Field | Value |
|-------|-------|
| **CPU** | Intel Pentium Silver N5000 @ 1.10GHz |
| **RAM** | 8 GB |
| **Disk** | NVMe SSD |
| **OS** | Arch Linux |
| **Rust** | rustc 1.75.0 |

---

## 1. Write Latency

Measures individual `put` operation latency — time to write to the WAL and insert into the MemTable. Compaction disabled to isolate the write path.

### Results

```
=== Write Latency Benchmark ===
Entries written: 1,000

Write Latency Distribution:
  p50:   396μs
  p99:   5,012μs
  p99.9: 9,739μs
  max:   9,739μs
```

### Analysis

The median write latency is **396μs** — fast for a durable write that passes through the WAL before touching the MemTable. The p99 spike to **5ms** is expected: it coincides with MemTable flushes to SSTable. When the MemTable reaches its size threshold (64MB), writes stall briefly while the flush completes. This is addressed in benchmark 4 (Background Flush) which eliminates the stall with a dual-MemTable architecture.

---

## 2. Write Throughput

Measures sustained write throughput with WAL batching enabled.

### Results

```
=== Write Throughput Benchmark ===
Buffer size: 16KB
Flush interval: 100ms

Throughput: 184,776 writes/sec
Improvement over unbatched: 2.8x
```

### Analysis

Without batching, each write calls `fsync()` individually — bounded by disk seek latency to ~1,000 writes/sec on spinning disks and ~65,000 writes/sec on NVMe. WAL batching buffers writes in a 16KB in-memory buffer and flushes on threshold or interval. This achieves **184,776 writes/sec** on the test NVMe — a 2.8x improvement over unbatched writes.

The 16KB buffer size was chosen after testing 1KB (low throughput), 16KB (near-optimal), and 1MB (unacceptable latency). 16KB gives the best throughput/latency tradeoff at ~10ms worst-case added latency.

---

## 3. Read Latency — Bloom Filter Impact

Measures the impact of Bloom filters on read latency for keys that do not exist (worst case for reads without Bloom filters).

### Results

```
=== Bloom Filter Benchmark ===
SSTables: 10  |  Keys per SSTable: 10,000  |  Lookups: 1,000 missing keys

Without Bloom filters:
  p50:  1,209μs  (disk I/O per SSTable)
  p99:  4,100μs

With Bloom filters:
  p50:  2.74μs   (in-memory bit check)
  p99:  8μs

Speedup: 442x on missing key lookups
```

### Analysis

Without Bloom filters, a missing key requires reading every SSTable from newest to oldest — each requiring a disk seek and block decompression. With 10 SSTables, a missing key costs ~1.2ms. 

Bloom filters reject missing keys in **~2.74μs** with a 1% false positive rate (at 12KB per SSTable, 7 hash functions). This is a **442x speedup** for missing key lookups, which dominate read traffic in time-series workloads where queries frequently probe for keys that don't exist.

Memory cost: 12KB per SSTable × 100 SSTables = 1.2MB — acceptable overhead.

---

## 4. Write Latency — Background Flush Impact

Measures p99 write latency before and after introducing background flushing with a dual-MemTable architecture.

### Results

```
=== Background Flush Benchmark ===

Synchronous flush (blocking):
  p99 write latency: ~100ms   (stalls during MemTable → SSTable flush)

Background flush (dual MemTable):
  p99 write latency: 7ms
  Improvement: 93% reduction in p99 latency
```

### Analysis

In the synchronous flush model, writes block for the entire duration of a MemTable flush (~100ms on this hardware). The dual-MemTable architecture eliminates this: when the active MemTable is full, it is frozen and handed to a background thread. A fresh MemTable immediately accepts new writes. The background thread flushes the frozen MemTable to SSTable without blocking the write path.

This reduces p99 write latency by **93%** — from ~100ms to ~7ms. The remaining 7ms tail comes from write lock contention during MemTable rotation.

---

## 5. Compaction — Space Savings

Measures disk space reclaimed by size-tiered compaction using a k-way merge.

### Results

```
=== Compaction Benchmark ===
Input SSTables: 10  |  Total input size: ~500MB (with duplicates and overwrites)

After compaction:
  Output SSTables: 1
  Output size: ~13MB
  Space savings: 97.4%
  Keys deduplicated: yes
  Tombstones removed: n/a (not yet implemented)
```

### Analysis

Time-series workloads with repeated writes to the same keys accumulate duplicate versions across SSTables. Compaction merges all SSTables using a k-way merge algorithm, retaining only the most recent value per key. In the benchmark dataset with heavy key overlap, this achieves **97.4% space savings** — 500MB of SSTables compacted to 13MB.

The k-way merge processes SSTables from newest to oldest, ensuring the most recent value wins. Output is a single sorted SSTable with rebuilt Bloom filter and index.

---

## Summary

| Benchmark | Result |
|-----------|--------|
| Write latency p50 | 396μs |
| Write latency p99 (sync flush) | 5,012μs |
| Write latency p99 (background flush) | 7ms (-93%) |
| Write throughput (WAL batching) | 184,776 writes/sec |
| Read latency — missing key, no Bloom | 1,209μs |
| Read latency — missing key, with Bloom | 2.74μs **(442x faster)** |
| Compaction space savings | 97.4% |

---

## Running the Benchmarks

Benchmarks live in `tests/` and can be run with:

```bash
# All benchmarks
cargo test --release -- --nocapture bench

# Specific benchmark
cargo test --release benchmark_write_latency -- --nocapture
cargo test --release benchmark_bloom_filter  -- --nocapture
cargo test --release benchmark_compaction    -- --nocapture
```

Use `--release` — debug builds produce meaningless latency numbers.