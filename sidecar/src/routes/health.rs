//! GET /health endpoint.
//!
//! Returns the sidecar's current status and loaded resource statistics.

use crate::state::AppState;
use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;
use std::sync::Arc;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub uptime_seconds: u64,
    pub embedding_model_loaded: bool,
    pub lore_index: IndexStatus,
    pub response_index: IndexStatus,
    pub memory_index: IndexStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sona: Option<SonaStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm: Option<LlmStatus>,
    pub personalities_loaded: usize,
}

#[derive(Serialize)]
pub struct IndexStatus {
    pub loaded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entries: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

#[derive(Serialize)]
pub struct SonaStatus {
    pub enabled: bool,
    pub trajectory_count: usize,
    pub pattern_count: usize,
    pub learning_cycles: u64,
    pub queries_enhanced: u64,
}

#[derive(Serialize)]
pub struct LlmStatus {
    pub enabled: bool,
    pub mode: String,
    pub status: String,
    pub model: String,
}

async fn handle_health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let response = HealthResponse {
        status: "ready".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: state.uptime_seconds(),
        embedding_model_loaded: state.embedder.is_some(),
        lore_index: IndexStatus {
            loaded: state.lore.is_loaded(),
            entries: if state.lore.is_loaded() {
                Some(state.lore.entry_count())
            } else {
                None
            },
            file: Some(state.config.indexes.lore_path.clone()),
        },
        response_index: IndexStatus {
            loaded: state.responses.is_loaded(),
            entries: if state.responses.is_loaded() {
                Some(state.responses.entry_count())
            } else {
                None
            },
            file: Some(state.config.indexes.responses_path.clone()),
        },
        memory_index: IndexStatus {
            loaded: state.memory.is_loaded(),
            entries: if state.memory.is_loaded() {
                Some(state.memory.entry_count())
            } else {
                None
            },
            file: Some(state.config.indexes.memory_path.clone()),
        },
        sona: state.sona.as_ref().map(|sona| {
            let stats = sona.stats();
            SonaStatus {
                enabled: stats.enabled,
                trajectory_count: stats.trajectory_count,
                pattern_count: stats.pattern_count,
                learning_cycles: stats.learning_cycles,
                queries_enhanced: stats.queries_enhanced,
            }
        }),
        llm: if state.config.llm.enabled {
            Some(LlmStatus {
                enabled: true,
                mode: format!("{:?}", state.config.llm.mode).to_lowercase(),
                status: state
                    .llm_router
                    .as_ref()
                    .map(|r| r.status().to_string())
                    .unwrap_or_else(|| "not_initialized".to_string()),
                model: state
                    .llm_router
                    .as_ref()
                    .map(|r| r.model_name())
                    .unwrap_or_else(|| "none".to_string()),
            })
        } else {
            None
        },
        personalities_loaded: state.personality_store.count(),
    };

    Json(response)
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/health", get(handle_health))
}
