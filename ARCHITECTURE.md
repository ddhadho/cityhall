# Architecture Deep Dive

This document explains the technical design decisions, trade-offs, and implementation details of the time-series LSM storage engine. Written for technical interviews and code reviews.

## Table of Contents
1. [System Overview](#system-overview)
2. [Write Path](#write-path)
3. [Read Path](#read-path)
4. [Crash Recovery](#crash-recovery)
5. [Data Structures](#data-structures)
6. [File Format](#file-format)
7. [Performance Analysis](#performance-analysis)
8. [Design Trade-offs](#design-trade-offs)

---

## System Overview

### Why LSM-Tree?

**Problem**: Time-series workloads have write-heavy patterns (sensors, logs, metrics). Traditional B-trees optimize for reads but cause random I/O on writes.

**Solution**: LSM-tree batches writes in memory, flushes sequentially to disk. Trade read complexity for write throughput.

```
Write Pattern:
  B-tree:    Random I/O per insert (slow on spinning disks)
  LSM-tree:  Sequential batch writes (SSD-friendly)

Read Pattern:
  B-tree:    Single lookup (fast)
  LSM-tree:  Multiple level checks (slower, but cacheable)
```

### Component Responsibilities

| Component | Purpose | Memory | Disk | Mutable |
|-----------|---------|--------|------|---------|
| **WAL** | Crash recovery | Batched buffer | Append-only log | Yes |
| **MemTable** | Fast writes | BTreeMap | None | Yes |
| **SSTable** | Persistent storage | Index only | Compressed blocks | No |
| **StorageEngine** | Orchestration | Minimal | Manages files | Yes |

---

## Write Path

### Step-by-Step Flow

```rust
engine.put(key, value)
  │
  ├─► 1. Append to WAL (batched, fsync on threshold)
  │      • Durability guarantee
  │      • ~100μs latency with batching
  │
  ├─► 2. Insert into MemTable (BTreeMap)
  │      • O(log n) insertion
  │      • Size tracking
  │
  └─► 3. Check if MemTable full (64MB default)
        │
        └─► Yes → Flush to SSTable
              │
              ├─► Create SSTable writer
              ├─► Write sorted entries with compression
              ├─► Build index + bloom filter
              ├─► Sync to disk
              └─► Open for reads
```

### WAL Batching Strategy

**Problem**: fsync() on every write = ~1000 writes/sec (disk seek latency)

**Solution**: Buffer writes, fsync periodically or on threshold.

```rust
// Buffer config
const BUFFER_SIZE: usize = 16 * 1024; // 16KB
const FLUSH_INTERVAL: Duration = Duration::from_millis(100);

// Automatic flush triggers:
1. Buffer full (BUFFER_SIZE reached)
2. Explicit flush call
3. Drop (RAII pattern - no data loss)
```

**Result**: 184,776 writes/sec (2.8x improvement on NVMe)

### MemTable Size Calculation

```rust
entry_size = key.len() + value.len() + 8 (timestamp) + 40 (overhead)

// BTreeMap overhead:
// - Node pointers: 16 bytes
// - Balancing metadata: ~24 bytes per entry
// Total: ~40 bytes

if memtable.size >= 64MB:
    flush_to_sstable()
```

**Why 64MB?**
- Large enough: Amortizes flush cost
- Small enough: Limits memory usage
- Balance: More frequent flushes = more SSTables = slower reads

---

## Read Path

### Multi-Level Search

```rust
get(key) -> Result<Option<Value>>
  │
  ├─► 1. Check MemTable (O(log n), <1μs)
  │      Found? → Return immediately
  │      
  ├─► 2. Check Immutable MemTable (if exists)
  │      • MemTable being flushed
  │      • Prevents blocking writes
  │
  └─► 3. Check SSTables (newest to oldest)
        │
        ├─► Bloom filter check (~1μs)
        │      • 1% false positive rate
        │      • Avoids disk I/O for missing keys
        │
        ├─► Binary search index (in-memory)
        │      • Find block containing key
        │      • O(log B) where B = num_blocks
        │
        ├─► Read block from disk (~100μs)
        │      • 16KB compressed blocks
        │      • Sequential within block
        │
        └─► Decompress + search block
              • Snappy decompression (~10μs)
              • Linear scan (keys sorted)
```

### Bloom Filter Mathematics

```
False positive probability: p = (1 - e^(-kn/m))^k

Where:
  m = bits in filter
  n = number of elements
  k = hash functions

Our config:
  n = 10,000 keys per SSTable
  p = 0.01 (1% false positive rate)
  m = -n*ln(p) / (ln(2)^2) ≈ 95,851 bits ≈ 12KB
  k = (m/n) * ln(2) ≈ 7 hash functions

Cost:
  12KB per SSTable × 100 SSTables = 1.2MB
  Acceptable memory overhead for 10x read speedup
```

### Index Structure

```rust
// In-memory per SSTable
struct Index {
    entries: Vec<IndexEntry>,  // Sorted by first_key
}

struct IndexEntry {
    first_key: Vec<u8>,  // First key in block
    offset: u64,         // File position
    size: u32,           // Compressed size
}

// Size calculation:
// 100 blocks × 40 bytes = 4KB per SSTable
// 100 SSTables × 4KB = 400KB total
// Small enough to keep in memory
```

---

## Crash Recovery

### Durability Guarantees

```
Scenario 1: Crash before MemTable flush
  1. Restart
  2. Replay WAL → Rebuild MemTable
  3. Load existing SSTables
  ✓ All data recovered

Scenario 2: Crash during SSTable flush
  1. Restart  
  2. Replay WAL (includes unflushed data)
  3. Load completed SSTables only
  4. Incomplete SSTable ignored (corrupted header)
  ✓ Partial SSTable discarded, data in WAL

Scenario 3: Corrupted SSTable
  1. Read path detects corruption
  2. Log warning, skip SSTable
  3. Continue with other SSTables
  ✓ Graceful degradation
```

### WAL Format

```
Entry:
  [checksum: 4 bytes]
  [key_len: 4 bytes]
  [value_len: 4 bytes]
  [timestamp: 8 bytes]
  [op_type: 1 byte]  // Put=0, Delete=1
  [key: key_len bytes]
  [value: value_len bytes]

Checksum: CRC32(key_len || value_len || timestamp || op_type || key || value)
```

**Corruption detection**: On replay, verify checksum. Stop at first mismatch (assumes append-only, no partial writes in middle).

---

## Data Structures

### MemTable: BTreeMap

**Chosen over**: HashMap, SkipList, Custom AVL

```rust
BTreeMap<Vec<u8>, (Vec<u8>, Timestamp)>
```

**Pros**:
- Sorted order (required for SSTable flush)
- O(log n) operations
- Simple, well-tested (std library)
- Cache-friendly (fewer allocations than trees)

**Cons**:
- Not lock-free (uses RwLock)
- Could be faster with SkipList for concurrent writes

**Future**: Consider lock-free skiplist if profiling shows lock contention.

### SSTable Index: Vec\<IndexEntry\>

**Chosen over**: HashMap, BTreeMap, Trie

```rust
Vec<IndexEntry>  // Sorted, immutable after creation
```

**Pros**:
- Minimal memory overhead
- Binary search is O(log n)
- Cache-friendly (contiguous memory)
- Immutable = no synchronization needed

**Cons**:
- Can't add entries after creation (okay, SSTables immutable)

---

## File Format

### SSTable Structure (Detailed)

```
Offset   | Size    | Content
---------|---------|------------------------------------------
0        | 64      | Header
         |         |   magic: 0x53535442 (4 bytes)
         |         |   version: 1 (4 bytes)
         |         |   num_blocks: N (4 bytes)
         |         |   min_timestamp: T_min (8 bytes)
         |         |   max_timestamp: T_max (8 bytes)
         |         |   padding: 36 bytes
---------|---------|------------------------------------------
64       | varies  | Data Block 0 (Snappy compressed)
64+B0    | varies  | Data Block 1
...      | ...     | ...
X        | varies  | Bloom Filter
         |         |   bits: variable
         |         |   num_hashes: k (serialized)
Y        | varies  | Index Block
         |         |   For each block:
         |         |     key_len: 2 bytes
         |         |     first_key: key_len bytes
         |         |     offset: 8 bytes
         |         |     size: 4 bytes
Z        | 64      | Footer
         |         |   index_offset: Y (8 bytes)
         |         |   bloom_offset: X (8 bytes)
         |         |   index_size: (4 bytes)
         |         |   bloom_size: (4 bytes)
         |         |   checksum: (4 bytes)
         |         |   padding: 36 bytes
```

### Block Format (Before Compression)

```
Entry format (prefix compressed):
  [shared_len: varint]     # Bytes shared with previous key
  [unshared_len: varint]   # Bytes unique to this key
  [value_len: varint]
  [key_delta: unshared_len bytes]
  [value: value_len bytes]
  [timestamp: 8 bytes]

Example:
  Entry 0: shared=0, unshared=16, key="sensor_001_temp"
  Entry 1: shared=11, unshared=3, key="sensor_001_hum" (shares "sensor_001_")
  Entry 2: shared=11, unshared=5, key="sensor_001_volt"

Space savings: 
  Uncompressed: 16+15+17 = 48 bytes
  Compressed: 16+3+5 = 24 bytes (50% reduction before Snappy!)
```

### Why Separate Index from Data?

**Design choice**: Index at end, not interleaved.

**Alternative**: Index entries embedded with blocks.

**Chosen approach**:
```
Pros:
- Load entire index at open (one read)
- Binary search without disk seeks
- Smaller index (only first_key per block)

Cons:
- Must read to end of file on open
- Can't stream index during write
```

**Rationale**: Read-optimized. Index is small (<1MB for 1GB file), worth loading upfront.

---

## Performance Analysis

### Write Amplification

```
Original write: 1KB
  ↓
WAL: 1KB written
MemTable: 1KB (memory)
Flush: 1KB → SSTable (compressed to ~100 bytes)
Compaction: 100 bytes × 10 levels = 1KB

Total disk writes: 1KB (WAL) + 100B (SSTable) + 1KB (compaction) = ~2KB
Write amplification: 2x
```

**Compared to B-tree**: 5-10x write amplification (node splits, rebalancing)

### Read Amplification

```
Worst case (key not found):
  1 MemTable check (memory)
  10 SSTable bloom checks (memory)
  0 disk reads (bloom says "not found")
  
Average case (key in SSTable 5):
  1 MemTable check (memory)
  5 bloom checks (memory)
  1 disk read (16KB block)
  Read amplification: 1x

Compared to B-tree: 1x (direct lookup)
```

### Space Amplification

```
Data: 1GB
  WAL: ~100MB (recent writes)
  MemTable: 64MB
  SSTables: 1GB (no compaction yet)
  Total: 1.164GB
  
Space amplification: 1.16x

With compaction:
  SSTables: ~300MB (10:1 compression + dedup)
  Total: ~464MB
  Space amplification: 0.46x
```

---

## Design Trade-offs

### 1. Block Size (16KB)

| Size | Pros | Cons |
|------|------|------|
| **4KB** | Better granularity | Worse compression, larger index |
| **16KB** ✓ | Good compression | May read excess data |
| **64KB** | Best compression | Wastes bandwidth on small reads |

**Chosen**: 16KB balances compression ratio (10:1) with read efficiency.

### 2. Flush Threshold (64MB)

| Size | Pros | Cons |
|------|------|------|
| **16MB** | Less memory | Frequent flushes, more SSTables |
| **64MB** ✓ | Balanced | Moderate memory |
| **256MB** | Fewer SSTables | High memory, longer recovery |

**Chosen**: 64MB matches typical L1 cache + RAM availability.

### 3. Batching Buffer (16KB)

| Size | Pros | Cons |
|------|------|------|
| **1KB** | Low latency | Frequent fsyncs, low throughput |
| **16KB** ✓ | 2.8x throughput | ~10ms latency |
| **1MB** | Max throughput | Unacceptable latency for writes |

**Chosen**: 16KB gives near-optimal throughput with acceptable latency.

### 4. Synchronous Flush

**Current**: Flush blocks writes.

```rust
if memtable.full() {
    flush_to_sstable();  // Blocks ~100ms
}
```

**Alternative**: Background flush with dual MemTables.

```rust
if memtable.full() {
    immutable_memtable = Some(memtable);
    memtable = MemTable::new();
    spawn_background_flush(immutable_memtable);
}
```

**Trade-off**: Simplicity now vs. performance later. Week 2 optimization.

---

## Testing Strategy

### Unit Tests (28 passing)
- **MemTable**: Insert, update, scan, size tracking
- **WAL**: Append, flush, recovery, corruption detection, cleanup
- **SSTable**: Write, read, compression, index search, bloom filter
- **Compaction**: Correctness of merge logic, SSTable selection
- **Metrics**: Counters, gauges, histograms

### Integration Tests (12 passing)
- End-to-end write → read
- Crash recovery
- Flush behavior
- Compaction (duplicate removal, space reduction)
- Large values, many keys
- Binary data handling

### Remaining Tests (Future)
- [ ] Concurrent writes
- [ ] Memory leak checks
- [ ] Stress tests (multi-hour runs)
- [ ] Property-based tests (e.g., QuickCheck)

---

## Future Optimizations

### Completed (Week 1-2)
- ✅ **Bloom Filters**: Custom implementation, 442x speedup on misses.
- ✅ **Background Flush**: Dual MemTable, 93% p99 latency improvement.
- ✅ **Size-Tiered Compaction**: k-way merge, 97% space savings.
- ✅ **Comprehensive Metrics**: 16 key metrics for observability.
- ✅ **Daemon & systemd**: Runs as a persistent background service.

### High Priority (Next Steps)
- [ ] **Delete Support**: Tombstones for key deletion and compaction cleanup.
- [ ] **Block Cache**: In-memory LRU cache for decompressed SSTable blocks.
- [ ] **Metrics Exposure**: Expose internal metrics via a client command or network endpoint (e.g., Prometheus).

### Medium Priority
- [ ] **Leveled Compaction**: Explore a level-based strategy (RocksDB style) for better read amplification.
- [ ] **Snapshot Isolation**: MVCC for concurrent readers without locks.
- [ ] **Compression Tuning**: Experiment with LZ4, Zstd.

---

## References

**Papers**:
- [The Log-Structured Merge-Tree](https://www.cs.umb.edu/~poneil/lsmtree.pdf) - O'Neil et al., 1996
- [Bigtable](https://static.googleusercontent.com/media/research.google.com/en//archive/bigtable-osdi06.pdf) - Chang et al., 2006

**Code**:
- [LevelDB](https://github.com/google/leveldb) - Original Google implementation
- [RocksDB](https://github.com/facebook/rocksdb) - Facebook's optimized fork

**Books**:
- Designing Data-Intensive Applications - Martin Kleppmann (Chapter 3)
- Database Internals - Alex Petrov (Chapters 1-7)

---

*Last updated: Day 14 - Code cleanup, documentation, and daemonization*
