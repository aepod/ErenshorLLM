//! Personality vector database builder.
//!
//! Reads personality JSON files from a directory, creates embeddings for
//! archetype descriptions and example phrases, and writes them to a
//! `.ruvector` database for semantic personality search at runtime.
//!
//! Expected input format (per JSON file):
//! ```json
//! {
//!   "name": "Fugs",
//!   "archetype": "Level 3 General - Absolute gremlin troll",
//!   "tone": "Chaotic and mischievous",
//!   "vocabulary": ["lmao", "yeet"],
//!   "speech_patterns": ["Types in all lowercase"],
//!   "knowledge_areas": ["Meme culture"],
//!   "quirks": ["Pulls extra mobs on purpose"],
//!   "example_phrases": ["lmao watch this"],
//!   "personality_type": 3,
//!   "chat_modifiers": { ... },
//!   "behavioral_attributes": { ... },
//!   "special_flags": { "rival": false, "is_gm_character": false },
//!   "guild_affinity": null
//! }
//! ```
//!
//! Output vectors:
//! - One per personality: archetype + tone + personality_type + guild affinity description
//! - One per example_phrase: for phrase-matching

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, info, warn};

use crate::intelligence::embedder::EmbeddingEngine;
use crate::intelligence::vector_store::{VectorStoreAdapter, VectorStoreConfig};

/// A personality entry parsed from JSON (reuses the existing Personality struct
/// but we keep it decoupled here to avoid circular deps with llm::personality).
#[derive(Debug, serde::Deserialize)]
struct PersonalityJson {
    name: String,
    #[serde(default)]
    archetype: String,
    #[serde(default)]
    tone: String,
    #[serde(default)]
    vocabulary: Vec<String>,
    #[serde(default)]
    speech_patterns: Vec<String>,
    #[serde(default)]
    knowledge_areas: Vec<String>,
    #[serde(default)]
    quirks: Vec<String>,
    #[serde(default)]
    example_phrases: Vec<String>,
    /// Personality type: 1=Nice, 2=Tryhard, 3=Mean, 5=Neutral
    #[serde(default = "default_personality_type")]
    personality_type: u8,
    /// Chat modifier flags (types_in_all_caps, typo_rate, etc.)
    #[serde(default)]
    chat_modifiers: Option<serde_json::Value>,
    /// Behavioral attributes (lore_chase, gear_chase, etc.)
    #[serde(default)]
    behavioral_attributes: Option<BehavioralAttributes>,
    /// Special flags (rival, is_gm_character)
    #[serde(default)]
    special_flags: Option<SpecialFlags>,
    /// Guild affinity (null or "friends_club")
    #[serde(default)]
    guild_affinity: Option<String>,
}

fn default_personality_type() -> u8 {
    5
}

#[derive(Debug, serde::Deserialize, Default)]
struct BehavioralAttributes {
    #[serde(default = "default_chase")]
    lore_chase: u8,
    #[serde(default = "default_chase")]
    gear_chase: u8,
    #[serde(default = "default_chase")]
    social_chase: u8,
    #[serde(default)]
    troublemaker: u8,
    #[serde(default = "default_chase")]
    dedication_level: u8,
    #[serde(default = "default_greed")]
    greed: f32,
    #[serde(default)]
    caution: bool,
    #[serde(default = "default_patience")]
    patience: u32,
}

fn default_chase() -> u8 {
    5
}

fn default_greed() -> f32 {
    1.0
}

fn default_patience() -> u32 {
    3000
}

#[derive(Debug, serde::Deserialize, Default)]
struct SpecialFlags {
    #[serde(default)]
    rival: bool,
    #[serde(default)]
    is_gm_character: bool,
}

/// Map personality_type integer to human-readable name for embedding text.
fn personality_type_name(pt: u8) -> &'static str {
    match pt {
        1 => "Nice",
        2 => "Tryhard",
        3 => "Mean",
        5 => "Neutral",
        _ => "Unknown",
    }
}

