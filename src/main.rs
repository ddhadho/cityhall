use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::env;

// Import the necessary components from your library
use cityhall::{StorageEngine, Result}; // Removed unused Entry, Key, Value

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Optional path to the database directory. Defaults to ~/.cityhall/data
    #[arg(short, long, global = true, value_name = "PATH")]
    db_path: Option<PathBuf>,

    /// Maximum size of the memtable in bytes before flushing to disk.
    /// Defaults to 4MB (4 * 1024 * 1024).
    #[arg(short, long, global = true, value_name = "BYTES")]
    memtable_max_size: Option<usize>,
}

#[derive(Subcommand)]
enum Commands {
    /// Puts a key-value pair into the database
    Put {
        key: String,
        value: String,
    },
    /// Gets the value associated with a key
    Get {
        key: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let db_path = cli.db_path.unwrap_or_else(|| {
        let mut path = env::home_dir().expect("Could not determine home directory");
        path.push(".cityhall");
        path.push("data");
        path
    });

    let memtable_max_size = cli.memtable_max_size.unwrap_or(4 * 1024 * 1024); // Default to 4MB

    println!("Using database path: {:?}", db_path);
    println!("Memtable max size: {} bytes", memtable_max_size);

    // Make storage_engine mutable
    let mut storage_engine = StorageEngine::new(db_path, memtable_max_size)?;

    match &cli.command {
        Commands::Put { key, value } => {
            storage_engine.put(key.clone().into_bytes(), value.clone().into_bytes())?;
            println!("Successfully put key: \"{}\"", key);
        }
        Commands::Get { key } => {
            match storage_engine.get(&key.clone().into_bytes())? {
                Some(value_bytes) => {
                    println!("Found key: \"{}\", value: \"{}\"", key, String::from_utf8_lossy(&value_bytes));
                }
                None => {
                    println!("Key \"{}\" not found.", key);
                }
            }
        }
    }

    Ok(())
}
