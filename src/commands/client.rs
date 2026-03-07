//! Client command implementation
//!
//! Provides PUT, GET, and DELETE operations against a running CityHall server.

use cityhall::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

/// Execute a PUT command: store a key-value pair on the server
pub async fn put(addr: &str, key: String, value: String) -> Result<()> {
    let mut stream = TcpStream::connect(addr).await?;
    let command = format!("PUT {} {}\n", key, value);
    stream.write_all(command.as_bytes()).await?;

    let mut reader = BufReader::new(&mut stream);
    let mut response = String::new();
    reader.read_line(&mut response).await?;

    let response = response.trim();
    if response == "OK" {
        println!("OK");
    } else {
        // Server returned ERROR <message>
        eprintln!("{}", response);
        std::process::exit(1);
    }

    Ok(())
}

/// Execute a GET command: retrieve a value by key from the server
pub async fn get(addr: &str, key: String) -> Result<()> {
    let mut stream = TcpStream::connect(addr).await?;
    let command = format!("GET {}\n", key);
    stream.write_all(command.as_bytes()).await?;

    let mut reader = BufReader::new(&mut stream);
    let mut response = String::new();
    reader.read_line(&mut response).await?;

    let response = response.trim();
    if let Some(value) = response.strip_prefix("VALUE ") {
        println!("{}", value);
    } else if response == "NOT_FOUND" {
        eprintln!("NOT_FOUND");
        std::process::exit(1);
    } else {
        // Server returned ERROR <message>
        eprintln!("{}", response);
        std::process::exit(1);
    }

    Ok(())
}

/// Execute a DELETE command.
///
/// DELETE is defined in the protocol but requires tombstone support in the
/// storage engine, which is not yet implemented. The server will return an
/// informative error until that work is complete.
pub async fn delete(addr: &str, key: String) -> Result<()> {
    let mut stream = TcpStream::connect(addr).await?;
    let command = format!("DELETE {}\n", key);
    stream.write_all(command.as_bytes()).await?;

    let mut reader = BufReader::new(&mut stream);
    let mut response = String::new();
    reader.read_line(&mut response).await?;

    let response = response.trim();
    // Server will respond: ERROR DELETE not yet implemented (tombstone support pending)
    eprintln!("{}", response);
    std::process::exit(1);
}