//! Cross-database search combining results from lore, personality, and memory stores.
//!
//! All three stores share the same embedding space (all-MiniLM-L6-v2, 384d),
//! so query vectors can be used across all stores. Results are tagged with
//! their source store for downstream prioritization.

use std::collections::HashMap;

use crate::intelligence::lore::LoreStore;
use crate::intelligence::memory::MemoryStore;
use crate::intelligence::personality_store::VectorPersonalityStore;

/// A search result tagged with its source store.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TaggedSearchResult {
    /// The matched text content.
    pub text: String,
    /// Similarity score (0.0 - 1.0, higher = more similar).
    pub score: f32,
    /// Source store: "lore", "personality", or "memory".
    pub source: String,
    /// Full metadata from the original store.
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Search all three vector stores and merge results by score.
///
/// Returns the top-k results across all stores, sorted by similarity
/// score descending. Each result is tagged with its source store.
///
/// This function searches each store independently with the same query
/// vector, then merges and re-sorts the combined results.
pub fn search_all_stores(
    query_embedding: &[f32],
    lore: &LoreStore,
    personality: &VectorPersonalityStore,
    memory: &MemoryStore,
    top_k: usize,
    min_score: f32,
) -> Vec<TaggedSearchResult> {
    let mut all_results: Vec<TaggedSearchResult> = Vec::new();

    // Search lore store
    let lore_results = lore.search(query_embedding, top_k, min_score);
    for r in lore_results {
        all_results.push(TaggedSearchResult {
            text: r.text,
            score: r.score,
            source: "lore".to_string(),
            metadata: r.metadata,
        });
    }

    // Search personality store
    let personality_results = personality.search(query_embedding, top_k, min_score);
    for r in personality_results {
        all_results.push(TaggedSearchResult {
            text: r.text,
            score: r.score,
            source: "personality".to_string(),
            metadata: r.metadata,
        });
    }

    // Search memory store
    let memory_results = memory.search(query_embedding, top_k, min_score);
    for r in memory_results {
        all_results.push(TaggedSearchResult {
            text: r.text,
            score: r.score,
            source: "memory".to_string(),
            metadata: r.metadata,
        });
    }

    // Sort by score descending
    all_results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Take top-k overall
    all_results.truncate(top_k);

    all_results
}

/// Search lore and personality stores only (no memory).
///
/// Useful for context assembly where memory is handled separately
/// with different parameters.
pub fn search_knowledge_stores(
    query_embedding: &[f32],
    lore: &LoreStore,
    personality: &VectorPersonalityStore,
    top_k: usize,
    min_score: f32,
) -> Vec<TaggedSearchResult> {
    let mut all_results: Vec<TaggedSearchResult> = Vec::new();

    let lore_results = lore.search(query_embedding, top_k, min_score);
    for r in lore_results {
        all_results.push(TaggedSearchResult {
            text: r.text,
            score: r.score,
            source: "lore".to_string(),
            metadata: r.metadata,
        });
    }

    let personality_results = personality.search(query_embedding, top_k, min_score);
    for r in personality_results {
        all_results.push(TaggedSearchResult {
            text: r.text,
            score: r.score,
            source: "personality".to_string(),
            metadata: r.metadata,
        });
    }

    all_results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    all_results.truncate(top_k);

    all_results
}
