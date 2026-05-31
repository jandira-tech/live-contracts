use axum::{Router, extract::State, response::Json, routing::get};
use serde_json::{json, Value};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone)]
pub struct HealthState {
    pub total_seen: Arc<AtomicU64>,
}

async fn health_handler(State(state): State<HealthState>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "total_seen": state.total_seen.load(Ordering::Relaxed),
    }))
}

pub fn router(state: HealthState) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .with_state(state)
}

pub async fn serve(port: u16, state: HealthState) {
    let app = router(state);
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("health server listening on {addr}");
    if let Err(e) = axum::serve(
        tokio::net::TcpListener::bind(addr).await.expect("bind health"),
        app,
    )
    .await
    {
        tracing::error!("health server error: {e}");
    }
}
