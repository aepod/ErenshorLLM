//! Offline response template index builder.
//!
//! Reads JSON template files from an input directory, embeds each template's
//! text using the ONNX model, and writes a .ruvector database.
//! Falls back to JSON output if the output path ends with `.json`.
//!
//! Usage: `erenshor-llm build-responses --input data/templates/ --output data/dist/responses.ruvector`

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
use tracing::{info, warn};

use crate::intelligence::embedder::EmbeddingEngine;
use crate::intelligence::templates::{save_template_index, RawTemplateFile, ResponseTemplate};
use crate::intelligence::vector_store::{VectorStoreAdapter, VectorStoreConfig};

/// Build a response template index from JSON files in the input directory.
///
/// If `output_path` ends with `.ruvector`, writes a redb-backed HNSW database.
/// If it ends with `.json`, writes the legacy JSON format.
///
/// Each `.json` file should contain a `RawTemplateFile` (category + templates array).
pub fn build_response_index(
    input_dir: &Path,
    output_path: &Path,
    embedder: &EmbeddingEngine,
) -> Result<()> {
    info!(
        "Building response index from {:?} -> {:?}",
        input_dir, output_path
    );

    if !input_dir.exists() {
        anyhow::bail!("Input directory {:?} does not exist", input_dir);
    }

    // Determine output format from extension
    let use_ruvector = !output_path
        .extension()
        .map_or(false, |e| e == "json");

    let mut all_templates: Vec<ResponseTemplate> = Vec::new();
    let mut file_count = 0;
    let mut template_count = 0;

    // Read all JSON files in the input directory
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for entry in std::fs::read_dir(input_dir)
        .with_context(|| format!("Failed to read directory {:?}", input_dir))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "json") {
            files.push(path);
        }
    }
    files.sort();

    for path in &files {
        file_count += 1;

        info!("Processing {:?}", path);

        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {:?}", path))?;

        let raw_file: RawTemplateFile = serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse {:?}", path))?;

        let category = raw_file.category.clone();

        for raw in &raw_file.templates {
            match embedder.embed(&raw.text) {
                Ok(embedding) => {
                    all_templates.push(ResponseTemplate {
                        id: raw.id.clone(),
                        text: raw.text.clone(),
                        category: category.clone(),
                        context_tags: raw.context_tags.clone(),
                        zone_affinity: raw.zone_affinity.clone(),
                        personality_affinity: raw.personality_affinity.clone(),
                        relationship_min: raw.relationship_min,
                        relationship_max: raw.relationship_max,
                        channel: raw.channel.clone(),
                        priority: raw.priority,
                        embedding,
                    });
                    template_count += 1;

                    if template_count % 50 == 0 {
                        info!("Embedded {}/{} templates...", template_count, template_count);
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to embed template '{}': {}",
                        raw.id, e
                    );
                }
            }
        }
    }

    info!(
        "Embedded {} templates from {} files",
        template_count, file_count
    );

    if all_templates.is_empty() {
        warn!("No templates found. Output will be empty.");
    }

    if use_ruvector {
        write_ruvector_templates(&all_templates, output_path)?;
    } else {
        save_template_index(&all_templates, output_path)?;
    }

    info!(
        "Response index built successfully: {} entries -> {:?}",
        all_templates.len(),
        output_path
    );

    Ok(())
}

/// Write templates to a .ruvector (redb-backed HNSW) database.
fn write_ruvector_templates(templates: &[ResponseTemplate], output_path: &Path) -> Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Delete existing file to avoid stale data
    if output_path.exists() {
        std::fs::remove_file(output_path)
            .with_context(|| format!("Failed to remove existing {:?}", output_path))?;
        info!("Removed existing database at {:?}", output_path);
    }

    let config = VectorStoreConfig {
        dimensions: 384,
        max_elements: templates.len().max(1000),
        ..Default::default()
    };

    let adapter = VectorStoreAdapter::open(output_path, &config)
        .with_context(|| format!("Failed to create VectorDB at {:?}", output_path))?;

    // Batch insert all templates with full metadata
    let batch: Vec<(String, Vec<f32>, HashMap<String, serde_json::Value>)> = templates
        .iter()
        .map(|tmpl| {
            let mut metadata = HashMap::new();
            metadata.insert(
                "text".to_string(),
                serde_json::Value::String(tmpl.text.clone()),
            );
            metadata.insert(
                "category".to_string(),
                serde_json::Value::String(tmpl.category.clone()),
            );
            metadata.insert(
                "context_tags".to_string(),
                serde_json::json!(tmpl.context_tags),
            );
            metadata.insert(
                "zone_affinity".to_string(),
                serde_json::json!(tmpl.zone_affinity),
            );
            metadata.insert(
                "personality_affinity".to_string(),
                serde_json::json!(tmpl.personality_affinity),
            );
            metadata.insert(
                "relationship_min".to_string(),
                serde_json::json!(tmpl.relationship_min),
            );
            metadata.insert(
                "relationship_max".to_string(),
                serde_json::json!(tmpl.relationship_max),
            );
            metadata.insert(
                "channel".to_string(),
                serde_json::json!(tmpl.channel),
            );
            metadata.insert(
                "priority".to_string(),
                serde_json::json!(tmpl.priority),
            );

            (tmpl.id.clone(), tmpl.embedding.clone(), metadata)
        })
        .collect();

    let count = adapter.insert_batch(batch)?;

    info!(
        "Wrote {} templates to VectorDB at {:?}",
        count, output_path
    );

    Ok(())
}
