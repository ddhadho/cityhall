# CityHall WAL Analysis - Actual Findings

**Date**: January 4, 2026  
**Analyzed By**: CityHall Sync Team  
**Source File**: `src/wal.rs` (Segmented Write-Ahead Log)

---

## Executive Summary

✅ **EXCELLENT NEWS**: CityHall's WAL is well-structured and perfect for replication!

**Key Findings**:
- ✅ **Segmented architecture** - Multiple WAL files instead of one giant file
- ✅ **Clear binary format** - Length-prefixed records with CRC32 checksums
- ✅ **Recovery function exists** - `Wal::recover()` already reads all entries
- ✅ **Operation types supported** - Put and Delete operations
- ⚠️ **No explicit sequence numbers** - Uses timestamp for ordering
- ✅ **Rotation & cleanup** - Automatic segment rotation and cleanup

**Replication Implications**:
- We can leverage existing `recover_segment()` for reading
- Need to add sequence numbers or use (segment_number, offset) as logical sequence
- Cleanup mechanism means we need WAL retention for replicas

---

## File Structure

### Location
```
~/.cityhall/data/wal_segments/
├── 000001.wal  (100MB max, rotated)
├── 000002.wal  (100MB max, rotated)
├── 000003.wal  (active segment)
└── ...
```

### File Naming Convention
- **Pattern**: `{segment_number:06}.wal` (e.g., `000001.wal`, `000042.wal`)
- **Padding**: 6-digit zero-padded decimal
- **Extension**: `.wal`

### Segment Management

#### Rotation Policy
- **Segment size limit**: 100 MB (configurable via `DEFAULT_SEGMENT_SIZE`)
- **Rotation trigger**: When `bytes_written >= segment_size_limit`
- **New segment**: `segment_number += 1`
- **Code location**: `wal.rs:119` - `rotate_segment()`

#### Cleanup Policy
- **Trigger**: After MemTable flush to SSTable
- **Cleanup**: Deletes all segments where `segment_num < last_flushed_segment`
- **Code location**: `wal.rs:135` - `cleanup_old_segments()`
- **Safety**: Only deletes segments that have been persisted to SSTables

#### Retention Implications for Replication
⚠️ **CRITICAL**: Leader must NOT delete segments until all replicas have synced them!

**Solution Options**:
1. Track min(`replica_last_synced_segment`) across all replicas
2. Configure longer retention (keep N segments regardless of flush)
3. Replicas must register and report their position

---

## Entry Format

### Rust Struct Definition

**Location**: Not explicitly defined as a struct, but encoded/decoded in functions

**Effective Structure**:
```rust
pub struct Entry {
    pub key: Vec<u8>,        // Arbitrary bytes (typically UTF-8)
    pub value: Vec<u8>,      // Arbitrary bytes (empty for Delete)
    pub timestamp: u64,      // Microseconds since Unix epoch
}

pub enum OpType {
    Put = 0,
    Delete = 1,
}
```

**Note**: No explicit sequence number in the Entry struct!

### Binary Encoding

**Serialization method**: Custom binary format (not serde/bincode)

**Wire Format (encode_record)**:

```
┌─────────────────────────────────────────────────────────┐
│                    WAL Record Format                     │
├──────────┬──────┬───────────────────────────────────────┤
│ Offset   │ Size │ Field                                 │
├──────────┼──────┼───────────────────────────────────────┤
│ 0        │ 4    │ checksum (u32, little-endian, CRC32) │
│ 4        │ 2    │ data_length (u16, little-endian)     │
│ 6        │ 1    │ op_type (u8: 0=Put, 1=Delete)        │
│ 7        │ 8    │ timestamp (u64, microseconds)        │
│ 15       │ 2    │ key_length (u16)                     │
│ 17       │ N    │ key_data (N bytes)                   │
│ 17+N     │ 4    │ value_length (u32)                   │
│ 21+N     │ M    │ value_data (M bytes)                 │
└──────────┴──────┴───────────────────────────────────────┘

Total Record Size: 21 + key_length + value_length bytes
```

**Checksum Scope**:
```rust
// From encode_record() line 256
hasher.update(&(data_len as u16).to_le_bytes());  // Length
hasher.update(&[op_type as u8]);                  // Op type
hasher.update(&data);                             // All data fields
```

**Max Sizes**:
- Record: 65,535 bytes (u16::MAX)
- Key: 65,535 bytes (u16::MAX)
- Value: 4,294,967,295 bytes (u32::MAX)

---

## Sequence Numbering

