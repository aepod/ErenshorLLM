//! Vector-backed personality store for semantic personality search.
//!
//! Wraps a VectorStoreAdapter loaded from a pre-built `.ruvector` database
//! containing personality archetype and phrase embeddings. Provides
//! personality-aware semantic search at runtime.
//!
//! This is distinct from `llm::personality::PersonalityStore` which is a
//! HashMap-based store for prompt construction. This store enables vector
//! similarity search across personality traits and example phrases.

use std::collections::HashMap;
use std::path::Path;
use tracing::{info, warn};

use crate::intelligence::vector_store::{AdapterConfig, VectorStoreAdapter};

/// A search result from the personality vector store.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PersonalitySearchResult {
    pub text: String,
    pub score: f32,
    pub name: String,
    pub entry_type: String,
    pub collection: String,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Vector-backed personality store for semantic search over personality
/// archetypes and example phrases.
pub struct VectorPersonalityStore {
    adapter: Option<VectorStoreAdapter>,
}

impl VectorPersonalityStore {
    /// Create an empty (unloaded) personality store.
    pub fn empty() -> Self {
        Self { adapter: None }
    }

    /// Open a personality store from an existing .ruvector database file.
    pub fn open(ruvector_path: &Path, config: &AdapterConfig) -> Self {
        if ruvector_path.exists() {
            let adapter = VectorStoreAdapter::open_or_empty(ruvector_path, config);
            if adapter.is_loaded() {
                info!(
                    "Personality vector store loaded: {} entries",
                    adapter.len()
                );
                return Self {
                    adapter: Some(adapter),
                };
            }
        }

        warn!(
            "Personality vector store not found at {:?}",
            ruvector_path
        );
        Self::empty()
    }

    /// Whether the personality store is loaded and has entries.
    pub fn is_loaded(&self) -> bool {
        self.adapter.as_ref().map_or(false, |a| a.is_loaded())
    }

    /// Number of entries in the store.
    pub fn entry_count(&self) -> usize {
        self.adapter.as_ref().map_or(0, |a| a.len())
    }

    /// Search all personality entries (archetypes + phrases).
    pub fn search(
        &self,
        query_embedding: &[f32],
        top_k: usize,
        min_score: f32,
    ) -> Vec<PersonalitySearchResult> {
        let adapter = match &self.adapter {
            Some(a) => a,
            None => return Vec::new(),
        };

        adapter
            .search(query_embedding, top_k, min_score)
            .into_iter()
            .map(|r| {
                let name = r
                    .metadata
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let entry_type = r
                    .metadata
                    .get("entry_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let text = r
                    .metadata
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                PersonalitySearchResult {
                    text,
                    score: r.score,
                    name,
                    entry_type,
                    collection: "personality".to_string(),
                    metadata: r.metadata,
                }
            })
            .collect()
    }

    /// Search personality entries filtered to a specific SimPlayer name.
    ///
    /// Performs a broader search then filters results by name. This is
    /// useful for finding the most relevant personality traits for a
    /// specific character.
    pub fn search_by_name(
        &self,
        name: &str,
        query_embedding: &[f32],
        top_k: usize,
    ) -> Vec<PersonalitySearchResult> {
        let adapter = match &self.adapter {
            Some(a) => a,
            None => return Vec::new(),
        };

        let name_lower = name.to_lowercase();

        // Search a broader set and filter by name
        let broad_k = top_k * 10;
        adapter
            .search(query_embedding, broad_k, 0.0)
            .into_iter()
            .filter_map(|r| {
                let entry_name = r
                    .metadata
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if entry_name.to_lowercase() != name_lower {
                    return None;
                }

                let entry_type = r
                    .metadata
                    .get("entry_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let text = r
                    .metadata
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                Some(PersonalitySearchResult {
                    text,
                    score: r.score,
                    name: entry_name.to_string(),
                    entry_type,
                    collection: "personality".to_string(),
                    metadata: r.metadata,
                })
            })
            .take(top_k)
            .collect()
    }
}
