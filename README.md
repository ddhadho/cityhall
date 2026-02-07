# CityHall 🏙️

---

## CityHall: Key Features

CityHall is a robust, crash-resilient time-series database designed for efficient data storage and retrieval.

### Core Features

**Crash-Resilient Durability**: Zero data loss on power failure with atomic state persistence.  
**High Throughput**: Optimized for high-volume data ingestion and retrieval.  
**Efficient Storage**: Utilizes advanced techniques for compact data storage.  

---

## Testing & Validation

### Integration Tests (All Passing)

```bash
# Run all tests
cargo test
```

**Zero compiler warnings. All tests pass.**

---

## Documentation

- **[BENCHMARKS.md](BENCHMARKS.md)** - Performance analysis with real measurements
- **[ARCHITECTURE.md](ARCHITECTURE.md)** - Storage engine internals

---

---

# The Storage Engine: CityHall Core

> **Note**: The following section describes the underlying storage engine that CityHall is built on top of. 

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


## License
This project is licensed under the MIT License.

---

<div align="center">


[![Benchmarks](https://img.shields.io/badge/benchmarks-measured-blue)](BENCHMARKS.md)
[![Language](https://img.shields.io/badge/language-Rust-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

</div>