/// Build a personality vector database from JSON files.
///
/// For each personality file:
/// - Creates one "archetype" vector from the combined archetype + tone + type text
/// - Creates one "phrase" vector per example_phrase
///
/// Files named `_default.json` are skipped.
/// Files named `simplayerList.md` are skipped.
pub fn build_personality_index(
    input_dir: &Path,
    output_path: &Path,
    embedder: &EmbeddingEngine,
) -> Result<()> {
    debug!(
        "Building personality index from {:?} -> {:?}",
        input_dir, output_path
    );

    if !input_dir.exists() {
        anyhow::bail!("Personality directory {:?} does not exist", input_dir);
    }

    // Collect all JSON files
    let mut json_files: Vec<std::path::PathBuf> = std::fs::read_dir(input_dir)
        .with_context(|| format!("Failed to read personality directory {:?}", input_dir))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let ext_ok = p.extension().map_or(false, |e| e == "json");
            let name = p.file_stem().and_then(|n| n.to_str()).unwrap_or("");
            let skip = name == "_default" || name == "simplayerList";
            ext_ok && !skip
        })
        .collect();

    json_files.sort();

    debug!("Found {} personality files", json_files.len());

    // Parse all personalities
    let mut personalities: Vec<PersonalityJson> = Vec::new();
    for path in &json_files {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {:?}", path))?;

        match serde_json::from_str::<PersonalityJson>(&content) {
            Ok(p) => personalities.push(p),
            Err(e) => {
                warn!("Skipping malformed personality file {:?}: {}", path, e);
            }
        }
    }

    debug!("Parsed {} personalities", personalities.len());

    // Prepare entries: (id, embedding, metadata)
    let mut entries: Vec<(String, Vec<f32>, HashMap<String, serde_json::Value>)> = Vec::new();
    let mut archetype_count = 0;
    let mut phrase_count = 0;
    let mut rival_count = 0;

    for personality in &personalities {
        let name_lower = personality.name.to_lowercase();
        let is_rival = personality.special_flags.as_ref().map_or(false, |f| f.rival);
        let type_name = personality_type_name(personality.personality_type);

        if is_rival {
            rival_count += 1;
        }

        // 1. Archetype vector: combine archetype + tone + personality type + guild affinity
        let guild_suffix = if is_rival {
            " Friends' Club rival."
        } else {
            ""
        };

        let archetype_text = format!(
            "{} - {} {}, {}. They know about: {}. Speech patterns: {}{}",
            personality.name,
            type_name,
            personality.archetype,
            personality.tone,
            personality.knowledge_areas.join(", "),
            personality.speech_patterns.join(". "),
            guild_suffix,
        );

        match embedder.embed(&archetype_text) {
            Ok(embedding) => {
                let mut metadata = HashMap::new();
                metadata.insert(
                    "source".to_string(),
                    serde_json::Value::String("personality".to_string()),
                );
                metadata.insert(
                    "entry_type".to_string(),
                    serde_json::Value::String("archetype".to_string()),
                );
                metadata.insert(
                    "name".to_string(),
                    serde_json::Value::String(personality.name.clone()),
                );
                metadata.insert(
                    "archetype".to_string(),
                    serde_json::Value::String(personality.archetype.clone()),
                );
                metadata.insert(
                    "tone".to_string(),
                    serde_json::Value::String(personality.tone.clone()),
                );
                metadata.insert(
                    "text".to_string(),
                    serde_json::Value::String(archetype_text.clone()),
                );
                // New fields in metadata
                metadata.insert(
                    "personality_type".to_string(),
                    serde_json::Value::Number(personality.personality_type.into()),
                );
                metadata.insert(
                    "personality_type_name".to_string(),
                    serde_json::Value::String(type_name.to_string()),
                );
                metadata.insert(
                    "rival".to_string(),
                    serde_json::Value::Bool(is_rival),
                );
                if let Some(ref guild) = personality.guild_affinity {
                    metadata.insert(
                        "guild_affinity".to_string(),
                        serde_json::Value::String(guild.clone()),
                    );
                }
                // Behavioral attributes as metadata
                if let Some(ref attrs) = personality.behavioral_attributes {
                    metadata.insert(
                        "lore_chase".to_string(),
                        serde_json::Value::Number(attrs.lore_chase.into()),
                    );
                    metadata.insert(
                        "gear_chase".to_string(),
                        serde_json::Value::Number(attrs.gear_chase.into()),
                    );
                    metadata.insert(
                        "social_chase".to_string(),
                        serde_json::Value::Number(attrs.social_chase.into()),
                    );
                    metadata.insert(
                        "troublemaker".to_string(),
                        serde_json::Value::Number(attrs.troublemaker.into()),
                    );
                    metadata.insert(
                        "dedication_level".to_string(),
                        serde_json::Value::Number(attrs.dedication_level.into()),
                    );
                }
                // Chat modifiers as JSON blob
                if let Some(ref mods) = personality.chat_modifiers {
                    metadata.insert(
                        "chat_modifiers".to_string(),
                        mods.clone(),
                    );
                }

                let id = format!("personality_archetype_{}", name_lower);
                entries.push((id, embedding, metadata));
                archetype_count += 1;
            }
            Err(e) => {
                warn!(
                    "Failed to embed archetype for {}: {}",
                    personality.name, e
                );
            }
        }

        // 2. Phrase vectors: one per example phrase
        for (i, phrase) in personality.example_phrases.iter().enumerate() {
            match embedder.embed(phrase) {
                Ok(embedding) => {
                    let mut metadata = HashMap::new();
                    metadata.insert(
                        "source".to_string(),
                        serde_json::Value::String("personality".to_string()),
                    );
                    metadata.insert(
                        "entry_type".to_string(),
                        serde_json::Value::String("phrase".to_string()),
                    );
                    metadata.insert(
                        "name".to_string(),
                        serde_json::Value::String(personality.name.clone()),
                    );
                    metadata.insert(
                        "text".to_string(),
                        serde_json::Value::String(phrase.clone()),
                    );
                    metadata.insert(
                        "personality_type".to_string(),
                        serde_json::Value::Number(personality.personality_type.into()),
                    );
                    metadata.insert(
                        "rival".to_string(),
                        serde_json::Value::Bool(is_rival),
                    );

                    let id = format!("personality_phrase_{}_{:03}", name_lower, i + 1);
                    entries.push((id, embedding, metadata));
                    phrase_count += 1;
                }
                Err(e) => {
                    warn!(
                        "Failed to embed phrase for {} #{}: {}",
                        personality.name,
                        i + 1,
                        e
                    );
                }
            }
        }
    }

    debug!(
        "Embedded {} archetype vectors + {} phrase vectors = {} total ({} rivals)",
        archetype_count,
        phrase_count,
        entries.len(),
        rival_count
    );

    if entries.is_empty() {
        warn!("No personality entries to write.");
        return Ok(());
    }

    // Write to .ruvector
    write_personality_ruvector(&entries, output_path)?;

    info!("Personality index: {} entries", entries.len());

    Ok(())
}

