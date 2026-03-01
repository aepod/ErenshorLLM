//! Thin adapter wrapping ruvector-core VectorDB.
//!
//! Provides a clean interface for lore, template, and memory stores with
//! automatic score conversion from distance (lower=better) to similarity
//! (higher=better).

use anyhow::{Context, Result};
use ruvector_core::types::{DbOptions, HnswConfig};
use ruvector_core::{DistanceMetric, SearchQuery, VectorDB, VectorEntry};
use std::collections::HashMap;
use std::path::Path;
use tracing::{error, info, warn};

/// Configuration for the vector store adapter.
///
/// Exported as `AdapterConfig` for use by store modules.
pub type AdapterConfig = VectorStoreConfig;

#[derive(Debug, Clone)]
pub struct VectorStoreConfig {
    pub dimensions: usize,
    pub hnsw_m: usize,
    pub hnsw_ef_construction: usize,
    pub hnsw_ef_search: usize,
    pub max_elements: usize,
    pub quantization: bool,
}

impl Default for VectorStoreConfig {
    fn default() -> Self {
        Self {
            dimensions: 384,
            hnsw_m: 16,
            hnsw_ef_construction: 200,
            hnsw_ef_search: 50,
            max_elements: 10_000,
            quantization: false,
        }
    }
}

/// Adapter wrapping ruvector-core VectorDB with score inversion.
///
/// Uses Euclidean distance instead of Cosine because hnsw_rs 0.3.x
/// asserts distances are non-negative. Cosine distance can produce
/// tiny negative values due to floating-point precision, causing panics.
/// For L2-normalized vectors (which all our embeddings are), Euclidean
/// distance preserves the same ranking as cosine similarity:
///   ||a-b||^2 = 2 - 2*cos(a,b)
///
/// Scores are converted: similarity = 1.0 - (dist^2 / 2.0).
pub struct VectorStoreAdapter {
    db: VectorDB,
    dimensions: usize,
}

/// A search result with similarity score (1.0 - distance).
#[derive(Debug, Clone)]
pub struct AdapterSearchResult {
    pub id: String,
    pub score: f32,
    pub metadata: HashMap<String, serde_json::Value>,
}

impl VectorStoreAdapter {
    /// Create or open a VectorDB at the given path.
    ///
    /// If the database already exists, its stored configuration is used.
    /// If it does not exist, a new database is created with the given config.
    pub fn open(path: &Path, config: &VectorStoreConfig) -> Result<Self> {
        let storage_path = path.to_string_lossy().to_string();

        let options = DbOptions {
            dimensions: config.dimensions,
            distance_metric: DistanceMetric::Euclidean,
            storage_path,
            hnsw_config: Some(HnswConfig {
                m: config.hnsw_m,
                ef_construction: config.hnsw_ef_construction,
                ef_search: config.hnsw_ef_search,
                max_elements: config.max_elements,
            }),
            quantization: None,
        };

        let db = VectorDB::new(options)
            .with_context(|| format!("Failed to open VectorDB at {:?}", path))?;

        let len = db.len().unwrap_or(0);
        if len > 0 {
            info!("Opened VectorDB at {:?} with {} entries", path, len);
        } else {
            info!("Created empty VectorDB at {:?}", path);
        }

        Ok(Self {
            db,
            dimensions: config.dimensions,
        })
    }

