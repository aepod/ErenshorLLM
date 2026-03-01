//! Runtime conversation memory store backed by ruvector-core VectorDB.
//!
//! Stores and retrieves conversation memories (game events, dialog snippets)
//! with HNSW-based vector search. Persistence is handled automatically by
//! redb -- no explicit flush needed. Falls back to JSON loading for
//! backward compatibility.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{info, warn};

use crate::intelligence::embedder::EmbeddingEngine;
use crate::intelligence::vector_store::{AdapterConfig, VectorStoreAdapter};

/// A search result from the memory store.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MemorySearchResult {
    pub text: String,
    pub score: f32,
    pub collection: String,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// In-memory entry used for JSON fallback.
#[derive(Debug, Clone)]
struct FallbackMemoryEntry {
    id: String,
    text: String,
    embedding: Vec<f32>,
    metadata: HashMap<String, serde_json::Value>,
}

/// The runtime memory store. Wraps VectorStoreAdapter for HNSW search
/// and automatic redb persistence, with JSON fallback for existing data.
pub struct MemoryStore {
    /// VectorDB-backed store (primary path).
    adapter: Option<VectorStoreAdapter>,
    /// In-memory fallback entries (JSON backward compat).
    fallback_entries: Vec<FallbackMemoryEntry>,
    /// Monotonic ID counter for new entries.
    next_id: AtomicU64,
}

impl MemoryStore {
    /// Create an empty memory store.
    pub fn empty() -> Self {
        Self {
            adapter: None,
            fallback_entries: Vec::new(),
            next_id: AtomicU64::new(1),
        }
    }

    /// Open a memory store from a .ruvector database file.
    /// Falls back to loading .json if the .ruvector file doesn't exist.
    /// Creates a new database if neither exists.
    pub fn open(ruvector_path: &Path, json_fallback_path: &Path, config: &AdapterConfig) -> Self {
        // Try .ruvector first (or create new)
        let adapter = VectorStoreAdapter::open_or_empty(ruvector_path, config);
        let existing_count = adapter.len();

        if existing_count > 0 {
            info!("Memory loaded from VectorDB: {} entries", existing_count);
            return Self {
                next_id: AtomicU64::new(existing_count as u64 + 1),
                adapter: Some(adapter),
                fallback_entries: Vec::new(),
            };
        }

        // Check for JSON fallback data
        if json_fallback_path.exists() {
            info!("Memory .ruvector empty, loading JSON fallback: {:?}", json_fallback_path);
            match Self::load_json_into_adapter(&adapter, json_fallback_path) {
                Ok(count) => {
                    info!("Migrated {} memory entries from JSON to VectorDB", count);
                    return Self {
                        next_id: AtomicU64::new(count as u64 + 1),
                        adapter: Some(adapter),
                        fallback_entries: Vec::new(),
                    };
                }
                Err(e) => {
                    warn!("Failed to migrate JSON memory: {}. Using JSON fallback.", e);
                    match Self::load_json_fallback(json_fallback_path) {
                        Ok(store) => return store,
                        Err(e2) => {
                            warn!("JSON fallback also failed: {}", e2);
                        }
                    }
                }
            }
        }

        // Empty but ready for new entries via VectorDB
        info!("Memory empty, starting fresh with VectorDB at {:?}", ruvector_path);
        Self {
            adapter: Some(adapter),
            fallback_entries: Vec::new(),
            next_id: AtomicU64::new(1),
        }
    }

    /// Load JSON entries and insert them into the VectorDB adapter.
    fn load_json_into_adapter(adapter: &VectorStoreAdapter, json_path: &Path) -> Result<usize> {
        let contents = std::fs::read_to_string(json_path)
            .context("Failed to read memory JSON file")?;

        let entries: Vec<SerializedMemoryEntry> = serde_json::from_str(&contents)
            .context("Failed to parse memory JSON")?;

        let count = entries.len();
        for entry in entries {
            let mut metadata = entry.metadata;
            metadata.insert("text".to_string(), serde_json::Value::String(entry.text));
            metadata.insert(
                "timestamp_ms".to_string(),
                serde_json::Value::Number(serde_json::Number::from(entry.timestamp_ms)),
            );

            adapter.insert(&entry.id, entry.embedding, metadata)?;
        }

        Ok(count)
    }

