//! Offline lore index builder.
//!
//! Reads markdown files from curated lore directories and/or wiki dump
//! directories, splits them into passages, embeds each passage using the
//! ONNX model, and writes a .ruvector database. Falls back to JSON output
//! if the output path ends with `.json`.
//!
//! Supports multiple input sources:
//! - `CuratedLore`: structured directory with category subdirs (existing behavior)
//! - `WikiDump`: wiki markdown files with YAML frontmatter
//!
//! Usage:
//!   `erenshor-llm build-index --input data/lore/ --output data/dist/lore.ruvector`
//!   `erenshor-llm build-index --lore data/lore/ --wiki ../../wikidump/erenshor_dump/ --output data/dist/lore.ruvector`

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::builder::wiki_parser;
use crate::intelligence::embedder::EmbeddingEngine;
use crate::intelligence::lore::{parse_lore_markdown, save_lore_index, LoreEntry};
use crate::intelligence::vector_store::{VectorStoreAdapter, VectorStoreConfig};

/// A source of lore content for index building.
#[derive(Debug, Clone)]
pub enum LoreSource {
    /// Curated lore markdown files organized by category subdirectory.
    /// Category is derived from the parent directory name.
    CuratedLore { path: PathBuf },
    /// Wiki dump markdown files with YAML frontmatter.
    /// Category is derived from wiki categories in frontmatter.
    WikiDump { path: PathBuf },
}

/// Build a lore index from a single curated directory (backward compatible).
///
/// If `output_path` ends with `.ruvector`, writes a redb-backed HNSW database.
/// If it ends with `.json`, writes the legacy JSON format.
///
/// Directory structure:
/// ```text
/// input/
///   zones/
///     port-azure.md
///     hidden-hills.md
///   items/
///     weapons.md
///   npcs/
///     bosses.md
///   quests/
///     main-story.md
/// ```
///
/// Each subdirectory name becomes the `category` metadata.
/// Each filename (without extension) becomes the `page` metadata.
pub fn build_lore_index(
    input_dir: &Path,
    output_path: &Path,
    embedder: &EmbeddingEngine,
) -> Result<()> {
    let sources = [LoreSource::CuratedLore {
        path: input_dir.to_path_buf(),
    }];
    build_lore_index_multi(&sources, output_path, embedder)
}

/// Build a lore index from multiple sources (curated + wiki dump).
///
/// All sources are merged into a single output .ruvector file.
pub fn build_lore_index_multi(
    sources: &[LoreSource],
    output_path: &Path,
    embedder: &EmbeddingEngine,
) -> Result<()> {
    info!(
        "Building lore index from {} source(s) -> {:?}",
        sources.len(),
        output_path
    );

    // Determine output format from extension
    let use_ruvector = !output_path.extension().map_or(false, |e| e == "json");

    let mut all_entries: Vec<LoreEntry> = Vec::new();
    let mut total_passage_count = 0;

    for source in sources {
        match source {
            LoreSource::CuratedLore { path } => {
                let (entries, count) = process_curated_lore(path, embedder)?;
                info!(
                    "Curated lore: {} entries from {:?}",
                    count, path
                );
                all_entries.extend(entries);
                total_passage_count += count;
            }
            LoreSource::WikiDump { path } => {
                let (entries, count) = process_wiki_dump(path, embedder)?;
                info!(
                    "Wiki dump: {} entries from {:?}",
                    count, path
                );
                all_entries.extend(entries);
                total_passage_count += count;
            }
        }
    }

    info!("Total embedded passages: {}", total_passage_count);

    if all_entries.is_empty() {
        warn!("No passages found. Output will be empty.");
    }

    if use_ruvector {
        write_ruvector_lore(&all_entries, output_path)?;
    } else {
        save_lore_index(&all_entries, output_path)?;
    }

    info!(
        "Lore index built successfully: {} entries -> {:?}",
        all_entries.len(),
        output_path
    );

    Ok(())
}

