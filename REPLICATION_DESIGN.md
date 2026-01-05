# CityHall Sync - Replication Design Document

**Version:** 1.0  
**Date:** January 2026  
**Project:** Rust Africa Hackathon 2026

---

## Table of Contents

1. [Overview](#overview)
2. [Goals & Non-Goals](#goals--non-goals)
3. [Architecture](#architecture)
4. [WAL-Based Replication Protocol](#wal-based-replication-protocol)
5. [Consistency Model](#consistency-model)
6. [Failure Handling](#failure-handling)
7. [Implementation Plan](#implementation-plan)
8. [Future Work](#future-work)

---

## Overview

**CityHall Sync** extends CityHall's local-first time-series database with leader-replica replication capabilities. This enables multi-site deployment where a single leader accepts writes while multiple replicas provide read scalability and durability through eventual consistency.

### Problem Statement

In distributed edge deployments (IoT sensors, multi-site monitoring, regional data collection), we need:

- **Durability**: Data must survive node failures
- **Availability**: Reads should work even if the leader is temporarily unavailable
- **Partition tolerance**: Nodes must handle network disconnections gracefully
- **Simplicity**: Avoid complex consensus protocols (Raft/Paxos) for MVP

### Solution Approach

Use **Write-Ahead Log (WAL) streaming** from a single leader to one or more replicas:

- Leader is the single source of truth for writes
- Replicas pull WAL entries periodically and replay them locally
- Eventual consistency: replicas lag but always converge
- No distributed consensus required (single-leader model)

---

## Goals & Non-Goals

### ✅ Goals (MVP - 2 Weeks)

1. **WAL-based replication**: Leader ships ordered WAL entries to replicas
2. **Replica catch-up**: Replicas track last applied sequence and sync missing entries
3. **Single-leader writes**: Only the leader accepts writes; replicas are read-only
4. **Eventual consistency**: Replicas may lag but eventually reflect all leader writes
5. **Failure recovery**: Handle node crashes, network partitions, and reconnections
6. **Observable**: CLI commands to check replication status and lag

### ❌ Non-Goals (Out of Scope)

- **Multi-leader writes**: No concurrent writes from multiple nodes
- **CRDTs**: No conflict-free replicated data types
- **Consensus protocols**: No Raft, Paxos, or similar
- **Automatic failover**: Manual leader promotion only (for MVP)
- **Sharding**: No horizontal partitioning of data
- **Read-your-writes consistency**: Replicas may serve stale data

---

## Architecture

### System Topology

```
┌─────────────────────────────────────────────────────────────┐
│                        LEADER NODE                          │
│  ┌────────────┐    ┌─────────┐    ┌──────────────────┐     │
│  │   Client   │───▶│ CityHall│───▶│  WAL (Append)    │     │
│  │  (Writes)  │    │  Engine │    │  Seq: 1,2,3,...  │     │
│  └────────────┘    └─────────┘    └──────────────────┘     │
│                                            │                 │
│                                            │                 │
│  ┌──────────────────────────────────────────────────────┐   │
│  │         Replication Server (Port 7879)               │   │
│  │  - Listens for replica sync requests                 │   │
│  │  - Sends WAL entries from requested sequence         │   │
│  └──────────────────────────────────────────────────────┘   │
└────────────────────────────┬────────────────────────────────┘
                             │
                             │ TCP (Poll-based sync)
                             │
        ┌────────────────────┼────────────────────┐
        │                    │                    │
        ▼                    ▼                    ▼
┌───────────────┐    ┌───────────────┐    ┌───────────────┐
│  REPLICA 1    │    │  REPLICA 2    │    │  REPLICA 3    │
│               │    │               │    │               │
│ ┌───────────┐ │    │ ┌───────────┐ │    │ ┌───────────┐ │
│ │Sync Agent │ │    │ │Sync Agent │ │    │ │Sync Agent │ │
│ │(Pull Loop)│ │    │ │(Pull Loop)│ │    │ │(Pull Loop)│ │
│ └─────┬─────┘ │    │ └─────┬─────┘ │    │ └─────┬─────┘ │
│       │       │    │       │       │    │       │       │
│       ▼       │    │       ▼       │    │       ▼       │
│ ┌───────────┐ │    │ ┌───────────┐ │    │ ┌───────────┐ │
│ │Local WAL  │ │    │ │Local WAL  │ │    │ │Local WAL  │ │
│ │(Replayed) │ │    │ │(Replayed) │ │    │ │(Replayed) │ │
│ └───────────┘ │    │ └───────────┘ │    │ └───────────┘ │
│               │    │               │    │               │
│ ┌───────────┐ │    │ ┌───────────┐ │    │ ┌───────────┐ │
│ │ CityHall  │ │    │ │ CityHall  │ │    │ │ CityHall  │ │
│ │ (Read-    │ │    │ │ (Read-    │ │    │ │ (Read-    │ │
│ │  Only)    │ │    │ │  Only)    │ │    │ │  Only)    │ │
│ └───────────┘ │    │ └───────────┘ │    │ └───────────┘ │
└───────────────┘    └───────────────┘    └───────────────┘
```

### Component Overview

#### 1. **Leader (Write Node)**
- **Responsibilities**:
  - Accept all writes from clients (port 7878)
  - Append entries to local WAL with monotonic sequence numbers
  - Run replication server (port 7879) to serve WAL entries to replicas
  - Continue normal CityHall operations (MemTable flush, compaction)

- **New Components**:
  - `ReplicationServer`: TCP listener for replica sync requests
  - `WALReader`: Read WAL entries from a given sequence number

#### 2. **Replica (Read Node)**
- **Responsibilities**:
  - Pull WAL entries from leader periodically
  - Apply entries to local WAL in order
  - Replay entries to local MemTable/SSTables
  - Serve read requests (with eventual consistency lag)
  - Track `last_applied_seq` persistently

- **New Components**:
  - `ReplicationAgent`: Background task that polls leader
  - `ReplicaState`: Persistent state tracking last applied sequence
  - `WALApplier`: Apply remote WAL entries to local storage

---

## WAL-Based Replication Protocol

### WAL Entry Format (Existing in CityHall)

Based on CityHall's architecture, WAL entries likely have this structure:

```rust
pub struct WALEntry {
    pub sequence: u64,        // Monotonically increasing
    pub timestamp: u64,       // Unix timestamp (microseconds)
    pub key: Vec<u8>,         // Key bytes
    pub value: Option<Vec<u8>>, // Value bytes (None for tombstone)
    pub checksum: u32,        // CRC32 for integrity
}
```

### Network Protocol (Binary over TCP)

We use a simple request-response protocol with length-prefixed messages:

#### Message Types

```rust
pub enum ReplicationMessage {
    // Replica → Leader: Request entries after a sequence number
    SyncRequest {
        replica_id: String,
        last_applied_seq: u64,
    },
    
    // Leader → Replica: Response with entries
    SyncResponse {
        entries: Vec<WALEntry>,
        leader_seq: u64,  // Current leader's highest sequence
    },
    
    // Leader → Replica: No new data available
    NoNewData {
        leader_seq: u64,
    },
    
    // Error response
    Error {
        message: String,
    },
}
```

#### Wire Format (using bincode)

```
┌────────────────────────────────────────┐
│  Message Length (4 bytes, big-endian)  │
├────────────────────────────────────────┤
│  Message Type (1 byte)                 │
│    0x01 = SyncRequest                  │
│    0x02 = SyncResponse                 │
│    0x03 = NoNewData                    │
│    0xFF = Error                        │
├────────────────────────────────────────┤
│  Payload (bincode-serialized)          │
│  - For SyncRequest: replica_id, seq    │
│  - For SyncResponse: Vec<WALEntry>     │
│  - For NoNewData: leader_seq           │
│  - For Error: error_message            │
└────────────────────────────────────────┘
```

### Sync Flow (Normal Operation)

```
Replica                                    Leader
   │                                          │
   │  1. Read local replica_state.json        │
   │     last_applied_seq = 1000              │
   │                                          │
   │  2. SyncRequest                          │
   │     { replica_id: "r1", seq: 1000 }      │
   ├─────────────────────────────────────────▶│
   │                                          │
   │                                          │  3. Query WAL
   │                                          │     for entries > 1000
   │                                          │
   │  4. SyncResponse                         │
   │     { entries: [1001, 1002, ..., 1050],  │
   │       leader_seq: 1050 }                 │
   │◀─────────────────────────────────────────┤
   │                                          │
   │  5. Apply entries locally:               │
   │     - Write to local WAL                 │
   │     - Apply to MemTable                  │
   │     - Update last_applied_seq = 1050     │
   │                                          │
   │  6. Sleep for sync_interval (5s)         │
   │                                          │
   │  7. Repeat from step 2                   │
   └──────────────────────────────────────────┘
```

### Sync Flow (Replica Lagging Behind)

If a replica has been offline and needs to catch up:

```
Replica (last_applied_seq = 1000)
   │
   │  Request: seq = 1000
   ├──────────────────────▶ Leader
   │  Response: entries [1001..2000], leader_seq = 5000
   │◀──────────────────────
   │  Apply batch 1
   │
   │  Request: seq = 2000
   ├──────────────────────▶ Leader
   │  Response: entries [2001..3000], leader_seq = 5000
   │◀──────────────────────
   │  Apply batch 2
   │
   │  ... (continue until caught up)
   │
   │  Request: seq = 5000
   ├──────────────────────▶ Leader
   │  Response: NoNewData, leader_seq = 5000
   │◀──────────────────────
   │  Fully caught up!
   └────────────────────────
```

### Batching Strategy

To avoid overwhelming the replica or network:

- **Leader limits**: Send at most 1000 entries per response (configurable)
- **Replica pacing**: Process entries in batches, checkpoint after each batch
- **Backpressure**: If replica is too far behind, it should increase sync frequency

---

## Consistency Model

### Guarantees

1. **Sequential Consistency on Leader**
   - All writes go through the leader's WAL in a total order
   - WAL sequence numbers define the global order
   - Every entry has a unique, monotonically increasing sequence number

2. **Eventual Consistency on Replicas**
   - Replicas lag behind the leader by some time delta (replication lag)
   - Given enough time and stable network, all replicas converge to leader's state
   - Replicas apply entries in the same order as the leader (deterministic replay)

3. **Read-Your-Writes NOT Guaranteed**
   - A client writing to the leader may not immediately see that write on a replica
   - Clients must query the leader if they need strong consistency

4. **Monotonic Reads NOT Guaranteed**
   - Reading from different replicas may return different values for the same key
   - Reading from the same replica guarantees monotonic reads (within that replica)

### Replication Lag

Replication lag is the time between:
- **T_write**: Entry written to leader's WAL (sequence N)
- **T_apply**: Entry applied to replica (sequence N)

**Lag = T_apply - T_write**

Factors affecting lag:
- Network latency between leader and replica
- Sync interval (how often replica polls)
- Batch size (larger batches = fewer round trips but higher latency)
- Leader write rate (if too high, replicas fall behind)

### Handling Stale Reads

Clients reading from replicas must be aware they may get stale data:

```bash
# Write to leader
cityhall client put --addr leader:7878 metric.cpu "75.3"

# Immediately read from replica (may not see the write yet)
cityhall client get --addr replica:7880 metric.cpu
# Output: NOT_FOUND (or old value)

# Wait for replication lag...
sleep 2

# Read again
cityhall client get --addr replica:7880 metric.cpu
# Output: VALUE 75.3
```

For applications requiring strong consistency:
- Always read from the leader
- Or implement "read-after-write" by tracking client's last write sequence

---

## Failure Handling

### 1. Replica Failure (Crash or Network Partition)

**Scenario**: Replica goes offline while leader continues accepting writes.

**Recovery**:
1. Replica restarts and loads `last_applied_seq` from persistent state
2. Replica reconnects to leader and requests entries from `last_applied_seq + 1`
3. Leader sends all missing entries (may be thousands if downtime was long)
4. Replica applies entries in batches until caught up

**Implementation**:
- Replica state must be persisted before marking entries as applied
- Use write-ahead pattern: update state, fsync, then ACK
- Leader must retain old WAL segments long enough for replicas to catch up

### 2. Leader Failure (Crash)

**Scenario**: Leader crashes mid-operation.

**Recovery (Manual Failover - MVP)**:
1. Administrator detects leader is down
2. Selects most up-to-date replica (highest `last_applied_seq`)
3. Promotes replica to leader:
   - Enable write mode
   - Start replication server
   - Update DNS or load balancer to point to new leader
4. Other replicas reconfigure to pull from new leader

**Limitations (MVP)**:
- Manual process (no automatic failover)
- Potential data loss: uncommitted entries in old leader's WAL may be lost
- No split-brain prevention (must ensure old leader is truly dead)

**Future Work**:
- Implement leader election (Raft-style)
- Use consensus to ensure safety during failover
- Automatic split-brain detection

### 3. Network Partition

**Scenario**: Network partition separates leader from replicas.

**Behavior**:
- Leader continues accepting writes (unaware of partition)
- Replicas cannot reach leader, sync requests fail
- Replicas serve stale reads (based on last synced data)

**Recovery**:
- When partition heals, replicas reconnect and catch up normally
- No data loss (leader's WAL is intact)
- Reads may have served stale data during partition (expected in eventual consistency)

**Implementation**:
- Replicas use exponential backoff when sync requests fail
- Replicas log warnings if lag exceeds threshold (e.g., 60 seconds)
- Optional: Replicas can stop serving reads if lag is too high (fail-stop mode)

### 4. Slow Replica (Persistent Lag)

**Scenario**: Replica cannot keep up with leader's write rate.

**Detection**:
- Monitor replication lag metric: `leader_seq - last_applied_seq`
- Alert if lag exceeds threshold (e.g., 10,000 entries or 5 minutes)

**Mitigation**:
- Increase sync frequency (reduce sleep interval)
- Increase batch size to reduce round-trip overhead
- Optimize replica's apply path (faster WAL writes, MemTable ops)
- If persistent, consider removing slow replica from pool

### 5. WAL Segment Deletion (Leader Purges Old Logs)

**Scenario**: Leader compacts and deletes old WAL segments, but replica still needs them.

**Prevention**:
- Leader tracks replicas' `last_applied_seq` (via heartbeat or registration)
- Leader retains WAL segments until all replicas have applied them
- Configuration: `wal_retention_hours` or `wal_retention_segments`

**Fallback (if WAL is deleted)**:
- Replica requests entries from sequence X, but leader's oldest entry is Y > X
- Leader responds with `ERROR: SnapshotRequired`
- Replica must perform full SSTable sync (copy all SSTables from leader)
- Future work: implement snapshot-based recovery

---

## Implementation Plan

### Phase 1: Foundation (Days 1-2)

**Goal**: Understand CityHall's WAL and design the replication protocol.

**Tasks**:
1. Analyze CityHall's existing WAL implementation:
   - Locate WAL code (likely `src/wal.rs` or `src/storage/wal.rs`)
   - Understand entry format, sequence numbering, and file layout
   - Identify how to read entries from a given sequence number

2. Design replication protocol:
   - Define message types and wire format
   - Choose serialization (bincode for efficiency)
   - Design replica state persistence format

3. Create architecture diagrams:
   - System topology (leader + replicas)
   - Sequence diagrams for sync flow
   - State machine diagram for replica lifecycle

**Deliverables**:
- This document (REPLICATION_DESIGN.md)
- Architecture diagrams (Mermaid or draw.io)
- Updated README with replication overview

### Phase 2: Replica State & Tracking (Days 3-4)

**Goal**: Implement persistent replica state tracking.

**Implementation**:
```rust
// src/replication/state.rs

#[derive(Serialize, Deserialize, Clone)]
pub struct ReplicaState {
    pub replica_id: String,
    pub leader_addr: String,
    pub last_applied_seq: u64,
    pub last_sync_time: SystemTime,
}

impl ReplicaState {
    pub fn new(replica_id: String, leader_addr: String) -> Self {
        Self {
            replica_id,
            leader_addr,
            last_applied_seq: 0,
            last_sync_time: SystemTime::now(),
        }
    }
    
    pub fn load(path: &Path) -> Result<Self> { /* ... */ }
    pub fn save(&self, path: &Path) -> Result<()> { /* ... */ }
    pub fn update_seq(&mut self, seq: u64) { /* ... */ }
}
```

**Tests**:
- State persistence across restarts
- Concurrent read/write safety
- Corrupted state file handling

### Phase 3: Network Protocol (Days 5-7)

**Goal**: Implement leader-replica communication.

**Leader Side**:
```rust
// src/replication/leader.rs

pub struct ReplicationServer {
    wal: Arc<WAL>,
    listener: TcpListener,
}

impl ReplicationServer {
    pub async fn serve(&self) -> Result<()> {
        loop {
            let (stream, addr) = self.listener.accept().await?;
            let wal = Arc::clone(&self.wal);
            tokio::spawn(async move {
                handle_replica(stream, wal).await
            });
        }
    }
}

async fn handle_replica(mut stream: TcpStream, wal: Arc<WAL>) -> Result<()> {
    // 1. Read SyncRequest
    // 2. Query WAL for entries after requested sequence
    // 3. Send SyncResponse or NoNewData
}
```

**Replica Side**:
```rust
// src/replication/replica.rs

pub struct ReplicationAgent {
    leader_addr: SocketAddr,
    state: ReplicaState,
    local_wal: Arc<WAL>,
    sync_interval: Duration,
}

impl ReplicationAgent {
    pub async fn run(&mut self) -> Result<()> {
        loop {
            if let Err(e) = self.sync_once().await {
                error!("Sync failed: {}", e);
                sleep(backoff_duration()).await;
            } else {
                sleep(self.sync_interval).await;
            }
        }
    }
    
    async fn sync_once(&mut self) -> Result<()> {
        // 1. Connect to leader
        // 2. Send SyncRequest with last_applied_seq
        // 3. Receive SyncResponse
        // 4. Apply entries to local WAL
        // 5. Update and persist state
    }
}
```

**Tests**:
- Basic sync (leader sends entries, replica applies)
- Empty sync (no new data)
- Large batch sync (1000+ entries)
- Network error handling

### Phase 4: Integration & Demo (Days 8-14)

See main roadmap for details.

---

## Future Work

### Automatic Failover
- Implement leader election (Raft consensus)
- Health checks and failure detection
- Automatic replica promotion

### Performance Optimizations
- Streaming replication (push instead of pull)
- Compression of WAL entries over network
- Parallel apply on replica (out-of-order for independent keys)
- Delta encoding for repeated keys

### Operational Features
- Replication metrics dashboard
- Lag alerting and monitoring
- Replica registration/discovery (Consul, etcd)
- Backup and restore from snapshots

### Advanced Consistency
- Read-your-writes guarantees (client session tracking)
- Causal consistency (vector clocks)
- Configurable consistency levels (strong, bounded-staleness, eventual)

---

## Appendix: Key Assumptions

1. **CityHall's WAL is append-only** with monotonic sequence numbers
2. **WAL entries are self-contained** (no dependencies between entries)
3. **Network is eventually reliable** (partitions are temporary)
4. **Leader's WAL is the source of truth** (no multi-leader conflicts)
5. **Time-series workload**: Mostly appends, rare updates/deletes
6. **Replicas can lag** without violating correctness (eventual consistency)
7. **Manual operations** are acceptable for failure recovery in MVP

---

## References

- CityHall Architecture: [ARCHITECTURE.md](./ARCHITECTURE.md)
- CityHall README: [README.md](./README.md)
- Rust Async Book: https://rust-lang.github.io/async-book/
- LSM-Tree Replication Patterns: Cassandra, RocksDB documentation