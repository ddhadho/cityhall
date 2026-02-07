mod cli;
mod commands;

use clap::Parser;
use cli::{Cli, ClientCommand, Commands};

#[tokio::main]
async fn main() -> cityhall::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Server {
            data_dir,
            port,
            wal_buffer_size,
            config: _,
        } => {
            commands::server::run_server(data_dir, port, wal_buffer_size).await?;
        }

        Commands::Client { addr, command } => match command {
            ClientCommand::Put { key, value } => {
                commands::client::put(&addr, key, value).await?;
            }

            ClientCommand::Get { key } => {
                commands::client::get(&addr, key).await?;
            }

            ClientCommand::Delete { key } => {
                commands::client::delete(&addr, key).await?;
            }
        },
    }

    Ok(())
}
