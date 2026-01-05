//! Replication module for CityHall Sync
//!
//! Implements leader-replica replication using WAL segment streaming

pub mod protocol;
pub mod state;

pub use state::ReplicaState;
