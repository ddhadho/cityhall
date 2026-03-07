use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;

#[derive(Clone)]
pub struct AppState {
    pub current_wal_segment: Arc<tokio::sync::RwLock<u64>>,
    pub start_time: Instant,
}

pub async fn start_dashboard_server(
    current_wal_segment: Arc<tokio::sync::RwLock<u64>>,
    start_time: Instant,
) {
    let state = AppState {
        current_wal_segment,
        start_time,
    };

    let app = Router::new()
        .route("/api/metrics", get(get_metrics))
        .route("/metrics", get(get_prometheus_metrics))  // Prometheus scrape endpoint
        .route("/dashboard", get(get_dashboard))
        .route("/", get(get_dashboard))
        .layer(CorsLayer::permissive())
        .with_state(state);

    match TcpListener::bind("0.0.0.0:8080").await {
        Ok(listener) => {
            println!("📊 Dashboard: http://localhost:8080/dashboard");
            println!("📈 API:       http://localhost:8080/api/metrics");
            println!("📡 Prometheus: http://localhost:8080/metrics");

            if let Err(e) = axum::serve(listener, app).await {
                eprintln!("❌ Dashboard server error: {}", e);
            }
        }
        Err(e) => {
            eprintln!("⚠️  Failed to start dashboard server on port 8080: {}", e);
            eprintln!("   Dashboard will not be available");
        }
    }
}

/* =========================
API TYPES
========================= */

#[derive(Serialize)]
pub struct DashboardMetrics {
    pub node_type: String,
    pub node_id: String,
    pub uptime_seconds: u64,
    pub storage_metrics: StorageMetrics,
    pub last_updated: u64,
}

#[derive(Serialize)]
pub struct StorageMetrics {
    // Operations
    pub writes_total: u64,
    pub reads_total: u64,
    pub reads_hits: u64,
    pub reads_misses: u64,
    pub flushes_total: u64,
    pub compactions_total: u64,

    // Performance
    pub read_hit_rate: f64,
    pub write_latency_p50_us: f64,
    pub write_latency_p99_us: f64,
    pub read_latency_p50_us: f64,
    pub read_latency_p99_us: f64,

    // System state
    pub memtable_size_mb: f64,
    pub memtable_entries: u64,
    pub sstable_count: u64,
    pub disk_usage_mb: f64,
    pub wal_size_mb: f64,

    // Bloom filter
    pub bloom_filter_hit_rate: f64,
    pub bloom_filter_fp_rate: f64,

    // Compaction
    pub compaction_space_savings: f64,
    pub write_amplification: f64,
}

/* =========================
HANDLERS
========================= */

async fn get_metrics(State(state): State<AppState>) -> Json<DashboardMetrics> {
    let metrics = crate::metrics::metrics();

    let storage_metrics = StorageMetrics {
        writes_total: metrics.writes_total.get(),
        reads_total: metrics.reads_total.get(),
        reads_hits: metrics.reads_hits.get(),
        reads_misses: metrics.reads_misses.get(),
        flushes_total: metrics.flushes_total.get(),
        compactions_total: metrics.compactions_total.get(),

        read_hit_rate: metrics.read_hit_rate(),
        write_latency_p50_us: metrics.write_latency.percentile(0.50).as_micros() as f64,
        write_latency_p99_us: metrics.write_latency.percentile(0.99).as_micros() as f64,
        read_latency_p50_us: metrics.read_latency.percentile(0.50).as_micros() as f64,
        read_latency_p99_us: metrics.read_latency.percentile(0.99).as_micros() as f64,

        memtable_size_mb: metrics.memtable_size_bytes.get() as f64 / 1_048_576.0,
        memtable_entries: metrics.memtable_entries.get(),
        sstable_count: metrics.sstable_count.get(),
        disk_usage_mb: metrics.disk_usage_bytes.get() as f64 / 1_048_576.0,
        wal_size_mb: metrics.wal_size_bytes.get() as f64 / 1_048_576.0,

        bloom_filter_hit_rate: metrics.bloom_filter_hit_rate(),
        bloom_filter_fp_rate: metrics.bloom_filter_fp_rate(),

        compaction_space_savings: metrics.compaction_space_savings(),
        write_amplification: metrics.write_amplification(),
    };

    let dashboard_metrics = DashboardMetrics {
        node_type: "Leader".to_string(),
        node_id: hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "leader-main".to_string()),
        uptime_seconds: state.start_time.elapsed().as_secs(),
        storage_metrics,
        last_updated: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    };

    Json(dashboard_metrics)
}

