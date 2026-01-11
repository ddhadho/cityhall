mod cli;
mod commands;

use clap::Parser;
use cli::{Cli, Commands, ReplicaAction, ClientCommand};

#[tokio::main]
async fn main() -> cityhall::Result<()> {
    let cli = Cli::parse();
    
    match cli.command {
        Commands::Leader {
            data_dir,
            port,
            replication_port,
            wal_buffer_size,
            config: _,
        } => {
            commands::leader::run_leader(
                data_dir,
                port,
                replication_port,
                wal_buffer_size,
            )
            .await?;
        }
        
        Commands::Replica { action } => {
            match action {
                ReplicaAction::Start {
                    leader,
                    data_dir,
                    port: _,  // Not used in this simple version
                    wal_buffer_size: _,  // Not used, hardcoded in replica
                    sync_interval,
                    connect_timeout,
                    read_timeout,
                    config: _,
                } => {
                    commands::replica::run_replica(
                        leader,
                        data_dir,
                        sync_interval,
                        connect_timeout,
                        read_timeout,
                    )
                    .await?;
                }
                
                ReplicaAction::Status {
                    data_dir,
                    verbose,
                    format,
                } => {
                    // Convert cli::OutputFormat to commands::replica::OutputFormat
                    let output_format = match format {
                        cli::OutputFormat::Text => commands::replica::OutputFormat::Text,
                        cli::OutputFormat::Json => commands::replica::OutputFormat::Json,
                        cli::OutputFormat::Compact => commands::replica::OutputFormat::Compact,
                    };
                    
                    commands::replica::show_replica_status(data_dir, verbose, output_format).await?;
                }
            }
        }
        
        Commands::Client { addr, command } => {
            match command {
                ClientCommand::Put { key, value } => {
                    commands::client::put(&addr, key, value).await?;
                }
                
                ClientCommand::Get { key } => {
                    commands::client::get(&addr, key).await?;
                }
                
                ClientCommand::Delete { key } => {
                    commands::client::delete(&addr, key).await?;
                }
            }
        }
    }
    
    Ok(())
}