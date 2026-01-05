pub mod compaction;
pub mod error;
pub mod memtable;
pub mod metrics;
pub mod replication;
pub mod sstable;
pub mod storage_engine;
pub mod wal;

pub use compaction::{compact_sstables, select_sstables_for_compaction, CompactionStats};
pub use error::{Result, StorageError};
pub use memtable::MemTable;
pub use sstable::{SsTableReader, SsTableWriter};
pub use storage_engine::StorageEngine;
pub use wal::Wal;

use serde::{Deserialize, Serialize};

// Core types that everything uses
pub type Key = Vec<u8>;
pub type Value = Vec<u8>;
pub type Timestamp = u64; // Unix timestamp in seconds

/// Entry in the storage system
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub key: Key,
    pub value: Value,
    pub timestamp: Timestamp,
}

impl Entry {
    pub fn new(key: Key, value: Value, timestamp: Timestamp) -> Self {
        Entry {
            key,
            value,
            timestamp,
        }
    }
}

/// Operation types for WAL
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OpType {
    Put = 1,
    Delete = 2,
}

impl OpType {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(OpType::Put),
            2 => Some(OpType::Delete),
            _ => None,
        }
    }
}
