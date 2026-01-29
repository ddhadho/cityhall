use axum::{
    extract::State,
    routing::get,
    Json, Router,
};
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use crate::replication::registry::ReplicaRegistry;
use crate::StorageEngine;
use parking_lot::Mutex;
use serde::Serialize;

#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<Mutex<StorageEngine>>,
    pub replica_registry: Arc<ReplicaRegistry>,
    pub current_wal_segment: Arc<tokio::sync::RwLock<u64>>,
}

#[derive(Serialize)]
pub struct DashboardMetrics {
    pub node_type: String,
    pub node_id: String,
    pub uptime_seconds: u64,
    pub total_entries: u64,
    pub wal_segments: u64,
    pub replicas: Vec<ReplicaDashboardInfo>,
    pub connected_count: usize,
    pub throughput_entries_per_sec: f64,
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

pub async fn start_dashboard_server(
    storage: Arc<Mutex<StorageEngine>>,
    replica_registry: Arc<ReplicaRegistry>,
    current_wal_segment: Arc<tokio::sync::RwLock<u64>>,
) {
    let state = AppState {
        storage,
        replica_registry,
        current_wal_segment,
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

async fn get_metrics(
    State(state): State<AppState>,
) -> Json<DashboardMetrics> {
    // Get current WAL segment
    let current_segment = *state.current_wal_segment.read().await;
    
    // Get all replicas from registry
    let replicas_info = state.replica_registry.get_all().await;
    
    // Transform replica info for dashboard
    let replicas = replicas_info.iter().map(|info| {
        let lag = current_segment as i64 - info.last_segment_requested as i64;
        let last_seen = info.last_heartbeat.elapsed().as_secs();
        
        ReplicaDashboardInfo {
            replica_id: info.replica_id.clone(),
            status: format!("{:?}", info.connection_state),
            last_segment: info.last_segment_requested,
            lag_segments: lag,
            last_seen_ago_secs: last_seen,
            bytes_sent: info.bytes_sent,
        }
    }).collect();
    
    let connected_count = replicas_info.iter()
        .filter(|r| r.connection_state != crate::replication::ConnectionState::Offline)
        .count();
    
    let metrics = DashboardMetrics {
        node_type: "Leader".to_string(),
        node_id: "leader-1".to_string(),
        uptime_seconds: 0, // TODO: Track uptime
        total_entries: 0,  // TODO: Get from storage engine
        wal_segments: current_segment,
        replicas,
        connected_count,
        throughput_entries_per_sec: 0.0, // TODO: Calculate
        last_updated: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    };
    
    Json(metrics)
}

async fn get_dashboard() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("../dashboard.html"))
}