    /// Load from a serialized JSON file (pure fallback, no VectorDB).
    fn load_json_fallback(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .context("Failed to read memory file")?;

        let entries: Vec<SerializedMemoryEntry> = serde_json::from_str(&contents)
            .context("Failed to parse memory JSON")?;

        let max_id = entries
            .iter()
            .filter_map(|e| e.id.strip_prefix("mem_").and_then(|n| n.parse::<u64>().ok()))
            .max()
            .unwrap_or(0);

        let fallback_entries: Vec<FallbackMemoryEntry> = entries
            .into_iter()
            .map(|e| FallbackMemoryEntry {
                id: e.id,
                text: e.text,
                embedding: e.embedding,
                metadata: e.metadata,
            })
            .collect();

        let count = fallback_entries.len();
        info!("Loaded {} memory entries from JSON fallback {:?}", count, path);

        Ok(Self {
            adapter: None,
            fallback_entries,
            next_id: AtomicU64::new(max_id + 1),
        })
    }

    /// Whether the memory store has entries.
    pub fn is_loaded(&self) -> bool {
        if let Some(ref adapter) = self.adapter {
            adapter.is_loaded()
        } else {
            !self.fallback_entries.is_empty()
        }
    }

    /// Number of entries.
    pub fn entry_count(&self) -> usize {
        if let Some(ref adapter) = self.adapter {
            adapter.len()
        } else {
            self.fallback_entries.len()
        }
    }

    /// Ingest a new memory entry. Persisted immediately via redb.
    ///
    /// Note: This method takes &self (not &mut self) because VectorDB
    /// handles its own internal locking.
    pub fn ingest(
        &self,
        text: String,
        embedding: Vec<f32>,
        metadata: HashMap<String, serde_json::Value>,
    ) -> String {
        let seq = self.next_id.fetch_add(1, Ordering::Relaxed);
        let id = format!("mem_{:06}", seq);

        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut full_metadata = metadata;
        full_metadata.insert("text".to_string(), serde_json::Value::String(text));
        full_metadata.insert(
            "timestamp_ms".to_string(),
            serde_json::Value::Number(serde_json::Number::from(timestamp_ms)),
        );

        if let Some(ref adapter) = self.adapter {
            if let Err(e) = adapter.insert(&id, embedding, full_metadata) {
                warn!("Failed to insert memory entry {}: {}", id, e);
            }
        }
        // If no adapter (pure fallback mode), silently skip.
        // This shouldn't happen in practice since open() always creates an adapter.

        id
    }

    /// Search the memory store using a query embedding vector.
    pub fn search(
        &self,
        query_embedding: &[f32],
        top_k: usize,
        min_score: f32,
    ) -> Vec<MemorySearchResult> {
        if let Some(ref adapter) = self.adapter {
            // HNSW path via VectorDB
            adapter
                .search(query_embedding, top_k, min_score)
                .into_iter()
                .map(|r| MemorySearchResult {
                    text: r.metadata.get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    score: r.score,
                    collection: "memory".to_string(),
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
    ) -> Vec<MemorySearchResult> {
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
                MemorySearchResult {
                    text: entry.text.clone(),
                    score,
                    collection: "memory".to_string(),
                    metadata: entry.metadata.clone(),
                }
            })
            .collect()
    }
}

/// Serialized format for memory entries (JSON, backward compat).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct SerializedMemoryEntry {
    pub id: String,
    pub text: String,
    pub embedding: Vec<f32>,
    pub metadata: HashMap<String, serde_json::Value>,
    pub timestamp_ms: u64,
}
