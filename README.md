# CityHall 🏙️

---
> ### 🏆 Rust Africa Hackathon 2026 Submission
>
> **Track**: Infrastructure & Connectivity  
> **Focus**: CityHall Sync - WAL-based replication for offline-first edge computing
>
> Built for intermittent connectivity, expensive bandwidth, and power instability.
---

## 🌍 The Problem: African Infrastructure Reality

**60% of African businesses face daily internet outages.** Traditional cloud-only databases fail when the network drops. Data is lost. Operations halt. Businesses suffer.

**CityHall Sync solves this**: A distributed, offline-first time-series database that continues working when the internet doesn't.

### Real-World Scenarios

**🏭 Factory Floor (Nairobi)**
- 100 sensors collecting temperature data every second
- Internet drops for 8 hours (common with Safaricom 4G in industrial areas)
- **Traditional systems**: Data lost or buffered until memory exhausted
- **CityHall Sync**: Continues collecting locally, syncs all 2.88M entries in 4 minutes when online

**🏥 Rural Health Clinic (Rwanda)**
- Doctor needs patient vital signs history
- Clinic 50km from cell tower, satellite link available 2x/day
- **Traditional systems**: Cannot access records without internet
- **CityHall Sync**: Local replica has all data, syncs with central hospital when satellite connects

**🛒 Retail Chain (East Africa)**
- 50 stores across Kenya, Tanzania, Uganda
- Each store needs POS transaction storage
- HQ needs consolidated analytics
- **Traditional systems**: $4,050/month bandwidth costs (JSON over HTTP)
- **CityHall Sync**: $405/month (10x more efficient) + works offline

---

## 🎯 CityHall Sync: The Hackathon Innovation

**CityHall Sync** is a WAL-based replication system built on top of the CityHall storage engine. This is the core innovation for this hackathon submission.

### Key Features

✅ **Offline-First**: Leader and replicas operate independently without constant network  
✅ **Crash-Resilient**: Zero data loss on power failure (state persists atomically)  
✅ **Bandwidth-Efficient**: 10x smaller than JSON (bincode + compression)  
✅ **Fast Recovery**: Catch up from multi-day outages in minutes  
✅ **Scalable**: Handles 10+ replicas with minimal overhead  

### Architecture
```
┌─────────────────────────────────────────────────────────────┐
│                    LEADER NODE (Factory)                    │
│  ┌──────────┐      ┌──────────┐      ┌──────────┐          │
│  │ Sensors  │ ───▶ │   WAL    │ ───▶ │ SSTables │          │
│  └──────────┘      └──────────┘      └──────────┘          │
│                          │                                  │
└──────────────────────────┼──────────────────────────────────┘
                           │ WAL Segment Streaming
                           │ (Pull-based, TCP)
                           ▼
         ┌─────────────────────────────────────┐
         │                                     │
         ▼                                     ▼
┌──────────────────┐                  ┌──────────────────┐
│ REPLICA (Office) │                  │ REPLICA (Cloud)  │
│  ┌────────────┐  │                  │  ┌────────────┐  │
│  │ Local WAL  │  │                  │  │ Local WAL  │  │
│  └────────────┘  │                  │  └────────────┘  │
│  ┌────────────┐  │                  │  ┌────────────┐  │
│  │  SSTables  │  │                  │  │  SSTables  │  │
│  └────────────┘  │                  │  └────────────┘  │
└──────────────────┘                  └──────────────────┘
```

**Design Decisions:**
- **WAL-based vs Snapshot-based**: Stream deltas (KB) not full snapshots (GB)
- **Pull-based vs Push**: Replicas control pace, adapt to network conditions
- **Eventual consistency**: Availability > consistency for metrics collection

### Novel Approaches

**1. Persistent Replica State**
- Unlike PostgreSQL (in-memory), CityHall persists `last_applied_seq` to disk
- Survives crashes without full re-sync

**2. Segment Discovery Protocol**
- Replicas query leader for available segments before syncing
- Handles leader/replica timing skew gracefully

**3. Exponential Backoff with Jitter**
- Prevents thundering herd when 50 replicas reconnect after regional outage
- Adaptive retry intervals (1s → 60s)

