//! A scripted stand-in for the `/api/ingest` route, for tests.
use axum::{Json, Router, extract::State, http::{HeaderMap, StatusCode}, routing::post};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct Script {
    /// Statuses to answer with, in order; once empty, every call gets 200.
    pub statuses: VecDeque<u16>,
    pub calls: usize,
    pub keys: Vec<String>,
    pub rows_seen: usize,
    pub user_agents: Vec<String>,
}

pub type Shared = Arc<Mutex<Script>>;

async fn handler(State(s): State<Shared>, headers: HeaderMap, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
    let mut s = s.lock().unwrap();
    s.calls += 1;
    s.keys.push(headers.get("x-api-key").and_then(|v| v.to_str().ok()).unwrap_or("").to_string());
    let status = s.statuses.pop_front().unwrap_or(200);
    let rows = body["rows"].as_array().cloned().unwrap_or_default();
    if status == 200 {
        s.rows_seen += rows.len();
        let ids: Vec<Value> = rows.iter().map(|r| r["id"].clone()).collect();
        return (StatusCode::OK, Json(json!({ "accepted": ids })));
    }
    (StatusCode::from_u16(status).unwrap(), Json(json!({ "error": "scripted" })))
}

async fn sec(State(s): State<Shared>, headers: HeaderMap) -> StatusCode {
    let ua = headers.get("user-agent").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    s.lock().unwrap().user_agents.push(ua);
    StatusCode::OK
}

/// Start the stand-in; returns its URL and the shared script.
pub async fn ingest_server(statuses: &[u16]) -> (String, Shared) {
    let shared: Shared = Arc::new(Mutex::new(Script { statuses: statuses.iter().copied().collect(), ..Default::default() }));
    let app = Router::new()
        .route("/api/ingest", post(handler))
        .route("/sec", axum::routing::get(sec)).with_state(shared.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/api/ingest"), shared)
}
