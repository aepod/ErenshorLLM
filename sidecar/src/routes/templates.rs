//! Routes for /v1/templates/* -- dynamic template management.
//!
//! POST /v1/templates/queue   -- Queue a template generation request (stub for M5)
//! GET  /v1/templates/lookup  -- Look up a variant by trigger + personality
//! GET  /v1/templates/stats   -- Template store statistics
//! POST /v1/templates/import  -- Bulk import template variants

use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

use crate::intelligence::template_learning::{GenerationRequest, PersonalityHint, StoredVariant};
use crate::state::AppState;

// ---- Request/Response types ----

#[derive(Debug, Deserialize)]
pub struct QueueRequest {
    pub trigger: String,
    pub personality: Option<PersonalityHint>,
    pub sim_name: Option<String>,
    pub channel: Option<String>,
    pub context: Option<String>,
    pub count: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct LookupParams {
    pub trigger: String,
    pub personality_style: Option<String>,
    pub personality_class_role: Option<String>,
    pub personality_traits: Option<String>, // comma-separated
}

#[derive(Debug, Deserialize)]
pub struct ImportRequest {
    pub templates: HashMap<String, Vec<ImportVariant>>,
}

#[derive(Debug, Deserialize)]
pub struct ImportVariant {
    pub text: String,
    #[serde(default)]
    pub personality: PersonalityHint,
    #[serde(default)]
    pub source_sim: String,
    #[serde(default = "default_channel")]
    pub channel: String,
}

fn default_channel() -> String {
    "say".to_string()
}

// ---- Handlers ----

/// POST /v1/templates/queue
///
/// Accepts a generation request and enqueues it for background LLM processing.
/// Returns 202 Accepted with queue position on success.
async fn handle_queue(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QueueRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    // Validate required fields
    if req.trigger.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "trigger is required"
            })),
        );
    }

    // Check if template store is enabled
    if state.template_store.is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "template store is not enabled"
            })),
        );
    }

    // Check if generator is available
    let generator = match &state.template_generator {
        Some(g) => g,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": "template generator is not available (LLM disabled?)"
                })),
            );
        }
    };

    let count = req.count.unwrap_or(4).min(8).max(1);

    let gen_req = GenerationRequest {
        trigger: req.trigger.clone(),
        original: req.context.unwrap_or_else(|| req.trigger.clone()),
        channel: req.channel.unwrap_or_else(|| "say".to_string()),
        personality: req.personality.unwrap_or_default(),
        count,
    };

    match generator.enqueue(gen_req).await {
        Ok(position) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "queued": true,
                "trigger": req.trigger,
                "count": count,
                "queue_position": position
            })),
        ),
        Err("queue_full") => (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({
                "error": "generation queue is full, try again later"
            })),
        ),
        Err(reason) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": reason
            })),
        ),
    }
}

/// GET /v1/templates/lookup?trigger=pulling&personality_style=enthusiastic&personality_class_role=tank
async fn handle_lookup(
    State(state): State<Arc<AppState>>,
    Query(params): Query<LookupParams>,
) -> Json<serde_json::Value> {
    let store_lock = match &state.template_store {
        Some(s) => s,
        None => {
            return Json(serde_json::json!({
                "found": false,
                "error": "template store is not enabled"
            }));
        }
    };

    let hint = PersonalityHint {
        style: params.personality_style.unwrap_or_default(),
        class_role: params.personality_class_role.unwrap_or_default(),
        traits: params
            .personality_traits
            .map(|t| {
                t.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
        source_sim: None,
    };

    let mut store = store_lock.write().await;
    match store.lookup(&params.trigger, &hint) {
        Some(text) => Json(serde_json::json!({
            "found": true,
            "trigger": params.trigger,
            "text": text
        })),
        None => Json(serde_json::json!({
            "found": false,
            "trigger": params.trigger
        })),
    }
}

/// GET /v1/templates/stats
async fn handle_stats(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    match &state.template_store {
        Some(store_lock) => {
            let store = store_lock.read().await;
            let (variant_count, trigger_count) = store.stats();
            let pending = state
                .template_generator
                .as_ref()
                .map(|g| g.pending())
                .unwrap_or(0);
            Json(serde_json::json!({
                "enabled": true,
                "trigger_count": trigger_count,
                "variant_count": variant_count,
                "pending_generations": pending,
                "generator_active": state.template_generator.is_some(),
                "dirty": store.is_dirty()
            }))
        }
        None => Json(serde_json::json!({
            "enabled": false,
            "trigger_count": 0,
            "variant_count": 0
        })),
    }
}

/// POST /v1/templates/import
async fn handle_import(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ImportRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let store_lock = match &state.template_store {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": "template store is not enabled"
                })),
            );
        }
    };

    // Convert ImportVariant -> StoredVariant
    let now = Utc::now();
    let data: HashMap<String, Vec<StoredVariant>> = req
        .templates
        .into_iter()
        .map(|(trigger, variants)| {
            let stored: Vec<StoredVariant> = variants
                .into_iter()
                .map(|v| StoredVariant {
                    text: v.text,
                    personality: v.personality,
                    source_sim: v.source_sim,
                    channel: v.channel,
                    generated_at: now,
                    last_used: now,
                    use_count: 0,
                })
                .collect();
            (trigger, stored)
        })
        .collect();

    let total_incoming: usize = data.values().map(|v| v.len()).sum();

    let mut store = store_lock.write().await;
    store.import(data, &state.embedder).await;
    let (variant_count, trigger_count) = store.stats();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "imported": true,
            "incoming_variants": total_incoming,
            "total_variants": variant_count,
            "total_triggers": trigger_count
        })),
    )
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/templates/queue", post(handle_queue))
        .route("/v1/templates/lookup", get(handle_lookup))
        .route("/v1/templates/stats", get(handle_stats))
        .route("/v1/templates/import", post(handle_import))
}