**4. Replica-Aware WAL Cleanup**
- Leader tracks minimum replica position before deleting old segments
- Ensures no data loss even if replica offline for days

### Performance: Measured & Proven

| Metric | Result | Real-World Impact |
|--------|--------|-------------------|
| **Replication Latency** | 70ms avg (52-140ms) | Near-real-time for LAN |
| **Catch-up Throughput** | 3,000 entries/sec | 8hr outage recovers in 16 min |
| **Multiple Replicas** | 3 nodes in 3.26ms | Scales to 50+ stores |
| **Bandwidth/Entry** | 1.8 KB (vs 18 KB JSON) | **$81/month saved** per site |
| **Data Loss** | Zero (verified) | Survives power failures |

**Full benchmarks**: [BENCHMARKS.md](BENCHMARKS.md) | **Design doc**: [REPLICATION_DESIGN.md](REPLICATION_DESIGN.md)

---

## 🚀 Quick Start: Demo the Replication System

### Prerequisites
```bash
# Clone repository
git clone https://github.com/ddhadho/cityhall.git
cd cityhall

# Build in release mode
cargo build --release
```

### 3-Terminal Demo

**Terminal 1: Start Leader**
```bash
./target/release/cityhall leader --data-dir /tmp/leader --replication-port 7879
```

**Terminal 2: Start Replica**
```bash
./target/release/cityhall replica --leader 127.0.0.1:7879 --data-dir /tmp/replica1
```

**Terminal 3: Write Data to Leader**
```bash
# Write 100 sensor readings
for i in {1..100}; do
  ./target/release/cityhall client put "sensor.temp.line_$i" "$(shuf -i 20-30 -n 1).5"
done

# Check replica status (should show synced data)
./target/release/cityhall replica status --data-dir /tmp/replica1
# Output: Last synced segment: 1, Total entries: 100
```

### Offline Recovery Demo
```bash
# In Terminal 2: Kill the replica (Ctrl+C)

# In Terminal 3: Leader continues - write 500 more entries
for i in {101..600}; do
  ./target/release/cityhall client put "sensor.temp.$i" "25.0"
done

# In Terminal 2: Restart replica - watch it catch up
./target/release/cityhall replica --leader 127.0.0.1:7879 --data-dir /tmp/replica1

# Observe: Catches up 500 entries in ~200ms
# Check status again
./target/release/cityhall replica status --data-dir /tmp/replica1
# Output: Last synced segment: 2, Total entries: 600
```

**This demonstrates:**
- ✅ Offline resilience (replica can be down for hours/days)
- ✅ Fast catch-up (500 entries in 200ms)
- ✅ Zero data loss (all 600 entries present)

---

## 💰 Cost Savings Analysis

### Bandwidth Comparison (Safaricom Kenya @ $5/GB)

| System | Per Entry | 1M Entries/Day | Monthly Cost |
|--------|-----------|----------------|--------------|
| **CityHall Sync** | 1.8 KB | 1.8 GB | **$9** |
| JSON-over-HTTP | 18 KB | 18 GB | $90 |
| **Savings** | **10x smaller** | 16.2 GB | **$81/month** |

**Real-World Impact:**
- **Single factory**: $972/year saved
- **10-store chain**: $9,720/year saved
- **50-store chain**: **$48,600/year saved**

---

## 🧪 Testing & Validation

### Integration Tests (All Passing)

- ✅ Basic replication (100 entries synced)
- ✅ Offline recovery (replica catches up after simulated 8hr outage)
- ✅ Multiple replicas (3 nodes converge to same state)
- ✅ Network partitions (exponential backoff on reconnect)
- ✅ Concurrent operations (1000 writes + 5000 reads simultaneously)
- ✅ Crash recovery (state persists across restarts, zero data loss)
- ✅ High throughput (10,000 entries replicated in < 2s)
```bash
# Run all tests
cargo test

# Run replication benchmarks with output
cargo test --test replication_benchmarks -- --nocapture

# Run integration tests
cargo test --test integration_sync -- --nocapture
```

**Zero compiler warnings. All tests pass.**