/// Write personality entries to a .ruvector (redb-backed HNSW) database.
fn write_personality_ruvector(
    entries: &[(String, Vec<f32>, HashMap<String, serde_json::Value>)],
    output_path: &Path,
) -> Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Delete existing file to avoid stale data
    if output_path.exists() {
        std::fs::remove_file(output_path)
            .with_context(|| format!("Failed to remove existing {:?}", output_path))?;
        debug!("Removed existing personality database at {:?}", output_path);
    }

    let config = VectorStoreConfig {
        dimensions: 384,
        max_elements: entries.len().max(1000),
        ..Default::default()
    };

    let adapter = VectorStoreAdapter::open(output_path, &config)
        .with_context(|| format!("Failed to create VectorDB at {:?}", output_path))?;

    // Sanitize embeddings: skip NaN/zero vectors, add perturbation to avoid
    // hnsw_rs cosine distance assertion when near-identical embeddings cause
    // floating-point distance < 0.
    let batch: Vec<(String, Vec<f32>, HashMap<String, serde_json::Value>)> = entries
        .iter()
        .enumerate()
        .filter_map(|(idx, (id, embedding, metadata))| {
            // Skip vectors with NaN or Inf values
            if embedding.iter().any(|v| !v.is_finite()) {
                warn!("Skipping {} - embedding contains NaN/Inf", id);
                return None;
            }
            let mut perturbed = embedding.clone();
            // Deterministic sub-epsilon jitter based on entry index
            let epsilon = 1e-6_f32;
            let jitter = epsilon * ((idx as f32 + 1.0) / (entries.len() as f32 + 1.0));
            let dim_idx = idx % perturbed.len();
            perturbed[dim_idx] += jitter;
            // Re-normalize so all vectors remain unit-length
            let norm: f32 = perturbed.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm < 1e-10 {
                warn!("Skipping {} - near-zero magnitude embedding", id);
                return None;
            }
            for v in perturbed.iter_mut() {
                *v /= norm;
            }
            Some((id.clone(), perturbed, metadata.clone()))
        })
        .collect();

    let count = adapter.insert_batch(batch)?;

    debug!(
        "Wrote {} personality entries to VectorDB at {:?}",
        count, output_path
    );

    Ok(())
}