### Current Implementation

**❌ NO EXPLICIT SEQUENCE NUMBERS**

The current WAL does **not** include sequence numbers in entries. Ordering is determined by:
1. Segment number (files are processed in order: 000001, 000002, ...)
2. Offset within segment (records are appended sequentially)
3. Timestamp (u64 microseconds, but not guaranteed unique)

**Code Evidence**:
```rust
// From encode_record() - line 244
data.put_u64_le(entry.timestamp);  // Only timestamp, no sequence
data.put_u16_le(entry.key.len() as u16);
// ... no sequence field
```

### Implications for Replication

**Problem**: Replicas need to track "last synced position" but there's no global sequence number.

**Solution Options**:

#### Option 1: Add Sequence Numbers (Recommended)
```rust
// Modify Entry struct
pub struct Entry {
    pub sequence: u64,      // NEW: Global sequence number
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub timestamp: u64,
}

// Modify Wal struct
pub struct Wal {
    // ... existing fields
    next_sequence: AtomicU64,  // NEW: Global counter
}
```

**Changes Needed**:
- Add `sequence` field to Entry
- Update `encode_record()` to include sequence
- Update `read_record()` to parse sequence
- Initialize `next_sequence` from recovery

**Pros**: Clean, standard approach, easy to track replication position
**Cons**: Requires modifying core WAL format (backward incompatible)

#### Option 2: Use (segment_number, offset) as Logical Sequence
```rust
#[derive(Serialize, Deserialize)]
pub struct ReplicaPosition {
    pub segment: u64,      // Which segment (e.g., 3)
    pub offset: u64,       // Byte offset within segment (e.g., 12345)
}
```

**Pros**: No WAL format changes needed
**Cons**: More complex to implement, requires byte-offset tracking

#### Option 3: Use Timestamp as Sequence (Not Recommended)
```rust
pub struct ReplicaState {
    pub last_synced_timestamp: u64,  // Microseconds
}
```

**Pros**: No changes needed
**Cons**: Timestamps not guaranteed unique, can't handle concurrent writes reliably

**Decision**: **Option 1 (Add Sequence Numbers)** - Most robust for replication

---

## Reading Strategy

### Existing APIs

**Public Methods** in `src/wal.rs`:

```rust
impl Wal {
    // Write operations
    pub fn append(&mut self, entry: &Entry) -> Result<()>
    pub fn append_delete(&mut self, key: &[u8], timestamp: u64) -> Result<()>
    pub fn flush(&mut self) -> Result<()>
    
    // Maintenance
    pub fn mark_flushed(&mut self) -> Result<()>
    pub fn cleanup_old_segments(&mut self) -> Result<()>
    
    // Recovery (reads ALL entries from ALL segments)
    pub fn recover(path: impl AsRef<Path>) -> Result<Vec<Entry>>
    
    // Size queries
    pub fn size(&self) -> Result<u64>
    pub fn file_size(&self) -> u64
}
```

**Private Methods** (can be exposed):

```rust
// Line 164 - Recovers entries from a single segment
fn recover_segment(path: &Path) -> Result<Vec<Entry>> {
    let mut file = File::open(path)?;
    let mut entries = Vec::new();
    
    loop {
        match read_record(&mut file) {  // Reads one record at a time
            Ok(Some((op_type, entry))) => entries.push(entry),
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

// Line 274 - Reads a single record from file
fn read_record(file: &mut File) -> Result<Option<(OpType, Entry)>>
```

### Reading from Sequence Number (NEW - To Implement)

**Current capability**: ❌ Cannot read from specific sequence (no sequences yet!)

**Implementation Strategy** (after adding sequence numbers):

```rust
impl Wal {
    /// Read all entries starting from a given sequence number
    /// Scans segments in order until finding start_seq, then reads forward
    pub fn read_from_seq(&self, start_seq: u64) -> Result<Vec<Entry>> {
        let mut segments = self.list_segments()?;
        segments.sort_by_key(|(num, _)| *num);
        
        let mut entries = Vec::new();
        let mut found_start = false;
        
        for (_segment_num, path) in segments {
            let segment_entries = Self::recover_segment(&path)?;
            
            for entry in segment_entries {
                if !found_start {
                    if entry.sequence >= start_seq {
                        found_start = true;
                        entries.push(entry);
                    }
                } else {
                    entries.push(entry);
                }
            }
            
            // Optimization: if we've started collecting and this segment
            // ended, all subsequent segments are definitely after start_seq
            if found_start {
                // Could break early if we know segment boundaries
            }
        }
        
        Ok(entries)
    }
    
    /// Get the highest sequence number currently in WAL
    pub fn latest_seq(&self) -> Result<u64> {
        // After adding sequence field, track this in memory
        Ok(self.next_sequence.load(Ordering::SeqCst).saturating_sub(1))
    }
    
    /// List all segment files
    fn list_segments(&self) -> Result<Vec<(u64, PathBuf)>> {
        let mut segments = Vec::new();
        
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if let Some(segment_num) = Self::parse_segment_number(&path) {
                segments.push((segment_num, path));
            }
        }
        
        Ok(segments)
    }
}
```

