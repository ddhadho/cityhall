# CityHall 🏙️

A personal, local-first, time-series database daemon for your digital life, built from scratch in Rust.

[![Build Status](https://img.shields.io/badge/build-passing-brightgreen)](https://github.com/ddhadho/f42VQ)
[![Tests](https://img.shields.io/badge/tests-47%20passing-brightgreen)](https://github.com/ddhadho/f42VQ)
[![Language](https://img.shields.io/badge/language-Rust-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

---

## The Philosophy

**CityHall is the administrative core for your personal machine.** Just as a city hall manages the official records and services of a city, CityHall is a foundational service that manages the data of your "digital city"—your personal computer.

This project is a deep dive into the art of systems engineering, built as a portfolio piece to demonstrate a mastery of the principles behind modern storage engines. It's an exploration of trade-offs, performance optimization, and robust, production-grade patterns.

The engine is built on a **Log-Structured Merge-Tree (LSM-Tree)**, the same architecture that powers databases like RocksDB, Cassandra, and InfluxDB.

## Core Features

-   **Blazing Fast Writes**: A batched, checksummed Write-Ahead Log (WAL) ensures durability while an in-memory `BTreeMap`-based MemTable ingests writes at high speed.
-   **Non-Blocking Operations**: A dual-MemTable architecture with a background thread for flushing to disk ensures that writes remain fast and responsive, avoiding latency spikes.
-   **Optimized Read Path**: Data is persisted to immutable, compressed SSTables (Sorted String Tables). Queries for non-existent keys are rejected in microseconds thanks to custom-built Bloom Filters.
-   **Automatic Housekeeping**: A background compaction process merges SSTables using a k-way merge algorithm, reclaiming space and improving read efficiency.
-   **Command-Line Interface**: A simple and intuitive CLI (`cityhall`) for interacting with the database.

## Performance Highlights

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

#### Write Path
```
Client → WAL (Batched) → Active MemTable (In-Memory BTreeMap)
                                ↓ (Threshold reached, e.g., 64MB)
                         Immutable MemTable (Frozen)
                                ↓ (Handed to Background Thread)
                         SSTable on Disk (Compressed, with Bloom Filter)
```

#### Read Path
```
Client → Active MemTable → Immutable MemTable → SSTables (Newest to Oldest)
                                                      ↳ Bloom Filter check (to skip I/O)
```

## Getting Started

### Building
The project is built with the standard Rust toolchain.

```bash
# Clone the repository
git clone https://github.com/ddhadho/f42VQ.git
cd f42VQ

# Build the binary in release mode
cargo build --release
```
The CLI binary will be available at `./target/release/cityhall`.

### Usage
The `cityhall` CLI is the primary way to interact with the database. By default, it stores data in `~/.cityhall/data`.

```bash
# Put a key-value pair
./target/release/cityhall put system.cpu.load "0.75"

# Get a value by its key
./target/release/cityhall get system.cpu.load
# Output: Found key: "system.cpu.load", value: "0.75"

# Get a key that doesn't exist
./target/release/cityhall get non.existent.key
# Output: Key "non.existent.key" not found.
```

### Running Tests
The project includes a comprehensive test suite.
```bash
cargo test
```

## Project Roadmap

CityHall is an active project. The next major areas of focus include:

-   [ ] **Metrics & Observability**: Implementing a full metrics system to track every operation and performance characteristic in real-time.
-   [ ] **Delete Support**: Adding support for tombstones to properly handle key deletions during reads and compaction.
-   [ ] **Block Cache**: Introducing an in-memory LRU cache for SSTable data blocks to reduce disk I/O for hot data.
-   [ ] **Network API**: Evolving from a CLI to a true daemon with a network interface (e.g., HTTP or gRPC) to allow other applications to use it as a service.

## License
This project is licensed under the MIT License.