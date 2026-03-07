//! Command dispatcher
//!
//! Routes parsed CLI commands to their implementations.

pub mod client;
pub mod server;

use crate::cli::{ClientCommand, Commands};
use cityhall::Result;

/// Dispatch a parsed command to its implementation
pub async fn dispatch(command: Commands) -> Result<()> {
    match command {
        Commands::Server {
            data_dir,
            port,
            wal_buffer_size,
            config: _, // config file support is reserved for a future release
        } => server::run_server(data_dir, port, wal_buffer_size).await,

        Commands::Client { addr, command } => match command {
            ClientCommand::Put { key, value } => client::put(&addr, key, value).await,
            ClientCommand::Get { key } => client::get(&addr, key).await,
            ClientCommand::Delete { key } => client::delete(&addr, key).await,
            ClientCommand::Metrics { dashboard_addr } => client::metrics(&dashboard_addr).await,
        },
    }
}