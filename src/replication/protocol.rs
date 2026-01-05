//! Replication protocol message types
//!
//! Wire protocol for leader-replica communication

use serde::{Deserialize, Serialize};

/// Request from replica to leader
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncRequest {
    /// Request entries from a specific segment
    GetSegment { segment_number: u64 },

    /// Get list of available segments
    ListSegments,
}

/// Response from leader to replica
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncResponse {
    /// Segment data with entries
    SegmentData {
        segment_number: u64,
        entries: Vec<crate::Entry>,
    },

    /// List of available segment numbers
    SegmentList {
        segments: Vec<u64>,
        current_segment: u64,
    },

    /// Segment not found (deleted or doesn't exist)
    SegmentNotFound { segment_number: u64 },

    /// Error occurred
    Error { message: String },
}

// TODO: Add wire format implementation (Day 5)
