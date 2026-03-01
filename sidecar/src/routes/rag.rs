//! RAG endpoints: search and ingest.
//!
//! POST /v1/rag/search - Embeds a query and searches lore/memory/all.
//! POST /v1/rag/ingest - Adds a document to the memory collection.

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use axum::{extract::State, routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tracing::debug;

// ─── Search ────────────────────────────────────────────────────────

/// Request body for /v1/rag/search
#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    /// The search query text.
    pub query: String,
    /// Maximum number of results (default: 3).
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Collection to search: "lore", "memory", or "all" (default: "all").
    #[serde(default = "default_collection")]
    pub collection: String,
    /// Minimum cosine similarity threshold (default: 0.3).
    #[serde(default = "default_min_score")]
    pub min_score: f32,
}

fn default_top_k() -> usize {
    3
}

fn default_collection() -> String {
    "all".to_string()
}

fn default_min_score() -> f32 {
    0.3
}

/// Response body for /v1/rag/search
#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub query_embedding_ms: u64,
    pub search_ms: u64,
    pub total_results: usize,
}

/// A single search result.
#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub text: String,
    pub score: f32,
    pub collection: String,
    pub metadata: HashMap<String, serde_json::Value>,
}

async fn handle_search(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SearchRequest>,
) -> AppResult<Json<SearchResponse>> {
    if request.query.is_empty() {
        return Err(AppError::BadRequest(
            "Field 'query' is required and must be non-empty".to_string(),
        ));
    }

    let embedder = state.embedder.as_ref().ok_or_else(|| {
        AppError::Unavailable("Embedding model not loaded".to_string())
    })?;

    // Step 1: Embed the query (spawn_blocking for ONNX thread safety)
    let embed_start = Instant::now();
    let query_embedding = embedder
        .embed_async(request.query.clone())
        .await
        .map_err(|e| AppError::Internal(format!("Query embedding failed: {}", e)))?;
    let embed_ms = embed_start.elapsed().as_millis() as u64;

    // Step 2: Search the relevant collection(s)
    let search_start = Instant::now();
    let mut all_results = Vec::new();

    let search_lore = request.collection == "lore" || request.collection == "all";
    let search_memory = request.collection == "memory" || request.collection == "all";

    if search_lore {
        let lore_results = state.lore.search(&query_embedding, request.top_k, request.min_score);
        all_results.extend(lore_results.into_iter().map(|r| SearchResult {
            text: r.text,
            score: r.score,
            collection: r.collection,
            metadata: r.metadata,
        }));
    }

    if search_memory {
        let memory_results = state.memory.search(&query_embedding, request.top_k, request.min_score);
        all_results.extend(memory_results.into_iter().map(|r| SearchResult {
            text: r.text,
            score: r.score,
            collection: r.collection,
            metadata: r.metadata,
        }));
    }

    // If searching "all", sort merged results by score and take top_k
    if request.collection == "all" && all_results.len() > request.top_k {
        all_results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all_results.truncate(request.top_k);
    }

    let search_ms = search_start.elapsed().as_millis() as u64;
    let total = all_results.len();

    debug!(
        "RAG search for '{}' ({}): embed={}ms, search={}ms, results={}",
        request.query, request.collection, embed_ms, search_ms, total
    );

    Ok(Json(SearchResponse {
        results: all_results,
        query_embedding_ms: embed_ms,
        search_ms,
        total_results: total,
    }))
}

// ─── Ingest ────────────────────────────────────────────────────────

/// Request body for POST /v1/rag/ingest
#[derive(Debug, Deserialize)]
pub struct IngestRequest {
    /// The document text to ingest.
    pub text: String,
    /// The collection to ingest into. Only "memory" is allowed.
    #[serde(default = "default_memory_collection")]
    pub collection: String,
    /// Optional metadata to attach.
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

fn default_memory_collection() -> String {
    "memory".to_string()
}

/// Response body for POST /v1/rag/ingest
#[derive(Debug, Serialize)]
pub struct IngestResponse {
    pub id: String,
    pub collection: String,
    pub embed_ms: u64,
}

async fn handle_ingest(
    State(state): State<Arc<AppState>>,
    Json(request): Json<IngestRequest>,
) -> AppResult<Json<IngestResponse>> {
    if request.text.is_empty() {
        return Err(AppError::BadRequest(
            "Field 'text' is required and must be non-empty".to_string(),
        ));
    }

    // Only "memory" collection is writable
    if request.collection != "memory" {
        return Err(AppError::BadRequest(format!(
            "Collection '{}' is read-only. Only 'memory' accepts ingestion.",
            request.collection
        )));
    }

    let embedder = state.embedder.as_ref().ok_or_else(|| {
        AppError::Unavailable("Embedding model not loaded".to_string())
    })?;

    // Embed the document (spawn_blocking for ONNX thread safety)
    let embed_start = Instant::now();
    let embedding = embedder
        .embed_async(request.text.clone())
        .await
        .map_err(|e| AppError::Internal(format!("Embedding failed: {}", e)))?;
    let embed_ms = embed_start.elapsed().as_millis() as u64;

    // Ingest into memory store (no lock needed, VectorDB handles concurrency)
    let id = state.memory.ingest(request.text, embedding, request.metadata);

    debug!("Ingested memory '{}' in {}ms", id, embed_ms);

    Ok(Json(IngestResponse {
        id,
        collection: "memory".to_string(),
        embed_ms,
    }))
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/rag/search", post(handle_search))
        .route("/v1/rag/ingest", post(handle_ingest))
}
