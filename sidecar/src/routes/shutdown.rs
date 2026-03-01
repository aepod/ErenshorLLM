//! POST /shutdown endpoint.
//!
//! Triggers graceful shutdown of the sidecar process.
//! Memory persistence is handled automatically by redb (no explicit flush needed).

use crate::state::AppState;
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use serde::Serialize;
use std::sync::Arc;
use tracing::info;

#[derive(Serialize)]
struct ShutdownResponse {
    status: String,
}

async fn handle_shutdown(
    State(state): State<Arc<AppState>>,
) -> (StatusCode, Json<ShutdownResponse>) {
    info!("Shutdown requested via POST /shutdown");

    // No explicit memory flush needed -- redb persists automatically.
    // Notify the server to begin graceful shutdown.
    state.shutdown.notify_one();

    (
        StatusCode::ACCEPTED,
        Json(ShutdownResponse {
            status: "shutting_down".to_string(),
        }),
    )
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/shutdown", post(handle_shutdown))
}
