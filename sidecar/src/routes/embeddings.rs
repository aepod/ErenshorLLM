//! POST /v1/embeddings endpoint.
//!
//! OpenAI-compatible embedding API. Accepts a string or array of strings
//! and returns 384-dimensional f32 vectors.

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{extract::State, routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tracing::debug;

/// Request body for /v1/embeddings
#[derive(Debug, Deserialize)]
pub struct EmbeddingRequest {
    /// The model name (ignored -- we only have one model).
    #[serde(default = "default_model")]
    pub model: String,
    /// A string or array of strings to embed.
    pub input: EmbeddingInput,
}

fn default_model() -> String {
    "all-minilm-l6-v2".to_string()
}

/// Input can be a single string or array of strings.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum EmbeddingInput {
    Single(String),
    Batch(Vec<String>),
}

/// Response body for /v1/embeddings (OpenAI-compatible format).
#[derive(Debug, Serialize)]
pub struct EmbeddingResponse {
    pub object: String,
    pub data: Vec<EmbeddingData>,
    pub model: String,
    pub usage: EmbeddingUsage,
}

#[derive(Debug, Serialize)]
pub struct EmbeddingData {
    pub object: String,
    pub embedding: Vec<f32>,
    pub index: usize,
}

#[derive(Debug, Serialize)]
pub struct EmbeddingUsage {
    pub prompt_tokens: usize,
    pub total_tokens: usize,
}

async fn handle_embeddings(
    State(state): State<Arc<AppState>>,
    Json(request): Json<EmbeddingRequest>,
) -> AppResult<Json<EmbeddingResponse>> {
    let embedder = state.embedder.as_ref().ok_or_else(|| {
        AppError::Unavailable("Embedding model not loaded".to_string())
    })?;

    let start = Instant::now();

    let texts: Vec<String> = match request.input {
        EmbeddingInput::Single(text) => {
            if text.is_empty() {
                return Err(AppError::BadRequest(
                    "Field 'input' must be a non-empty string or array of strings".to_string(),
                ));
            }
            vec![text]
        }
        EmbeddingInput::Batch(texts) => {
            if texts.is_empty() {
                return Err(AppError::BadRequest(
                    "Field 'input' must be a non-empty string or array of strings".to_string(),
                ));
            }
            texts
        }
    };

    let mut data = Vec::with_capacity(texts.len());
    let mut total_tokens = 0;

    for (i, text) in texts.iter().enumerate() {
        let embedding = embedder
            .embed_async(text.clone())
            .await
            .map_err(|e| AppError::Internal(format!("Embedding failed: {}", e)))?;

        total_tokens += embedder.token_count(text);

        data.push(EmbeddingData {
            object: "embedding".to_string(),
            embedding,
            index: i,
        });
    }

    let elapsed = start.elapsed();
    debug!(
        "Embedded {} texts in {:?} ({} tokens)",
        data.len(),
        elapsed,
        total_tokens
    );

    Ok(Json(EmbeddingResponse {
        object: "list".to_string(),
        data,
        model: "all-minilm-l6-v2".to_string(),
        usage: EmbeddingUsage {
            prompt_tokens: total_tokens,
            total_tokens,
        },
    }))
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/v1/embeddings", post(handle_embeddings))
}