---

## Concurrency Model

### Write Concurrency

**Single writer**: ✅ YES (by design)
- The `Wal` struct takes `&mut self` for all write operations
- Rust's ownership ensures only one mutable reference exists
- No explicit locking needed at WAL level (caller handles it)

**Code evidence**:
```rust
pub fn append(&mut self, entry: &Entry) -> Result<()>  // &mut self
pub fn append_delete(&mut self, key: &[u8], timestamp: u64) -> Result<()>  // &mut self
```

### Read Concurrency

**Multiple readers**: ⚠️ DEPENDS ON CALLER

Current implementation:
- `recover()` is a **static method** - opens files independently
- `recover_segment()` opens files in read-only mode
- No built-in locking mechanism in WAL code

**Implication**: Safe to read segments concurrently from multiple processes/threads IF:
- Leader has flushed and closed the segment
- Or readers use read-only file handles

**For Active Segment**:
- `current_segment` is being written to
- Concurrent read of active segment could see partial writes
- **Solution**: Only replicate from closed/flushed segments

### Thread Safety for Replication

**Leader Serving Replicas While Writing**:

```rust
// SAFE approach:
// 1. Leader writes to active segment (e.g., 000003.wal)
// 2. Replication server reads from closed segments (000001.wal, 000002.wal)
// 3. When segment rotates, old segment becomes available for replication

pub struct ReplicationServer {
    wal_dir: PathBuf,  // Read from here
    // Does NOT hold a reference to Wal struct
}

impl ReplicationServer {
    // Reads closed segments only
    pub fn get_entries_from_segment(&self, segment_num: u64) -> Result<Vec<Entry>> {
        let path = self.wal_dir.join(format!("{:06}.wal", segment_num));
        Wal::recover_segment(&path)  // Safe - reads closed file
    }
}
```

**Architecture Decision**:
- Leader's `Wal` struct handles writes
- Replication server reads segment files directly (not through Wal struct)
- No shared mutable state between writer and readers
- Only read closed segments to avoid partial reads

---

## Integration Points for Replication

### Option 1: Segment-Based Replication (RECOMMENDED)

**Approach**: Replicate entire segments after they're closed

```rust
// Leader notifies replication server when segment rotates
impl Wal {
    fn rotate_segment(&mut self) -> Result<()> {
        self.current_segment.flush()?;
        
        let closed_segment = self.segment_number;
        
        // NEW: Notify replication layer
        if let Some(hook) = &self.replication_hook {
            hook.on_segment_closed(closed_segment);
        }
        
        self.segment_number += 1;
        self.current_segment = WalSegment::new(
            &self.dir,
            self.segment_number,
            self.current_segment.buffer_capacity,
        )?;
        
        Ok(())
    }
}

// Replication hook trait
pub trait ReplicationHook: Send + Sync {
    fn on_segment_closed(&self, segment_number: u64);
}
```

**Replica Tracking**:
```rust
pub struct ReplicaState {
    pub replica_id: String,
    pub last_synced_segment: u64,  // e.g., "synced up to segment 5"
}

// Replica requests: "Give me segment 6"
// Leader sends entire segment file (or all entries from it)
```

**Pros**:
- Simple: replicate whole segments at a time
- No need to track individual entries
- Efficient: batch transfer
- Natural with existing segment architecture

**Cons**:
- Coarser granularity (100 MB per sync)
- Replicas lag by up to one segment

### Option 2: Entry-Based Replication (Requires Sequence Numbers)

**Approach**: After adding sequence numbers, replicate individual entries

```rust
impl Wal {
    pub fn read_from_seq(&self, start_seq: u64) -> Result<Vec<Entry>> {
        // Implementation shown earlier
    }
}

pub struct ReplicaState {
    pub last_synced_seq: u64,  // e.g., "synced up to entry 12345"
}
```

