use clap::{Parser, Subcommand};
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use parking_lot::RwLock;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use cityhall::{Result, StorageEngine, Wal};

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
    /// Starts the CityHall server daemon
    Server {
        /// The address to bind the server to.
        #[arg(short, long, default_value = "127.0.0.1:7878")]
        bind_addr: String,
    },
    /// Client commands to interact with the server
    Client {
        #[command(subcommand)]
        command: ClientCommands,

        /// The address of the server to connect to.
        #[arg(short, long, default_value = "127.0.0.1:7878")]
        server_addr: String,
    },
}

#[derive(Subcommand)]
enum ClientCommands {
    /// Puts a key-value pair into the database
    Put { key: String, value: String },
    /// Gets the value associated with a key
    Get { key: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Server { bind_addr } => {
            let db_path = cli.db_path.unwrap_or_else(|| {
                let mut path = env::home_dir().expect("Could not determine home directory");
                path.push(".cityhall");
                path.push("data");
                path
            });
            let memtable_max_size = cli.memtable_max_size.unwrap_or(4 * 1024 * 1024);

            println!("Starting CityHall server on {}...", bind_addr);
            println!("Using database path: {:?}", db_path);

            // Create WAL first
            let wal_path = db_path.join("data.wal");
            let wal = Wal::new(&wal_path, 64 * 1024)?; // 64KB buffer
            let wal = Arc::new(RwLock::new(wal));

            // Now create StorageEngine with the WAL
            let engine = StorageEngine::new(db_path, memtable_max_size, wal)?;
            let engine = Arc::new(Mutex::new(engine));

            let listener = TcpListener::bind(bind_addr).await.unwrap();

            loop {
                let (stream, addr) = listener.accept().await.unwrap();
                println!("Accepted connection from: {}", addr);

                let engine = Arc::clone(&engine);

                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, engine).await {
                        eprintln!("Error handling connection: {}", e);
                    }
                });
            }
        }
        Commands::Client {
            command,
            server_addr,
        } => {
            let mut stream = TcpStream::connect(server_addr).await?;

            match command {
                ClientCommands::Put { key, value } => {
                    let command_str = format!("PUT {} {}\n", key, value);
                    stream.write_all(command_str.as_bytes()).await?;
                    let mut reader = BufReader::new(&mut stream);
                    let mut response_line = String::new();
                    reader.read_line(&mut response_line).await?;
                    print!("{}", response_line);
                }
                ClientCommands::Get { key } => {
                    let command_str = format!("GET {}\n", key);
                    stream.write_all(command_str.as_bytes()).await?;
                    let mut reader = BufReader::new(&mut stream);
                    let mut response_line = String::new();
                    reader.read_line(&mut response_line).await?;
                    print!("{}", response_line);
                }
            }
        }
    }

    Ok(())
}

async fn handle_connection(
    stream: TcpStream,
    engine: Arc<Mutex<StorageEngine>>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line).await?;
        if bytes_read == 0 {
            // Connection closed
            return Ok(());
        }

        let parts: Vec<&str> = line.trim().splitn(3, ' ').collect();
        let cmd = parts.get(0).unwrap_or(&"");

        let mut engine_lock = engine.lock().await;
        let writer = reader.get_mut();

        match *cmd {
            "PUT" => {
                if let (Some(key), Some(value)) = (parts.get(1), parts.get(2)) {
                    if let Err(e) =
                        engine_lock.put(key.as_bytes().to_vec(), value.as_bytes().to_vec())
                    {
                        writer
                            .write_all(format!("ERROR: {}\n", e).as_bytes())
                            .await?;
                    } else {
                        writer.write_all(b"OK\n").await?;
                    }
                } else {
                    writer.write_all(b"ERROR invalid PUT format\n").await?;
                }
            }
            "GET" => {
                if let Some(key) = parts.get(1) {
                    match engine_lock.get(key.as_bytes()) {
                        Ok(Some(value)) => {
                            let response = format!("VALUE {}\n", String::from_utf8_lossy(&value));
                            writer.write_all(response.as_bytes()).await?;
                        }
                        Ok(None) => {
                            writer.write_all(b"NOT_FOUND\n").await?;
                        }
                        Err(e) => {
                            writer
                                .write_all(format!("ERROR: {}\n", e).as_bytes())
                                .await?;
                        }
                    }
                } else {
                    writer.write_all(b"ERROR invalid GET format\n").await?;
                }
            }
            "" => { /* Ignore empty lines */ }
            _ => {
                writer.write_all(b"ERROR unknown command\n").await?;
            }
        }
    }
}