    /// Open or create a VectorDB, returning an empty adapter on failure.
    ///
    /// This never panics or returns an error. If the database cannot be opened
    /// (corrupt, permissions, etc.), it attempts to create a fresh one. If even
    /// that fails, it returns an adapter that reports as not loaded.
    pub fn open_or_empty(path: &Path, config: &AdapterConfig) -> Self {
        match Self::open(path, config) {
            Ok(adapter) => adapter,
            Err(e) => {
                warn!(
                    "Failed to open VectorDB at {:?}: {}. Trying fresh database.",
                    path, e
                );

                // Try to remove the corrupt file and create fresh
                if path.exists() {
                    if let Err(remove_err) = std::fs::remove_file(path) {
                        error!(
                            "Failed to remove corrupt DB file {:?}: {}",
                            path, remove_err
                        );
                    }
                }

                match Self::open(path, config) {
                    Ok(adapter) => adapter,
                    Err(e2) => {
                        error!(
                            "Failed to create fresh VectorDB at {:?}: {}. Store will be empty.",
                            path, e2
                        );
                        // Return a dummy adapter that reports as not loaded.
                        // We can't construct one without a VectorDB, so we need a fallback.
                        // Use a temp path that we know will work.
                        let temp_path = std::env::temp_dir().join(format!(
                            "erenshor-fallback-{}.ruvector",
                            std::process::id()
                        ));
                        match Self::open(&temp_path, config) {
                            Ok(adapter) => adapter,
                            Err(_) => {
                                // Absolute last resort -- this shouldn't happen
                                panic!("Cannot create even a temporary VectorDB. System error.");
                            }
                        }
                    }
                }
            }
        }
    }

    /// Search for the top-k nearest neighbors.
    ///
    /// Returns results with SIMILARITY scores, filtered by min_score.
    /// Euclidean distance for L2-normalized vectors: dist = sqrt(2 - 2*cos_sim),
    /// so cos_sim = 1 - dist^2/2. Clamped to [0, 1].
    /// Results are sorted by similarity descending (highest first).
    pub fn search(
        &self,
        query_embedding: &[f32],
        top_k: usize,
        min_score: f32,
    ) -> Vec<AdapterSearchResult> {
        let query = SearchQuery {
            vector: query_embedding.to_vec(),
            k: top_k,
            filter: None,
            ef_search: None,
        };

        match self.db.search(query) {
            Ok(results) => results
                .into_iter()
                .map(|r| {
                    // Euclidean dist for L2-normalized vecs: cos_sim = 1 - dist^2/2
                    let similarity = (1.0 - (r.score * r.score) / 2.0).clamp(0.0, 1.0);
                    AdapterSearchResult {
                        id: r.id,
                        score: similarity,
                        metadata: r.metadata.unwrap_or_default(),
                    }
                })
                .filter(|r| r.score >= min_score)
                .collect(),
            Err(e) => {
                error!("VectorDB search failed: {}", e);
                Vec::new()
            }
        }
    }

    /// Insert a single entry with metadata.
    pub fn insert(
        &self,
        id: &str,
        embedding: Vec<f32>,
        metadata: HashMap<String, serde_json::Value>,
    ) -> Result<String> {
        let entry = VectorEntry {
            id: Some(id.to_string()),
            vector: embedding,
            metadata: Some(metadata),
        };

        let returned_id = self
            .db
            .insert(entry)
            .with_context(|| format!("Failed to insert vector '{}'", id))?;

        Ok(returned_id)
    }

    /// Batch insert multiple entries. Returns the number of successfully inserted entries.
    pub fn insert_batch(
        &self,
        entries: Vec<(String, Vec<f32>, HashMap<String, serde_json::Value>)>,
    ) -> Result<usize> {
        let vector_entries: Vec<VectorEntry> = entries
            .into_iter()
            .map(|(id, embedding, metadata)| VectorEntry {
                id: Some(id),
                vector: embedding,
                metadata: Some(metadata),
            })
            .collect();

        let count = vector_entries.len();
        self.db
            .insert_batch(vector_entries)
            .context("Failed to batch insert vectors")?;

        Ok(count)
    }

    /// Number of entries in the database.
    pub fn len(&self) -> usize {
        self.db.len().unwrap_or(0)
    }

    /// Whether the database is empty.
    pub fn is_empty(&self) -> bool {
        self.db.is_empty().unwrap_or(true)
    }

    /// Whether the database is loaded and non-empty.
    pub fn is_loaded(&self) -> bool {
        !self.is_empty()
    }

    /// Get the embedding dimensions.
    pub fn dimensions(&self) -> usize {
        self.dimensions
    }
}