**Pros**:
- Fine-grained replication
- Lower latency (don't wait for segment to close)
- Standard replication approach

**Cons**:
- Requires adding sequence numbers to WAL
- More complex implementation

### Chosen Approach

**Week 1 MVP**: **Option 1 (Segment-Based)** - Use existing code, no format changes
**Week 2 Enhancement**: Add sequence numbers for finer-grained replication

---

## Modifications Needed

### Phase 1: Segment-Based Replication (No WAL Changes)

#### 1. Expose Segment Reading (NEW PUBLIC API)
```rust
// Add to src/wal.rs
impl Wal {
    /// Get list of closed segments available for replication
    pub fn list_closed_segments(&self) -> Result<Vec<u64>> {
        let mut segments = Vec::new();
        
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if let Some(num) = Self::parse_segment_number(&path) {
                // Only return closed segments (not current)
                if num < self.segment_number {
                    segments.push(num);
                }
            }
        }
        
        segments.sort_unstable();
        Ok(segments)
    }
    
    /// Read all entries from a specific segment
    pub fn read_segment(&self, segment_num: u64) -> Result<Vec<Entry>> {
        let path = self.dir.join(format!("{:06}.wal", segment_num));
        Self::recover_segment(&path)
    }
    
    /// Get current active segment number
    pub fn current_segment_number(&self) -> u64 {
        self.segment_number
    }
    
    /// Get directory path (for replication server to read directly)
    pub fn segment_dir(&self) -> &Path {
        &self.dir
    }
}
```

#### 2. Prevent Premature Cleanup (MODIFY EXISTING)
```rust
// Modify cleanup_old_segments() to respect replication state
impl Wal {
    pub fn cleanup_old_segments(&mut self, min_replica_segment: u64) -> Result<()> {
        let mut deleted_count = 0;
        let mut reclaimed_bytes = 0u64;
        
        // NEW: Consider both flush point AND replica positions
        let safe_to_delete = self.last_flushed_segment.min(min_replica_segment);
        
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if let Some(segment_num) = Self::parse_segment_number(&path) {
                // CHANGED: Use safe_to_delete instead of last_flushed_segment
                if segment_num < safe_to_delete {
                    let size = entry.metadata()?.len();
                    std::fs::remove_file(&path)?;
                    deleted_count += 1;
                    reclaimed_bytes += size;
                    
                    println!("🗑️  Deleted WAL segment {:06}.wal ({} bytes)",
                             segment_num, size);
                }
            }
        }
        
        if deleted_count > 0 {
            println!("✅ WAL cleanup: {} segments, {} MB reclaimed",
                     deleted_count, reclaimed_bytes / 1_048_576);
        }
        
        Ok(())
    }
}
```

#### 3. Add Replication Hook (OPTIONAL, for push notifications)
```rust
// Add to src/wal.rs or new file src/replication/hooks.rs

pub trait ReplicationHook: Send + Sync {
    fn on_segment_closed(&self, segment_number: u64);
}

// Add to Wal struct
pub struct Wal {
    // ... existing fields
    replication_hook: Option<Arc<dyn ReplicationHook>>,
}

impl Wal {
    pub fn set_replication_hook(&mut self, hook: Arc<dyn ReplicationHook>) {
        self.replication_hook = Some(hook);
    }
}
```

### Phase 2: Entry-Based Replication (Add Sequence Numbers)

**Future Enhancement** - Skip for MVP, implement if time permits in Week 2

---

## Testing Plan

### Unit Tests (Add to tests/wal_tests.rs)

```rust
#[test]
fn test_read_specific_segment() {
    let dir = tempdir().unwrap();
    let mut wal = Wal::new(dir.path().join("test.wal"), 1024).unwrap();
    
    // Write to force multiple segments
    wal.segment_size_limit = 10_000; // Small for testing
    
    for i in 0..100 {
        let entry = Entry {
            key: format!("key_{}", i).into_bytes(),
            value: vec![0u8; 200],
            timestamp: 1000 + i,
        };
        wal.append(&entry).unwrap();
    }
    
    wal.flush().unwrap();
    
    // Read specific segment
    let closed_segments = wal.list_closed_segments().unwrap();
    assert!(closed_segments.len() > 0, "Should have closed segments");
    
    let entries = wal.read_segment(closed_segments[0]).unwrap();
    assert!(entries.len() > 0, "Segment should have entries");
}

#[test]
fn test_cleanup_respects_replica_position() {
    let dir = tempdir().unwrap();
    let mut wal = Wal::new(dir.path().join("test.wal"), 1024).unwrap();
    
    // Create multiple segments
    wal.segment_size_limit = 10_000;
    for i in 0..100 {
        let entry = Entry {
            key: format!("key_{}", i).into_bytes(),
            value: vec![0u8; 200],
            timestamp: 1000 + i,
        };
        wal.append(&entry).unwrap();
    }
    
    let segments_before = wal.list_closed_segments().unwrap();
    
    // Mark as flushed
    wal.mark_flushed().unwrap();
    
    // Cleanup but replica is at segment 2
    wal.cleanup_old_segments(2).unwrap();
    
    let segments_after = wal.list_closed_segments().unwrap();
    
    // Should keep segment 2 and later
    assert!(segments_after.contains(&2), "Should keep segment 2 for replica");
}
```

### Integration Tests (New file: tests/replication_wal.rs)

```rust
#[tokio::test]
async fn test_concurrent_write_and_read() {
    // Leader writes while replication server reads closed segments
    // Verify no data corruption or race conditions
}

#[test]
fn test_segment_boundary_recovery() {
    // Write entry that spans segment boundary
    // Verify replica can recover correctly
}
```

---

## Key Decisions for Replication

### 1. Segment-Based vs Entry-Based
**Decision**: Start with segment-based (Week 1 MVP)
**Rationale**: 
- No WAL format changes needed
- Leverages existing segment architecture
- Can add entry-based in Week 2 if time permits

### 2. When to Replicate
**Decision**: Replicate closed segments only (not active segment)
**Rationale**:
- Avoids reading partial writes
- Simpler concurrency model
- Natural checkpoint boundaries

### 3. WAL Retention
**Decision**: Track replica positions, don't delete needed segments
**Rationale**:
- Prevents data loss if replica is behind
- Leader needs `cleanup_old_segments(min_replica_seg)`
- Replicas report position via heartbeat or state tracking

### 4. Read API
**Decision**: Expose `read_segment()` and `list_closed_segments()`
**Rationale**:
- Minimal changes to existing code
- Clear separation between writer and readers
- Replication server reads directly from files

---

## Open Questions & Answers

### Q1: Are entries globally ordered across segments?
**A**: ✅ YES - Segments are numbered sequentially (000001, 000002, ...) and entries within each segment are ordered by append order.

### Q2: What happens if replica requests deleted segment?
**A**: ⚠️ Error - Need to implement snapshot-based recovery or full state transfer. **MVP: Return error, replica must restart from segment 1**.

### Q3: Can we read active segment safely?
**A**: ❌ NO - Active segment is being written to. Only read closed segments.

### Q4: Is timestamp unique enough for ordering?
**A**: ⚠️ NO - Microsecond timestamps can collide under high write rates. Use (segment, offset) or add sequence numbers.

### Q5: How does recovery handle corrupted entries?
**A**: ✅ HANDLED - `read_record()` detects checksum mismatches, stops reading at corruption point (line 175).

---

## Summary & Recommendations

### What We Have
✅ Well-structured segmented WAL  
✅ Existing recovery code we can reuse  
✅ Clear binary format with checksums  
✅ Segment rotation and cleanup logic  

### What We Need
📝 Add sequence numbers (Phase 2, optional for MVP)  
📝 Expose segment reading APIs  
📝 Modify cleanup to respect replica positions  
📝 Implement replica position tracking  

### Replication Strategy

**Week 1 (MVP)**:
```
Replica State: { last_synced_segment: 5 }
Sync Protocol:
  1. Replica: "Give me segment 6"
  2. Leader: "Here are all entries from segment 6" (or "Segment 6 is still active, try later")
  3. Replica: Apply entries, update state to { last_synced_segment: 6 }
  4. Repeat every N seconds
```

**Week 2 (Enhanced)**:
- Add sequence numbers to entries
- Switch to: `{ last_synced_seq: 12345 }`
- Fine-grained replication

### Confidence Level
**HIGH** - The WAL code is clean, well-tested, and suitable for replication with minimal modifications.

### Blockers Identified
**NONE** - All issues have clear solutions:
- No sequence numbers → Use segment-based replication for MVP
- Cleanup might delete needed segments → Track replica positions
- No read API → Add public methods

---

## Next Steps (Day 3)

1. ✅ Create `src/replication/` module structure
2. ✅ Implement `ReplicaState` with segment tracking
3. ✅ Add `read_segment()` and related APIs to WAL
4. ✅ Write tests for segment reading
5. ✅ Update `cleanup_old_segments()` to accept min replica segment

**Ready to start coding!** 🚀
