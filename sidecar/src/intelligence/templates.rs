//! Response template store backed by ruvector-core VectorDB.
//!
//! Loads pre-embedded response templates and provides semantic search with
//! HNSW indexing. Falls back to JSON loading if .ruvector file is not found
//! (backward compatibility).

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
use tracing::{info, warn};

use crate::intelligence::embedder::EmbeddingEngine;
use crate::intelligence::vector_store::{AdapterConfig, VectorStoreAdapter};

/// A single response template with its metadata.
#[derive(Debug, Clone)]
pub struct ResponseTemplate {
    pub id: String,
    pub text: String,
    pub category: String,
    pub context_tags: Vec<String>,
    pub zone_affinity: Vec<String>,
    pub personality_affinity: Vec<String>,
    pub relationship_min: f32,
    pub relationship_max: f32,
    pub channel: Vec<String>,
    pub priority: f32,
    /// Embedding stored only for JSON fallback path; VectorDB stores it internally.
    pub embedding: Vec<f32>,
}

/// A candidate result from semantic search (before re-ranking).
#[derive(Debug, Clone)]
pub struct TemplateCandidate {
    pub template: ResponseTemplate,
    pub semantic_score: f32,
}

/// The response template store. Wraps VectorStoreAdapter for HNSW search,
/// with fallback to in-memory brute-force for JSON-loaded data.
pub struct ResponseStore {
    /// VectorDB-backed store (primary path).
    adapter: Option<VectorStoreAdapter>,
    /// In-memory fallback templates (JSON backward compat).
    fallback_templates: Vec<ResponseTemplate>,
}

impl ResponseStore {
    /// Create an empty (unloaded) store.
    pub fn empty() -> Self {
        Self {
            adapter: None,
            fallback_templates: Vec::new(),
        }
    }

    /// Open a response store from a .ruvector database file.
    /// Falls back to loading .json if the .ruvector file doesn't exist.
    pub fn open(ruvector_path: &Path, json_fallback_path: &Path, config: &AdapterConfig) -> Self {
        // Try .ruvector first
        if ruvector_path.exists() {
            let adapter = VectorStoreAdapter::open_or_empty(ruvector_path, config);
            if adapter.is_loaded() {
                info!("Response templates loaded from VectorDB: {} entries", adapter.len());
                return Self {
                    adapter: Some(adapter),
                    fallback_templates: Vec::new(),
                };
            }
        }

        // Fall back to JSON
        if json_fallback_path.exists() {
            info!("Responses .ruvector not found, falling back to JSON: {:?}", json_fallback_path);
            match Self::load_json_fallback(json_fallback_path) {
                Ok(store) => return store,
                Err(e) => {
                    warn!("Failed to load JSON response fallback: {}", e);
                }
            }
        }

        warn!("No response templates found at {:?} or {:?}", ruvector_path, json_fallback_path);
        Self::empty()
    }

    /// Load from a serialized JSON file (backward compatibility).
    fn load_json_fallback(path: &Path) -> Result<Self> {
        let contents =
            std::fs::read_to_string(path).context("Failed to read response index file")?;

        let entries: Vec<SerializedTemplate> =
            serde_json::from_str(&contents).context("Failed to parse response index JSON")?;

        let templates: Vec<ResponseTemplate> = entries
            .into_iter()
            .map(|e| ResponseTemplate {
                id: e.id,
                text: e.text,
                category: e.category,
                context_tags: e.context_tags,
                zone_affinity: e.zone_affinity,
                personality_affinity: e.personality_affinity,
                relationship_min: e.relationship_min,
                relationship_max: e.relationship_max,
                channel: e.channel,
                priority: e.priority,
                embedding: e.embedding,
            })
            .collect();

        info!(
            "Loaded {} response templates from JSON fallback {:?}",
            templates.len(),
            path
        );

        Ok(Self {
            adapter: None,
            fallback_templates: templates,
        })
    }

    /// Whether the store is loaded and has entries.
    pub fn is_loaded(&self) -> bool {
        if let Some(ref adapter) = self.adapter {
            adapter.is_loaded()
        } else {
            !self.fallback_templates.is_empty()
        }
    }

    /// Number of templates in the store.
    pub fn entry_count(&self) -> usize {
        if let Some(ref adapter) = self.adapter {
            adapter.len()
        } else {
            self.fallback_templates.len()
        }
    }

    /// Search for the top-k semantically similar templates.
    ///
    /// Returns candidates sorted by semantic similarity (descending).
    /// The caller should then apply context re-ranking via the Ranker.
    pub fn search(
        &self,
        query_embedding: &[f32],
        top_k: usize,
        min_score: f32,
    ) -> Vec<TemplateCandidate> {
        if let Some(ref adapter) = self.adapter {
            // HNSW path via VectorDB
            adapter
                .search(query_embedding, top_k, min_score)
                .into_iter()
                .filter_map(|r| {
                    // Reconstruct ResponseTemplate from VectorDB metadata
                    let template = Self::template_from_metadata(&r.id, &r.metadata);
                    template.map(|t| TemplateCandidate {
                        template: t,
                        semantic_score: r.score,
                    })
                })
                .collect()
        } else {
            // Brute-force fallback for JSON-loaded data
            self.search_fallback(query_embedding, top_k, min_score)
        }
    }