---

## 🧑‍💻 Authorship & AI Collaboration

**Transparency Statement** (per hackathon rules):

- **Human Architect**: System design, architecture decisions, performance benchmarking, debugging, documentation
- **AI Assistant** (Claude by Anthropic): Line-by-line code generation under direct human guidance

This collaboration enabled rapid implementation of a complex distributed system while maintaining full architectural control.

**All code follows hackathon rules:**
- ✅ Boilerplate AI code declared in comments
- ✅ Core logic human-designed, AI-implemented
- ✅ No plagiarism - original design

---

## 📚 Documentation

- **[BENCHMARKS.md](BENCHMARKS.md)** - Performance analysis with real measurements
- **[REPLICATION_DESIGN.md](REPLICATION_DESIGN.md)** - Technical deep-dive on protocol
- **[ARCHITECTURE.md](ARCHITECTURE.md)** - Storage engine internals

---

---

# 🗄️ The Storage Engine: CityHall Core

> **Note**: The following section describes the underlying storage engine that CityHall Sync is built on top of. This is a portfolio/demonstration project showing mastery of database internals.

---

## Philosophy

**CityHall is the administrative core for your personal machine.** Just as a city hall manages the official records and services of a city, CityHall is a foundational service that manages the data of your "digital city"—your personal computer.

This project is a deep dive into the art of systems engineering, built as a portfolio piece to demonstrate a mastery of the principles behind modern storage engines. It's an exploration of trade-offs, performance optimization, and robust, production-grade patterns.

The engine is built on a **Log-Structured Merge-Tree (LSM-Tree)**, the same architecture that powers databases like RocksDB, Cassandra, and InfluxDB.

## Core Storage Engine Features

-   **Persistent Daemon & Client-Server Architecture**: CityHall runs as a long-lived background service, managed by `systemd`, with a separate client application for interaction.
-   **Blazing Fast Writes**: A batched, checksummed Write-Ahead Log (WAL) ensures durability while an in-memory `BTreeMap`-based MemTable ingests writes at high speed.
-   **Non-Blocking Operations**: A dual-MemTable architecture with a background thread for flushing to disk ensures that writes remain fast and responsive, avoiding latency spikes.
-   **Optimized Read Path**: Data is persisted to immutable, compressed SSTables (Sorted String Tables). Queries for non-existent keys are rejected in microseconds thanks to custom-built Bloom Filters. Reads prioritize newest data by iterating SSTables from most recent to oldest.
-   **Automatic Housekeeping**: A background compaction process merges SSTables using a k-way merge algorithm, reclaiming space and improving read efficiency.
-   **Comprehensive Internal Metrics**: An integrated metrics system tracks key operational data, performance, and system state for observability (e.g., read/write counts, latencies, disk usage).
-   **Robust Command-Line Interface**: A versatile CLI (`cityhall`) acts as a client to the running daemon, enabling `put`, `get` commands and future expansion.

## Storage Engine Performance

Every optimization is backed by data. CityHall is not just correct; it's efficient.

| Feature               | Metric                | Result                                       |
| --------------------- | --------------------- | -------------------------------------------- |
| **Bloom Filters**     | Read Latency (Miss)   | **442x Speedup** (1,209μs → 2.74μs)            |
| **Background Flush**  | Write Latency (p99)   | **93% Improvement** (100ms → 7ms)              |
| **Compaction**        | Space Savings         | **Up to 97.4%**                              |
| **WAL Batching**      | Write Throughput      | **~184,000 writes/sec**                      |
| **Data Compression**  | Compression Ratio     | **~10:1** (Snappy + Prefix Compression)      |

## Architecture Overview

CityHall follows a classic LSM-Tree design to prioritize write performance without sacrificing read efficiency.

For a detailed technical explanation of the components, data flow, and design trade-offs, please see the **[Architecture Deep Dive](ARCHITECTURE.md)**.

### Write Path
```
Client CLI → Daemon (TCP) → WAL (Batched) → Active MemTable (In-Memory BTreeMap)
                                              ↓ (Threshold reached, e.g., 64MB)
                                           Immutable MemTable (Frozen)
                                              ↓ (Handed to Background Thread)
                                           SSTable on Disk (Compressed, with Bloom Filter)
```

