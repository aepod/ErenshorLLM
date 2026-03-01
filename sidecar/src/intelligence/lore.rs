//! Lore index store backed by ruvector-core VectorDB.
//!
//! Loads a pre-built vector database of Erenshor lore passages and provides
//! HNSW-based semantic search. Falls back to JSON loading if .ruvector file
//! is not found but .json file exists (backward compatibility).

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
use tracing::{info, warn};

use crate::intelligence::embedder::EmbeddingEngine;
use crate::intelligence::vector_store::{AdapterConfig, VectorStoreAdapter};

/// A search result from the lore index.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LoreSearchResult {
    pub text: String,
    pub score: f32,
    pub collection: String,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// In-memory lore entry used by the builder and JSON fallback.
#[derive(Debug, Clone)]
pub struct LoreEntry {
    pub id: String,
    pub text: String,
    pub embedding: Vec<f32>,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// The lore store. Wraps VectorStoreAdapter for HNSW search,
/// with fallback to in-memory brute-force for JSON-loaded data.
pub struct LoreStore {
    /// VectorDB-backed store (primary path).
    adapter: Option<VectorStoreAdapter>,
    /// In-memory fallback entries (JSON backward compat).
    fallback_entries: Vec<LoreEntry>,
}

impl LoreStore {
    /// Create an empty (unloaded) lore store.
    pub fn empty() -> Self {
        Self {
            adapter: None,
            fallback_entries: Vec::new(),
        }
    }

    /// Open a lore store from a .ruvector database file.
    /// Falls back to loading .json if the .ruvector file doesn't exist.
    pub fn open(ruvector_path: &Path, json_fallback_path: &Path, config: &AdapterConfig) -> Self {
        // Try .ruvector first
        if ruvector_path.exists() {
            let adapter = VectorStoreAdapter::open_or_empty(ruvector_path, config);
            if adapter.is_loaded() {
                info!("Lore index loaded from VectorDB: {} entries", adapter.len());
                return Self {
                    adapter: Some(adapter),
                    fallback_entries: Vec::new(),
                };
            }
        }

        // Fall back to JSON
        if json_fallback_path.exists() {
            info!("Lore .ruvector not found, falling back to JSON: {:?}", json_fallback_path);
            match Self::load_json_fallback(json_fallback_path) {
                Ok(store) => return store,
                Err(e) => {
                    warn!("Failed to load JSON lore fallback: {}", e);
                }
            }
        }

        warn!("No lore index found at {:?} or {:?}", ruvector_path, json_fallback_path);
        Self::empty()
    }

    /// Load from a serialized JSON file (backward compatibility).
    fn load_json_fallback(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .context("Failed to read lore index file")?;

        let entries: Vec<SerializedLoreEntry> = serde_json::from_str(&contents)
            .context("Failed to parse lore index JSON")?;

        let lore_entries: Vec<LoreEntry> = entries
            .into_iter()
            .map(|e| LoreEntry {
                id: e.id,
                text: e.text,
                embedding: e.embedding,
                metadata: e.metadata,
            })
            .collect();

        info!("Loaded {} lore entries from JSON fallback {:?}", lore_entries.len(), path);

        Ok(Self {
            adapter: None,
            fallback_entries: lore_entries,
        })
    }

    /// Whether the lore index is loaded and has entries.
    pub fn is_loaded(&self) -> bool {
        if let Some(ref adapter) = self.adapter {
            adapter.is_loaded()
        } else {
            !self.fallback_entries.is_empty()
        }
    }

    /// Number of entries in the index.
    pub fn entry_count(&self) -> usize {
        if let Some(ref adapter) = self.adapter {
            adapter.len()
        } else {
            self.fallback_entries.len()
        }
    }

    /// Search the lore index using a query embedding vector.
    ///
    /// Returns the top-k results with similarity above min_score.
    pub fn search(
        &self,
        query_embedding: &[f32],
        top_k: usize,
        min_score: f32,
    ) -> Vec<LoreSearchResult> {
        if let Some(ref adapter) = self.adapter {
            // HNSW path via VectorDB
            adapter
                .search(query_embedding, top_k, min_score)
                .into_iter()
                .map(|r| LoreSearchResult {
                    text: r.metadata.get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    score: r.score,
                    collection: "lore".to_string(),
                    metadata: r.metadata,
                })
                .collect()
        } else {
            // Brute-force fallback for JSON-loaded data
            self.search_fallback(query_embedding, top_k, min_score)
        }
    }

    /// Brute-force cosine similarity search (JSON fallback path).
    fn search_fallback(
        &self,
        query_embedding: &[f32],
        top_k: usize,
        min_score: f32,
    ) -> Vec<LoreSearchResult> {
        if self.fallback_entries.is_empty() {
            return Vec::new();
        }

        let mut scored: Vec<(usize, f32)> = self
            .fallback_entries
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let score = EmbeddingEngine::cosine_similarity(query_embedding, &entry.embedding);
                (i, score)
            })
            .filter(|(_, score)| *score >= min_score)
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        scored
            .into_iter()
            .take(top_k)
            .map(|(i, score)| {
                let entry = &self.fallback_entries[i];
                LoreSearchResult {
                    text: entry.text.clone(),
                    score,
                    collection: "lore".to_string(),
                    metadata: entry.metadata.clone(),
                }
            })
            .collect()
    }
}

/// Serialized format for lore entries (JSON index file).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct SerializedLoreEntry {
    pub id: String,
    pub text: String,
    pub embedding: Vec<f32>,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Parse markdown lore files into passages.
///
/// Each file is split on `---` separators. The title (first `#` line) is
/// extracted as metadata. Passages shorter than 20 characters are discarded.
pub fn parse_lore_markdown(content: &str, category: &str, page: &str) -> Vec<(String, HashMap<String, serde_json::Value>)> {
    let parts: Vec<&str> = content.split("---").collect();
    let mut passages = Vec::new();

    for part in parts {
        let trimmed = part.trim();

        // Skip the title line and very short passages
        if trimmed.is_empty() || trimmed.len() < 20 || trimmed.starts_with('#') {
            continue;
        }

        let mut metadata = HashMap::new();
        metadata.insert("source".to_string(), serde_json::Value::String("erenshor-wiki".to_string()));
        metadata.insert("category".to_string(), serde_json::Value::String(category.to_string()));
        metadata.insert("page".to_string(), serde_json::Value::String(page.to_string()));

        passages.push((trimmed.to_string(), metadata));
    }

    passages
}

/// Save lore entries to a JSON index file (builder output, backward compat).
pub fn save_lore_index(entries: &[LoreEntry], path: &Path) -> Result<()> {
    let serialized: Vec<SerializedLoreEntry> = entries
        .iter()
        .map(|e| SerializedLoreEntry {
            id: e.id.clone(),
            text: e.text.clone(),
            embedding: e.embedding.clone(),
            metadata: e.metadata.clone(),
        })
        .collect();

    let json = serde_json::to_string_pretty(&serialized)
        .context("Failed to serialize lore index")?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(path, json)
        .context("Failed to write lore index file")?;

    info!("Saved {} lore entries to {:?}", entries.len(), path);
    Ok(())
}