    /// Reconstruct a ResponseTemplate from VectorDB metadata.
    fn template_from_metadata(id: &str, metadata: &HashMap<String, serde_json::Value>) -> Option<ResponseTemplate> {
        let text = metadata.get("text")?.as_str()?.to_string();
        let category = metadata.get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("catchall")
            .to_string();

        Some(ResponseTemplate {
            id: id.to_string(),
            text,
            category,
            context_tags: json_str_array(metadata.get("context_tags")),
            zone_affinity: json_str_array(metadata.get("zone_affinity")),
            personality_affinity: json_str_array(metadata.get("personality_affinity")),
            relationship_min: metadata.get("relationship_min")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0) as f32,
            relationship_max: metadata.get("relationship_max")
                .and_then(|v| v.as_f64())
                .unwrap_or(10.0) as f32,
            channel: json_str_array(metadata.get("channel")),
            priority: metadata.get("priority")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0) as f32,
            embedding: Vec::new(), // Not needed when using VectorDB
        })
    }

    /// Brute-force cosine similarity search (JSON fallback path).
    fn search_fallback(
        &self,
        query_embedding: &[f32],
        top_k: usize,
        min_score: f32,
    ) -> Vec<TemplateCandidate> {
        if self.fallback_templates.is_empty() {
            return Vec::new();
        }

        let mut scored: Vec<(usize, f32)> = self
            .fallback_templates
            .iter()
            .enumerate()
            .map(|(i, tmpl)| {
                let score =
                    EmbeddingEngine::cosine_similarity(query_embedding, &tmpl.embedding);
                (i, score)
            })
            .filter(|(_, score)| *score >= min_score)
            .collect();

        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        scored
            .into_iter()
            .take(top_k)
            .map(|(i, score)| TemplateCandidate {
                template: self.fallback_templates[i].clone(),
                semantic_score: score,
            })
            .collect()
    }

    /// Get a random fallback template (from catchall category, or first available).
    pub fn fallback_template(&self) -> Option<ResponseTemplate> {
        if let Some(ref adapter) = self.adapter {
            // For VectorDB path, we don't have all templates in memory.
            // Search with a generic query to find a catchall template.
            // As a simple fallback, return None and let the caller handle it.
            // This case is rare -- only triggered when NO templates match at all.
            None
        } else {
            // JSON fallback path -- templates are in memory
            let catchalls: Vec<&ResponseTemplate> = self
                .fallback_templates
                .iter()
                .filter(|t| t.category == "catchall")
                .collect();

            if !catchalls.is_empty() {
                let idx = rand::random::<usize>() % catchalls.len();
                Some(catchalls[idx].clone())
            } else {
                self.fallback_templates.first().cloned()
            }
        }
    }
}

/// Extract a string array from a JSON value.
fn json_str_array(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Serialized format for templates (JSON index file).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct SerializedTemplate {
    pub id: String,
    pub text: String,
    pub category: String,
    pub context_tags: Vec<String>,
    pub zone_affinity: Vec<String>,
    pub personality_affinity: Vec<String>,
    pub relationship_min: f32,
    pub relationship_max: f32,
    pub channel: Vec<String>,
    pub priority: f32,
    pub embedding: Vec<f32>,
}

/// The raw template format as authored in JSON files (before embedding).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RawTemplateFile {
    pub category: String,
    pub templates: Vec<RawTemplate>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RawTemplate {
    pub id: String,
    pub text: String,
    pub context_tags: Vec<String>,
    #[serde(default)]
    pub zone_affinity: Vec<String>,
    #[serde(default)]
    pub personality_affinity: Vec<String>,
    #[serde(default = "default_relationship_min")]
    pub relationship_min: f32,
    #[serde(default = "default_relationship_max")]
    pub relationship_max: f32,
    #[serde(default)]
    pub channel: Vec<String>,
    #[serde(default = "default_priority")]
    pub priority: f32,
}

fn default_relationship_min() -> f32 {
    0.0
}

fn default_relationship_max() -> f32 {
    10.0
}

fn default_priority() -> f32 {
    1.0
}

/// Save templates to a JSON index file (builder output, backward compat).
pub fn save_template_index(templates: &[ResponseTemplate], path: &Path) -> Result<()> {
    let serialized: Vec<SerializedTemplate> = templates
        .iter()
        .map(|t| SerializedTemplate {
            id: t.id.clone(),
            text: t.text.clone(),
            category: t.category.clone(),
            context_tags: t.context_tags.clone(),
            zone_affinity: t.zone_affinity.clone(),
            personality_affinity: t.personality_affinity.clone(),
            relationship_min: t.relationship_min,
            relationship_max: t.relationship_max,
            channel: t.channel.clone(),
            priority: t.priority,
            embedding: t.embedding.clone(),
        })
        .collect();

    let json =
        serde_json::to_string(&serialized).context("Failed to serialize template index")?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(path, json).context("Failed to write template index file")?;

    info!("Saved {} templates to {:?}", templates.len(), path);
    Ok(())
}