/// Prometheus text format scrape endpoint.
///
/// Compatible with any Prometheus scraper or Grafana data source.
/// Accessible at GET /metrics on the dashboard port (default: 8080).
///
/// Example:
///   curl http://localhost:8080/metrics
async fn get_prometheus_metrics(State(state): State<AppState>) -> String {
    let m = crate::metrics::metrics();
    let uptime = state.start_time.elapsed().as_secs();

    let mut out = String::with_capacity(2048);

    // helper macros for emitting Prometheus lines
    macro_rules! counter {
        ($name:expr, $help:expr, $val:expr) => {
            out.push_str(&format!(
                "# HELP {} {}\n# TYPE {} counter\n{} {}\n",
                $name, $help, $name, $name, $val
            ));
        };
    }

    macro_rules! gauge {
        ($name:expr, $help:expr, $val:expr) => {
            out.push_str(&format!(
                "# HELP {} {}\n# TYPE {} gauge\n{} {}\n",
                $name, $help, $name, $name, $val
            ));
        };
    }

    // Operations
    counter!("cityhall_writes_total",      "Total write operations",      m.writes_total.get());
    counter!("cityhall_reads_total",       "Total read operations",       m.reads_total.get());
    counter!("cityhall_reads_hits_total",  "Total read hits",             m.reads_hits.get());
    counter!("cityhall_reads_misses_total","Total read misses",           m.reads_misses.get());
    counter!("cityhall_flushes_total",     "Total MemTable flushes",      m.flushes_total.get());
    counter!("cityhall_compactions_total", "Total compaction runs",       m.compactions_total.get());

    // Latency
    gauge!("cityhall_write_latency_p50_us", "Write latency 50th percentile microseconds",
           m.write_latency.percentile(0.50).as_micros());
    gauge!("cityhall_write_latency_p99_us", "Write latency 99th percentile microseconds",
           m.write_latency.percentile(0.99).as_micros());
    gauge!("cityhall_read_latency_p50_us",  "Read latency 50th percentile microseconds",
           m.read_latency.percentile(0.50).as_micros());
    gauge!("cityhall_read_latency_p99_us",  "Read latency 99th percentile microseconds",
           m.read_latency.percentile(0.99).as_micros());

    // Storage state
    gauge!("cityhall_memtable_size_bytes",  "Current MemTable size in bytes",   m.memtable_size_bytes.get());
    gauge!("cityhall_memtable_entries",     "Current MemTable entry count",      m.memtable_entries.get());
    gauge!("cityhall_sstable_count",        "Number of SSTables on disk",        m.sstable_count.get());
    gauge!("cityhall_disk_usage_bytes",     "Total disk usage in bytes",         m.disk_usage_bytes.get());
    gauge!("cityhall_wal_size_bytes",       "Current WAL size in bytes",         m.wal_size_bytes.get());

    // Computed rates
    gauge!("cityhall_read_hit_rate",              "Read hit rate (0.0 to 1.0)",             m.read_hit_rate());
    gauge!("cityhall_bloom_filter_hit_rate",      "Bloom filter hit rate (0.0 to 1.0)",     m.bloom_filter_hit_rate());
    gauge!("cityhall_bloom_filter_fp_rate",       "Bloom filter false positive rate",        m.bloom_filter_fp_rate());
    gauge!("cityhall_compaction_space_savings",   "Compaction space savings (0.0 to 1.0)",  m.compaction_space_savings());
    gauge!("cityhall_write_amplification",        "Write amplification factor",              m.write_amplification());

    // Process
    gauge!("cityhall_uptime_seconds", "Server uptime in seconds", uptime);

    out
}

async fn get_dashboard() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("../dashboard.html"))
}