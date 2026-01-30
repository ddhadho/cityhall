//! Client command implementation
//!
//! Provides PUT/GET/DELETE operations against a CityHall server

use cityhall::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

/// Execute a PUT command
pub async fn put(addr: &str, key: String, value: String) -> Result<()> {
    let mut stream = TcpStream::connect(addr).await?;

    let command = format!("PUT {} {}\n", key, value);
    stream.write_all(command.as_bytes()).await?;

    let mut reader = BufReader::new(&mut stream);
    let mut response = String::new();
    reader.read_line(&mut response).await?;

    print!("{}", response);

    Ok(())
}

/// Execute a GET command
pub async fn get(addr: &str, key: String) -> Result<()> {
    let mut stream = TcpStream::connect(addr).await?;

    let command = format!("GET {}\n", key);
    stream.write_all(command.as_bytes()).await?;

    let mut reader = BufReader::new(&mut stream);
    let mut response = String::new();
    reader.read_line(&mut response).await?;

    print!("{}", response);

    Ok(())
}

/// Execute a DELETE command
pub async fn delete(addr: &str, key: String) -> Result<()> {
    let mut stream = TcpStream::connect(addr).await?;

    let command = format!("DELETE {}\n", key);
    stream.write_all(command.as_bytes()).await?;

    let mut reader = BufReader::new(&mut stream);
    let mut response = String::new();
    reader.read_line(&mut response).await?;

    print!("{}", response);

    Ok(())
}