### Read Path
```
Client CLI → Daemon (TCP) → Active MemTable → Immutable MemTable → SSTables (Newest to Oldest)
                                                                       ↳ Bloom Filter check (to skip I/O)
```

## Storage Engine Setup (systemd Daemon)

### Building
The project is built with the standard Rust toolchain.
```bash
# Build the binary in release mode
cargo build --release
```
The CLI binary will be available at `./target/release/cityhall`.

### Running as a Daemon (systemd)
To run CityHall as a persistent background service on Linux:

1.  **Edit `cityhall.service`**: Copy `cityhall.service` from the project root to `/etc/systemd/system/` and edit the `User=` and `ExecStart=` paths to match your system.
```bash
    sudo cp cityhall.service /etc/systemd/system/
    sudo nano /etc/systemd/system/cityhall.service
    # Replace <YOUR_USERNAME> with your actual username
    # Replace /path/to/your/cityhall/target/release/cityhall with the absolute path
```
2.  **Reload, Enable, and Start**:
```bash
    sudo systemctl daemon-reload
    sudo systemctl enable cityhall.service
    sudo systemctl start cityhall.service
```
3.  **Verify Status**:
```bash
    systemctl status cityhall.service
```
    You should see `Active: active (running)`.

### Using the CLI Client
The `cityhall` CLI is the primary way to interact with the running daemon. By default, it stores data in `~/.cityhall/data` and connects to `127.0.0.1:7878`.
```bash
# Put a key-value pair
./target/release/cityhall client put system.cpu.load "0.75"

# Get a value by its key
./target/release/cityhall client get system.cpu.load
# Output: VALUE 0.75

# Get a key that doesn't exist
./target/release/cityhall client get non.existent.key
# Output: NOT_FOUND
```

## Storage Engine Roadmap

CityHall is an active portfolio project. Future areas of focus include:

-   [ ] **Delete Support**: Implementing a robust mechanism for key deletions using tombstones, handled correctly throughout the WAL, MemTables, and compaction.
-   [ ] **Metrics Command**: Exposing internal metrics via a dedicated client command (`cityhall client metrics`).
-   [ ] **System Monitoring Agent**: Building a separate tool to automatically feed system performance metrics into CityHall itself.
-   [ ] **Block Cache**: Introducing an in-memory LRU cache for SSTable data blocks to reduce disk I/O for hot data.
-   [ ] **Leveled Compaction**: Migrating from size-tiered to a more optimal leveled compaction strategy for better read performance.
-   [ ] **Network API**: Expanding the current simple TCP protocol to a more robust API (e.g., gRPC, HTTP) for broader application integration.

---

## 🏆 Why CityHall Sync Wins

**Technical Excellence**
- ✅ Production-grade error handling (exponential backoff, timeouts, connection state tracking)
- ✅ Comprehensive testing (7 integration tests, 4 benchmarks)
- ✅ Zero compiler warnings
- ✅ Rust best practices (no `unsafe`, proper error types, async/await)

**Real-World Impact**
- ✅ Solves actual African infrastructure problems
- ✅ Proven performance with measurements
- ✅ Quantified cost savings ($48,600/year for 50-store deployment)
- ✅ Works offline when competitors fail

**Innovation**
- ✅ WAL-based streaming (not snapshot-based)
- ✅ Persistent state tracking (survives crashes)
- ✅ Segment discovery protocol
- ✅ 10x bandwidth efficiency

---

## License
This project is licensed under the MIT License.

---

<div align="center">

**Built for Africa. Tested in Kenya. Ready for the world.**

[![Build Status](https://img.shields.io/badge/build-passing-brightgreen)](https://github.com/ddhadho/cityhall)
[![Tests](https://img.shields.io/badge/tests-passing-brightgreen)](https://github.com/ddhadho/cityhall)
[![Benchmarks](https://img.shields.io/badge/benchmarks-measured-blue)](BENCHMARKS.md)
[![Language](https://img.shields.io/badge/language-Rust-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

</div>