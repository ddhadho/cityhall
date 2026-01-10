//! Command-line interface definitions
//!
//! Defines all CLI commands and arguments using clap

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// CityHall - Time-series database with replication
#[derive(Parser, Debug)]
#[command(name = "cityhall")]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

/// Available commands
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start CityHall in leader mode (accepts writes, serves replicas)
    Leader {
        /// Data directory for WAL and state
        #[arg(long, default_value = "~/.cityhall/leader")]
        data_dir: PathBuf,
        
        /// Port for client connections
        #[arg(long, short = 'p', default_value = "7878")]
        port: u16,
        
        /// Port for replication server
        #[arg(long, short = 'r', default_value = "7879")]
        replication_port: u16,
        
        /// WAL buffer size in bytes
        #[arg(long, default_value = "1048576")]
        wal_buffer_size: usize,
        
        /// Configuration file (TOML or JSON)
        #[arg(long, short = 'c')]
        config: Option<PathBuf>,
    },
    
    /// Start CityHall in replica mode (read-only, syncs from leader)
    Replica {
        #[command(subcommand)]
        action: ReplicaAction,
    },
    
    /// Client commands (get, put, delete)
    Client {
        /// Server address (host:port)
        #[arg(long, short = 'a', default_value = "127.0.0.1:7878")]
        addr: String,
        
        #[command(subcommand)]
        command: ClientCommand,
    },
}

/// Replica subcommands
#[derive(Subcommand, Debug)]
pub enum ReplicaAction {
    /// Start replica and sync from leader
    Start {
        /// Leader address (host:port)
        #[arg(long, short = 'l')]
        leader: String,
        
        /// Data directory for WAL and state
        #[arg(long, default_value = "~/.cityhall/replica")]
        data_dir: PathBuf,
        
        /// Port for client connections (read-only)
        #[arg(long, short = 'p', default_value = "7880")]
        port: u16,
        
        /// WAL buffer size in bytes
        #[arg(long, default_value = "1048576")]
        wal_buffer_size: usize,
        
        /// Sync interval in seconds
        #[arg(long, short = 's', default_value = "5")]
        sync_interval: u64,
        
        /// Connect timeout in seconds
        #[arg(long, default_value = "5")]
        connect_timeout: u64,
        
        /// Read timeout in seconds
        #[arg(long, default_value = "30")]
        read_timeout: u64,
        
        /// Configuration file (TOML or JSON)
        #[arg(long, short = 'c')]
        config: Option<PathBuf>,
    },
    
    /// Show replica status (health, metrics, sync state)
    Status {
        /// Data directory
        #[arg(long, default_value = "~/.cityhall/replica")]
        data_dir: PathBuf,
        
        /// Show detailed metrics
        #[arg(long, short = 'v')]
        verbose: bool,
        
        /// Output format
        #[arg(long, short = 'f', default_value = "text")]
        format: OutputFormat,
    },
}

/// Client subcommands
#[derive(Subcommand, Debug)]
pub enum ClientCommand {
    /// Put a key-value pair
    Put {
        /// Key to store
        key: String,
        
        /// Value to store
        value: String,
    },
    
    /// Get a value by key
    Get {
        /// Key to retrieve
        key: String,
    },
    
    /// Delete a key
    Delete {
        /// Key to delete
        key: String,
    },
}

/// Output format for status commands
#[derive(Debug, Clone, clap::ValueEnum)]
pub enum OutputFormat {
    /// Human-readable text
    Text,
    /// JSON format
    Json,
    /// Compact format
    Compact,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_leader_command() {
        let cli = Cli::parse_from(&[
            "cityhall",
            "leader",
            "--data-dir",
            "/data/leader",
            "--port",
            "8000",
            "--replication-port",
            "8001",
        ]);
        
        match cli.command {
            Commands::Leader { data_dir, port, replication_port, .. } => {
                assert_eq!(data_dir, PathBuf::from("/data/leader"));
                assert_eq!(port, 8000);
                assert_eq!(replication_port, 8001);
            }
            _ => panic!("Expected Leader command"),
        }
    }
    
    #[test]
    fn test_parse_replica_start() {
        let cli = Cli::parse_from(&[
            "cityhall",
            "replica",
            "start",
            "--leader",
            "10.0.0.1:7879",
            "--data-dir",
            "/data/replica",
        ]);
        
        match cli.command {
            Commands::Replica { action } => {
                match action {
                    ReplicaAction::Start { leader, data_dir, .. } => {
                        assert_eq!(leader, "10.0.0.1:7879");
                        assert_eq!(data_dir, PathBuf::from("/data/replica"));
                    }
                    _ => panic!("Expected Start action"),
                }
            }
            _ => panic!("Expected Replica command"),
        }
    }
    
    #[test]
    fn test_parse_replica_status() {
        let cli = Cli::parse_from(&[
            "cityhall",
            "replica",
            "status",
            "--data-dir",
            "/data/replica",
            "--verbose",
            "--format",
            "json",
        ]);
        
        match cli.command {
            Commands::Replica { action } => {
                match action {
                    ReplicaAction::Status { data_dir, verbose, format } => {
                        assert_eq!(data_dir, PathBuf::from("/data/replica"));
                        assert!(verbose);
                        assert!(matches!(format, OutputFormat::Json));
                    }
                    _ => panic!("Expected Status action"),
                }
            }
            _ => panic!("Expected Replica command"),
        }
    }
    
    #[test]
    fn test_parse_client_put() {
        let cli = Cli::parse_from(&[
            "cityhall",
            "client",
            "put",
            "test.key",
            "test_value",
        ]);
        
        match cli.command {
            Commands::Client { command, .. } => {
                match command {
                    ClientCommand::Put { key, value } => {
                        assert_eq!(key, "test.key");
                        assert_eq!(value, "test_value");
                    }
                    _ => panic!("Expected Put command"),
                }
            }
            _ => panic!("Expected Client command"),
        }
    }
    
    #[test]
    fn test_parse_client_get() {
        let cli = Cli::parse_from(&[
            "cityhall",
            "client",
            "--addr",
            "192.168.1.1:7878",
            "get",
            "my.key",
        ]);
        
        match cli.command {
            Commands::Client { addr, command } => {
                assert_eq!(addr, "192.168.1.1:7878");
                match command {
                    ClientCommand::Get { key } => {
                        assert_eq!(key, "my.key");
                    }
                    _ => panic!("Expected Get command"),
                }
            }
            _ => panic!("Expected Client command"),
        }
    }
    
    #[test]
    fn test_default_values() {
        let cli = Cli::parse_from(&[
            "cityhall",
            "leader",
        ]);
        
        match cli.command {
            Commands::Leader { port, replication_port, wal_buffer_size, .. } => {
                assert_eq!(port, 7878);
                assert_eq!(replication_port, 7879);
                assert_eq!(wal_buffer_size, 1048576);
            }
            _ => panic!("Expected Leader command"),
        }
    }
}