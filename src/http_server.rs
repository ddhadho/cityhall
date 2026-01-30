use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;

use crate::replication::metrics::ReplicationMetrics;
use crate::replication::registry::ReplicaRegistry;
use crate::replication::ConnectionState;

#[derive(Clone)]
pub struct AppState {
    pub metrics: Arc<ReplicationMetrics>,
    pub replica_registry: Arc<ReplicaRegistry>,
    pub current_wal_segment: Arc<tokio::sync::RwLock<u64>>,
    pub start_time: Instant,
}

pub async fn start_dashboard_server(
    metrics: Arc<ReplicationMetrics>,
    replica_registry: Arc<ReplicaRegistry>,
    current_wal_segment: Arc<tokio::sync::RwLock<u64>>,
    start_time: Instant,
) {
    let state = AppState {
        metrics,
        replica_registry,
        current_wal_segment,
        start_time,
    };

    let app = Router::new()
        .route("/api/metrics", get(get_metrics))
        .route("/dashboard", get(get_dashboard))
        .route("/", get(get_dashboard))
        .layer(CorsLayer::permissive())
        .with_state(state);

    match TcpListener::bind("0.0.0.0:8080").await {
        Ok(listener) => {
            println!("📊 Dashboard: http://localhost:8080/dashboard");
            println!("📈 API: http://localhost:8080/api/metrics");

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

    // Replication
    pub total_entries: u64,
    pub wal_segments: u64,
    pub replicas: Vec<ReplicaDashboardInfo>,
    pub connected_count: usize,
    pub throughput_entries_per_sec: f64,

    // Storage
    pub storage_metrics: StorageMetrics,

    pub last_updated: u64,
}

#[derive(Serialize)]
pub struct ReplicaDashboardInfo {
    pub replica_id: String,
    pub status: String,
    pub last_segment: u64,
    pub lag_segments: i64,
    pub last_seen_ago_secs: u64,
    pub bytes_sent: u64,
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
    let current_segment = *state.current_wal_segment.read().await;
    let replicas_info = state.replica_registry.get_all().await;

    let replicas: Vec<ReplicaDashboardInfo> = replicas_info
        .iter()
        .map(|info| {
            let lag = current_segment as i64 - info.last_segment_requested as i64;
            let last_seen = info.last_heartbeat.elapsed().as_secs();

            ReplicaDashboardInfo {
                replica_id: info.replica_id.clone(),
                status: format!("{:?}", info.connection_state),
                last_segment: info.last_segment_requested,
                lag_segments: lag.max(0),
                last_seen_ago_secs: last_seen,
                bytes_sent: info.bytes_sent,
            }
        })
        .collect();

    let connected_count = replicas_info
        .iter()
        .filter(|r| r.connection_state != ConnectionState::Offline)
        .count();

    let metrics = crate::metrics::metrics();

    // Calculate throughput since leader start
    let uptime_secs = state.start_time.elapsed().as_secs_f64();
    let throughput = if uptime_secs > 0.0 {
        metrics.writes_total.get() as f64 / uptime_secs
    } else {
        0.0
    };

    let storage_metrics = StorageMetrics {
        // Operations
        writes_total: metrics.writes_total.get(),
        reads_total: metrics.reads_total.get(),
        reads_hits: metrics.reads_hits.get(),
        reads_misses: metrics.reads_misses.get(),
        flushes_total: metrics.flushes_total.get(),
        compactions_total: metrics.compactions_total.get(),

        // Performance
        read_hit_rate: metrics.read_hit_rate(),
        write_latency_p50_us: metrics.write_latency.percentile(0.50).as_micros() as f64,
        write_latency_p99_us: metrics.write_latency.percentile(0.99).as_micros() as f64,
        read_latency_p50_us: metrics.read_latency.percentile(0.50).as_micros() as f64,
        read_latency_p99_us: metrics.read_latency.percentile(0.99).as_micros() as f64,

        // System state
        memtable_size_mb: metrics.memtable_size_bytes.get() as f64 / 1_048_576.0,
        memtable_entries: metrics.memtable_entries.get(),
        sstable_count: metrics.sstable_count.get(),
        disk_usage_mb: metrics.disk_usage_bytes.get() as f64 / 1_048_576.0,
        wal_size_mb: metrics.wal_size_bytes.get() as f64 / 1_048_576.0,

        // Bloom filter
        bloom_filter_hit_rate: metrics.bloom_filter_hit_rate(),
        bloom_filter_fp_rate: metrics.bloom_filter_fp_rate(),

        // Compaction
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

        total_entries: metrics.writes_total.get(),
        wal_segments: current_segment,
        replicas,
        connected_count,
        throughput_entries_per_sec: throughput,

        storage_metrics,

        last_updated: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    };

    Json(dashboard_metrics)
}

async fn get_dashboard() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("../dashboard.html"))
}
