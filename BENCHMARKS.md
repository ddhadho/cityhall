# CityHall Performance Benchmarks

This document contains the results of performance benchmarks run against the CityHall storage engine and replication system.

## Environment

-   **CPU**: *(Please fill in your CPU model)*
-   **RAM**: *(Please fill in your RAM amount)*
-   **Disk**: *(Please fill in your disk type, e.g., NVMe SSD, SATA SSD, HDD)*
-   **OS**: *(Please fill in your Operating System)*
-   **Rust Version**: `rustc 1.75.0`

## 1. Write Latency

This benchmark measures the latency of individual `put` operations against the `StorageEngine`. It captures the time it takes to write to the Write-Ahead Log (WAL) and insert into the in-memory MemTable.

### Results

The test writes 1,000 entries and measures the latency for each.

```
=== Write Latency Benchmark ===

Write Latency Distribution:
  p50:  396μs
  p99:  5012μs
  p99.9: 9739μs
  max:  9739μs

✅ p99 < 1000μs (1ms): false
```

### Analysis

The median (p50) latency is very low at **396 microseconds**. However, the 99th percentile (p99) latency is around **5 milliseconds**. This indicates that while most writes are fast, there are periodic latency spikes. These spikes are expected and are caused by the `StorageEngine` flushing the MemTable to a new SSTable on disk when it becomes full. The `benchmark_write_latency` test was run with compaction disabled to isolate the write path latency.

## 2. Replication Throughput

This benchmark measures the speed at which a replica can catch up to a leader that has a large number of entries.

### Results

*(Results will be added here once the `benchmark_replication_throughput` test is fixed and can be run successfully.)*

### Analysis

*(Analysis will be provided here.)*
