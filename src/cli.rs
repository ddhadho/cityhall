//! Command-line interface definitions
//!
//! Defines all CLI commands and arguments using clap

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// CityHall - Time-series database
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
    /// Start CityHall in server mode (accepts writes)
    Server {
        /// Data directory for WAL and state
        #[arg(long, default_value = "~/.cityhall/server")]
        data_dir: PathBuf,

        /// Port for client connections
        #[arg(long, short = 'p', default_value = "7878")]
        port: u16,

        /// WAL buffer size in bytes
        #[arg(long, default_value = "1048576")]
        wal_buffer_size: usize,

        /// Configuration file (TOML or JSON)
        #[arg(long, short = 'c')]
        config: Option<PathBuf>,
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
    fn test_parse_server_command() {
        let cli = Cli::parse_from(&[
            "cityhall",
            "server",
            "--data-dir",
            "/data/server",
            "--port",
            "8000",
        ]);

        match cli.command {
            Commands::Server {
                data_dir,
                port,
                ..
            } => {
                assert_eq!(data_dir, PathBuf::from("/data/server"));
                assert_eq!(port, 8000);
            }
            _ => panic!("Expected Server command"),
        }
    }

    #[test]
    fn test_parse_client_put() {
        let cli = Cli::parse_from(&["cityhall", "client", "put", "test.key", "test_value"]);

        match cli.command {
            Commands::Client { command, .. } => match command {
                ClientCommand::Put { key, value } => {
                    assert_eq!(key, "test.key");
                    assert_eq!(value, "test_value");
                }
                _ => panic!("Expected Put command"),
            },
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
        let cli = Cli::parse_from(&["cityhall", "server"]);

        match cli.command {
            Commands::Server {
                port,
                wal_buffer_size,
                ..
            } => {
                assert_eq!(port, 7878);
                assert_eq!(wal_buffer_size, 1048576);
            }
            _ => panic!("Expected Server command"),
        }
    }
}