/// Process curated lore markdown files from a category-structured directory.
fn process_curated_lore(
    input_dir: &Path,
    embedder: &EmbeddingEngine,
) -> Result<(Vec<LoreEntry>, usize)> {
    info!("Processing curated lore from {:?}", input_dir);

    if !input_dir.exists() {
        anyhow::bail!("Input directory {:?} does not exist", input_dir);
    }

    let mut entries: Vec<LoreEntry> = Vec::new();
    let mut passage_count = 0;
    let mut file_count = 0;

    for entry in walkdir(input_dir)? {
        let path = entry;
        if !path.extension().map_or(false, |e| e == "md") {
            continue;
        }

        file_count += 1;

        let category = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let page = path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        info!(
            "Processing {:?} (category={}, page={})",
            path, category, page
        );

        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {:?}", path))?;

        let passages = parse_lore_markdown(&content, category, page);

        for (i, (text, mut metadata)) in passages.into_iter().enumerate() {
            let id = format!("lore_curated_{}_{:03}", page, i + 1);

            // Tag source as curated
            metadata.insert(
                "source".to_string(),
                serde_json::Value::String("curated".to_string()),
            );

            match embedder.embed(&text) {
                Ok(embedding) => {
                    entries.push(LoreEntry {
                        id,
                        text,
                        embedding,
                        metadata,
                    });
                    passage_count += 1;

                    if passage_count % 50 == 0 {
                        info!("Curated: embedded {} passages...", passage_count);
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to embed curated passage '{}...': {}",
                        &text[..text.len().min(50)],
                        e
                    );
                }
            }
        }
    }

    info!(
        "Curated lore: {} passages from {} files",
        passage_count, file_count
    );

    Ok((entries, passage_count))
}

/// Process wiki dump markdown files with YAML frontmatter.
fn process_wiki_dump(
    wiki_dir: &Path,
    embedder: &EmbeddingEngine,
) -> Result<(Vec<LoreEntry>, usize)> {
    info!("Processing wiki dump from {:?}", wiki_dir);

    let wiki_passages = wiki_parser::parse_wiki_dump(wiki_dir)?;

    let mut entries: Vec<LoreEntry> = Vec::new();
    let mut passage_count = 0;

    for (i, passage) in wiki_passages.into_iter().enumerate() {
        let page = passage
            .metadata
            .get("page")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let category = passage
            .metadata
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("misc");

        let id = format!("lore_wiki_{}_{:03}", category, i + 1);

        match embedder.embed(&passage.text) {
            Ok(embedding) => {
                entries.push(LoreEntry {
                    id,
                    text: passage.text,
                    embedding,
                    metadata: passage.metadata,
                });
                passage_count += 1;

                if passage_count % 100 == 0 {
                    info!("Wiki: embedded {} passages...", passage_count);
                }
            }
            Err(e) => {
                warn!(
                    "Failed to embed wiki passage '{}' #{}: {}",
                    page,
                    i + 1,
                    e
                );
            }
        }
    }

    info!("Wiki dump: {} passages embedded", passage_count);

    Ok((entries, passage_count))
}

/// Write lore entries to a .ruvector (redb-backed HNSW) database.
fn write_ruvector_lore(entries: &[LoreEntry], output_path: &Path) -> Result<()> {
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
        max_elements: entries.len().max(1000),
        ..Default::default()
    };

    let adapter = VectorStoreAdapter::open(output_path, &config)
        .with_context(|| format!("Failed to create VectorDB at {:?}", output_path))?;

    // Batch insert all entries
    let batch: Vec<(String, Vec<f32>, HashMap<String, serde_json::Value>)> = entries
        .iter()
        .map(|entry| {
            let mut metadata = entry.metadata.clone();
            metadata.insert(
                "text".to_string(),
                serde_json::Value::String(entry.text.clone()),
            );
            (entry.id.clone(), entry.embedding.clone(), metadata)
        })
        .collect();

    let count = adapter.insert_batch(batch)?;

    info!(
        "Wrote {} lore entries to VectorDB at {:?}",
        count, output_path
    );

    Ok(())
}

/// Simple recursive directory walker (avoids adding `walkdir` crate dependency).
fn walkdir(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut files = Vec::new();

    if !dir.is_dir() {
        anyhow::bail!("{:?} is not a directory", dir);
    }

    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("Failed to read directory {:?}", dir))?
    {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            files.extend(walkdir(&path)?);
        } else {
            files.push(path);
        }
    }

    // Sort for deterministic order
    files.sort();
    Ok(files)
}
