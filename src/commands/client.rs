//! Client command implementation
//!
//! Provides PUT, GET, DELETE, and METRICS operations against a running CityHall server.

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

    eprintln!("{}", response.trim());
    std::process::exit(1);
}

/// Fetch and pretty-print live metrics from the dashboard HTTP server.
///
/// Hits GET /api/metrics on the dashboard address (default: http://127.0.0.1:8080)
/// and renders the response in a readable format matching the server startup style.
pub async fn metrics(dashboard_addr: &str) -> Result<()> {
    let url = format!("{}/api/metrics", dashboard_addr.trim_end_matches('/'));
    let body = fetch_metrics(&url).await?;

    let data: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| cityhall::StorageError::Corruption(format!("invalid metrics JSON: {}", e)))?;

    let s = &data["storage_metrics"];
    let uptime = data["uptime_seconds"].as_u64().unwrap_or(0);
    let node_id = data["node_id"].as_str().unwrap_or("unknown");

    let h = uptime / 3600;
    let m = (uptime % 3600) / 60;
    let sec = uptime % 60;

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🏙️  CityHall Metrics  —  {}", node_id);
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("   Uptime:  {}h {}m {}s", h, m, sec);
    println!();

    println!("📊 Operations");
    println!("   Writes:      {:>12}", fmt_u64(&s["writes_total"]));
    println!(
        "   Reads:       {:>12}  (hits: {}, misses: {})",
        fmt_u64(&s["reads_total"]),
        fmt_u64(&s["reads_hits"]),
        fmt_u64(&s["reads_misses"]),
    );
    println!("   Flushes:     {:>12}", fmt_u64(&s["flushes_total"]));
    println!("   Compactions: {:>12}", fmt_u64(&s["compactions_total"]));
    println!();

    println!("⚡ Latency");
    println!("   Write  p50:  {:>9.1} us", s["write_latency_p50_us"].as_f64().unwrap_or(0.0));
    println!("   Write  p99:  {:>9.1} us", s["write_latency_p99_us"].as_f64().unwrap_or(0.0));
    println!("   Read   p50:  {:>9.1} us", s["read_latency_p50_us"].as_f64().unwrap_or(0.0));
    println!("   Read   p99:  {:>9.1} us", s["read_latency_p99_us"].as_f64().unwrap_or(0.0));
    println!();

    println!("💾 Storage");
    println!(
        "   MemTable:    {:>9.2} MB  ({} entries)",
        s["memtable_size_mb"].as_f64().unwrap_or(0.0),
        fmt_u64(&s["memtable_entries"]),
    );
    println!("   SSTables:    {:>12}", fmt_u64(&s["sstable_count"]));
    println!("   WAL:         {:>9.2} MB", s["wal_size_mb"].as_f64().unwrap_or(0.0));
    println!("   Disk:        {:>9.2} MB", s["disk_usage_mb"].as_f64().unwrap_or(0.0));
    println!();

    println!("🌸 Bloom Filter");
    println!("   Hit rate:    {:>9.2}%", s["bloom_filter_hit_rate"].as_f64().unwrap_or(0.0) * 100.0);
    println!("   FP rate:     {:>9.2}%", s["bloom_filter_fp_rate"].as_f64().unwrap_or(0.0) * 100.0);
    println!();

    println!("🔄 Compaction");
    println!("   Space saved: {:>9.2}%", s["compaction_space_savings"].as_f64().unwrap_or(0.0) * 100.0);
    println!("   Write amp:   {:>9.2}x", s["write_amplification"].as_f64().unwrap_or(0.0));
    println!();

    println!("   Read hit rate: {:.1}%", s["read_hit_rate"].as_f64().unwrap_or(0.0) * 100.0);
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    Ok(())
}

// --- helpers ----------------------------------------------------------------

fn fmt_u64(v: &serde_json::Value) -> String {
    v.as_u64()
        .map(|n| {
            let s = n.to_string();
            let mut out = String::new();
            for (i, c) in s.chars().rev().enumerate() {
                if i > 0 && i % 3 == 0 {
                    out.push(',');
                }
                out.push(c);
            }
            out.chars().rev().collect()
        })
        .unwrap_or_else(|| "-".to_string())
}

/// Raw HTTP GET over tokio TCP — no extra dependencies needed
async fn fetch_metrics(url: &str) -> Result<String> {
    let url = url.strip_prefix("http://").ok_or_else(|| {
        cityhall::StorageError::Corruption("metrics URL must start with http://".into())
    })?;

    let (host_port, path) = if let Some(idx) = url.find('/') {
        (&url[..idx], &url[idx..])
    } else {
        (url, "/")
    };

    let mut stream = TcpStream::connect(host_port).await.map_err(|e| {
        cityhall::StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            format!(
                "could not connect to dashboard at {} — is the server running? ({})",
                host_port, e
            ),
        ))
    })?;

    let request = format!(
        "GET {} HTTP/1.0\r\nHost: {}\r\nAccept: application/json\r\n\r\n",
        path, host_port
    );
    stream.write_all(request.as_bytes()).await?;

    let mut reader = BufReader::new(&mut stream);
    let mut raw = String::new();
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        raw.push_str(&line);
    }

    // Strip HTTP headers — body starts after the first blank line
    if let Some(idx) = raw.find("\r\n\r\n") {
        Ok(raw[idx + 4..].to_string())
    } else if let Some(idx) = raw.find("\n\n") {
        Ok(raw[idx + 2..].to_string())
    } else {
        Err(cityhall::StorageError::Corruption(
            "malformed HTTP response from dashboard".into(),
        ))
    }
}