use axum::{
    extract::State,
    routing::get,
    Json, Router,
};
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use crate::replication::metrics::ReplicationMetrics;
use crate::replication::state::ReplicaRegistry;

#[derive(Clone)]
pub struct AppState {
    pub metrics: Arc<ReplicationMetrics>,
    pub replica_registry: Arc<ReplicaRegistry>,
    pub current_wal_segment: Arc<tokio::sync::RwLock<u64>>,
}

pub async fn start_dashboard_server(
    metrics: Arc<ReplicationMetrics>,
    replica_registry: Arc<ReplicaRegistry>,
    current_wal_segment: Arc<tokio::sync::RwLock<u64>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let state = AppState {
        metrics,
        replica_registry,
        current_wal_segment,
    };

    let app = Router::new()
        .route("/api/metrics", get(get_metrics))
        .route("/dashboard", get(get_dashboard))
        .route("/", get(get_dashboard))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = TcpListener::bind("0.0.0.0:8080").await?;
    println!("📊 Dashboard: http://localhost:8080/dashboard");
    println!("📈 API: http://localhost:8080/api/metrics");
    
    axum::serve(listener, app).await?;
    Ok(())
}

async fn get_metrics(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let current_segment = *state.current_wal_segment.read().await;
    let dashboard_metrics = state.metrics
        .to_dashboard_metrics(&state.replica_registry, current_segment)
        .await;
    
    Json(serde_json::to_value(dashboard_metrics).unwrap())
}

async fn get_dashboard() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("../dashboard.html"))
}