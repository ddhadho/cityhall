mod cli;
mod commands;

use clap::Parser;
use cli::Cli;

#[tokio::main]
async fn main() -> cityhall::Result<()> {
    let cli = Cli::parse();
    commands::dispatch(cli.command).await
}