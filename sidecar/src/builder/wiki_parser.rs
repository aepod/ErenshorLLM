//! Wiki markdown parser for Erenshor wiki dump files.
//!
//! Reads `.md` files from the wiki dump directory, extracts YAML frontmatter,
//! maps wiki categories to lore categories, cleans wiki syntax, and chunks
//! content for embedding.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
use tracing::{info, warn};

/// A parsed and chunked wiki passage ready for embedding.
#[derive(Debug, Clone)]
pub struct WikiPassage {
    pub text: String,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// YAML frontmatter extracted from a wiki page.
#[derive(Debug, Clone)]
struct WikiFrontmatter {
    title: String,
    source: String,
    categories: Vec<String>,
}

/// Map wiki categories to normalized lore categories.
///
/// Categories are matched case-insensitively. The first match wins.
fn map_wiki_category(wiki_categories: &[String]) -> &'static str {
    for cat in wiki_categories {
        let lower = cat.to_lowercase();
        match lower.as_str() {
            "items" | "consumables" | "equipment" | "armor" | "weapons"
            | "fishing" | "rings" | "earrings" | "necklaces" | "shields"
            | "robes" | "leggings" | "boots" | "gloves" | "helmets"
            | "belts" | "cloaks" | "bracelets" | "chestpieces"
            | "cosmetic_items" | "charms" | "auras" => return "items",

            "enemies" | "bosses" => return "enemies",

            "npcs" | "npc" | "merchants" | "quest_givers" | "trainers"
            | "characters" | "simulated_players" | "vendors" => return "npcs",

            "abilities" | "spells" | "spell_lines" | "buffs" | "debuffs"
            | "stances" => return "abilities",

            "quests" => return "quests",

            "zones" | "cities" | "dungeons" | "locations" => return "zones",

            "classes" | "class" => return "classes",

            "lore" | "history" | "deities" | "factions" | "organizations" => return "world",

            "mechanics" | "systems" | "combat" | "crafting" | "tradeskills"
            | "reputation" | "experience" | "pets" | "bank"
            | "teleportation" => return "mechanics",

            _ => continue,
        }
    }
    "misc"
}

/// Parse YAML frontmatter from a wiki markdown file.
///
/// Expected format:
/// ```text
/// ---
/// title: "Page Title"
/// source: "https://..."
/// categories: ["Cat1", "Cat2"]
/// ---
/// ```
fn parse_frontmatter(content: &str) -> Option<(WikiFrontmatter, &str)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }

    // Find the closing ---
    let after_first = &trimmed[3..];
    let end_idx = after_first.find("\n---")?;
    let yaml_block = &after_first[..end_idx];
    let body_start = 3 + end_idx + 4; // skip "---" + "\n---"
    let body = if body_start < trimmed.len() {
        &trimmed[body_start..]
    } else {
        ""
    };

    let mut title = String::new();
    let mut source = String::new();
    let mut categories = Vec::new();

    for line in yaml_block.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("title:") {
            title = val.trim().trim_matches('"').to_string();
        } else if let Some(val) = line.strip_prefix("source:") {
            source = val.trim().trim_matches('"').to_string();
        } else if let Some(val) = line.strip_prefix("categories:") {
            // Parse inline JSON array: ["Cat1", "Cat2"]
            let val = val.trim();
            if val.starts_with('[') && val.ends_with(']') {
                let inner = &val[1..val.len() - 1];
                categories = inner
                    .split(',')
                    .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }
    }

    if title.is_empty() {
        return None;
    }

    Some((WikiFrontmatter { title, source, categories }, body))
}

/// Clean wiki link syntax from markdown text.
///
/// Transforms `[text](</wiki/...> "tooltip")` to just `text`.
/// Removes image references: `[](</wiki/File:...>)` and similar.
fn clean_wiki_syntax(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '[' {
            // Find the matching ]
            if let Some(close_bracket) = find_closing_bracket(&chars, i) {
                let link_text: String = chars[i + 1..close_bracket].iter().collect();

                // Check if followed by (...)
                if close_bracket + 1 < len && chars[close_bracket + 1] == '(' {
                    if let Some(close_paren) = find_closing_paren(&chars, close_bracket + 1) {
                        let url: String = chars[close_bracket + 2..close_paren].iter().collect();

                        // Image reference: empty link text + File: URL
                        if link_text.is_empty() || url.contains("/wiki/File:") {
                            if link_text.is_empty() {
                                // Skip entirely (image)
                                i = close_paren + 1;
                                continue;
                            }
                        }

                        // Regular wiki link: keep just the text
                        result.push_str(link_text.trim());
                        i = close_paren + 1;
                        continue;
                    }
                }

                // Bare [text] without URL
                result.push_str(&link_text);
                i = close_bracket + 1;
                continue;
            }
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

fn find_closing_bracket(chars: &[char], start: usize) -> Option<usize> {
    let mut depth = 0;
    for i in start..chars.len() {
        match chars[i] {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn find_closing_paren(chars: &[char], start: usize) -> Option<usize> {
    let mut depth = 0;
    for i in start..chars.len() {
        match chars[i] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Collapse markdown tables to plain text lines.
///
/// Converts `col1 | col2 | col3` rows to `col1: col2: col3` and
/// strips separator rows (`---|---|---`).
fn collapse_tables(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();

        // Skip table separator rows
        if trimmed.chars().all(|c| c == '-' || c == '|' || c == ' ' || c == ':')
            && trimmed.contains("---")
        {
            continue;
        }

        // Convert table rows
        if trimmed.contains('|') && !trimmed.starts_with('#') {
            let cells: Vec<&str> = trimmed
                .split('|')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();

            if !cells.is_empty() {
                lines.push(cells.join(": "));
            }
        } else {
            lines.push(line.to_string());
        }
    }

    lines.join("\n")
}

/// Remove empty lines that result from stripping images/links, and
/// collapse multiple blank lines into one.
fn normalize_whitespace(text: &str) -> String {
    let mut result = Vec::new();
    let mut prev_blank = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !prev_blank {
                result.push("");
            }
            prev_blank = true;
        } else {
            result.push(trimmed);
            prev_blank = false;
        }
    }

    result.join("\n").trim().to_string()
}

/// Chunk a wiki page body into passages.
///
/// Pages smaller than 1KB are returned as a single chunk.
/// Larger pages are split at `##` headers.
fn chunk_body(body: &str, title: &str) -> Vec<String> {
    let cleaned = clean_wiki_syntax(body);
    let collapsed = collapse_tables(&cleaned);
    let normalized = normalize_whitespace(&collapsed);

    if normalized.len() < 1024 {
        // Small page: single chunk with title prefix
        let chunk = format!("{}\n\n{}", title, normalized);
        let trimmed = chunk.trim().to_string();
        if trimmed.len() >= 20 {
            return vec![trimmed];
        }
        return Vec::new();
    }

    // Split at ## headers
    let mut chunks: Vec<String> = Vec::new();
    let mut current_chunk = format!("{}\n\n", title);
    let mut current_section = String::new();

    for line in normalized.lines() {
        if line.starts_with("## ") {
            // Flush previous section
            if !current_section.trim().is_empty() {
                current_chunk.push_str(&current_section);
                if current_chunk.trim().len() >= 20 {
                    chunks.push(current_chunk.trim().to_string());
                }
                current_chunk = format!("{}\n\n", title);
            }
            current_section = format!("{}\n", line);
        } else {
            current_section.push_str(line);
            current_section.push('\n');
        }
    }

    // Flush last section
    if !current_section.trim().is_empty() {
        current_chunk.push_str(&current_section);
        if current_chunk.trim().len() >= 20 {
            chunks.push(current_chunk.trim().to_string());
        }
    }

    // If chunking produced nothing useful, fall back to single chunk
    if chunks.is_empty() {
        let full = format!("{}\n\n{}", title, normalized);
        if full.trim().len() >= 20 {
            chunks.push(full.trim().to_string());
        }
    }

    chunks
}

/// Parse a single wiki markdown file into passages with metadata.
pub fn parse_wiki_page(content: &str) -> Result<Vec<WikiPassage>> {
    let (frontmatter, body) = parse_frontmatter(content)
        .ok_or_else(|| anyhow::anyhow!("Missing or invalid YAML frontmatter"))?;

    let category = map_wiki_category(&frontmatter.categories);
    let chunks = chunk_body(body, &frontmatter.title);

    let passages: Vec<WikiPassage> = chunks
        .into_iter()
        .enumerate()
        .map(|(_i, text)| {
            let mut metadata = HashMap::new();
            metadata.insert(
                "source".to_string(),
                serde_json::Value::String("wiki".to_string()),
            );
            metadata.insert(
                "category".to_string(),
                serde_json::Value::String(category.to_string()),
            );
            metadata.insert(
                "page".to_string(),
                serde_json::Value::String(frontmatter.title.clone()),
            );
            metadata.insert(
                "wiki_title".to_string(),
                serde_json::Value::String(frontmatter.title.clone()),
            );
            metadata.insert(
                "wiki_source".to_string(),
                serde_json::Value::String(frontmatter.source.clone()),
            );
            metadata.insert(
                "wiki_categories".to_string(),
                serde_json::Value::String(frontmatter.categories.join(", ")),
            );
            WikiPassage { text, metadata }
        })
        .collect();

    Ok(passages)
}

/// Parse all wiki markdown files in a directory.
///
/// Returns a flat list of all passages from all pages.
pub fn parse_wiki_dump(wiki_dir: &Path) -> Result<Vec<WikiPassage>> {
    info!("Parsing wiki dump from {:?}", wiki_dir);

    if !wiki_dir.exists() {
        anyhow::bail!("Wiki dump directory {:?} does not exist", wiki_dir);
    }

    let mut all_passages = Vec::new();
    let mut file_count = 0;
    let mut error_count = 0;

    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(wiki_dir)
        .with_context(|| format!("Failed to read wiki directory {:?}", wiki_dir))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |e| e == "md"))
        .collect();

    files.sort();

    for path in &files {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read {:?}: {}", path, e);
                error_count += 1;
                continue;
            }
        };

        match parse_wiki_page(&content) {
            Ok(passages) => {
                all_passages.extend(passages);
                file_count += 1;
            }
            Err(e) => {
                warn!("Failed to parse {:?}: {}", path, e);
                error_count += 1;
            }
        }

        if file_count % 100 == 0 && file_count > 0 {
            info!("Parsed {} wiki pages ({} passages so far)...", file_count, all_passages.len());
        }
    }

    info!(
        "Wiki dump parsed: {} pages, {} passages, {} errors",
        file_count,
        all_passages.len(),
        error_count
    );

    Ok(all_passages)
}

// --- Public wrappers for reuse by wiki_importer ---

/// Public wrapper for clean_wiki_syntax (used by wiki_importer).
pub fn clean_wiki_syntax_pub(text: &str) -> String {
    clean_wiki_syntax(text)
}

/// Public wrapper for collapse_tables (used by wiki_importer).
pub fn collapse_tables_pub(text: &str) -> String {
    collapse_tables(text)
}

/// Public wrapper for normalize_whitespace (used by wiki_importer).
pub fn normalize_whitespace_pub(text: &str) -> String {
    normalize_whitespace(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter() {
        let content = r#"---
title: "A Beaktooth"
source: "https://erenshor.wiki.gg/wiki/A_Beaktooth"
categories: ["Items", "Consumables", "Fishing"]
---

# A Beaktooth

Some content here."#;

        let (fm, body) = parse_frontmatter(content).unwrap();
        assert_eq!(fm.title, "A Beaktooth");
        assert_eq!(fm.categories, vec!["Items", "Consumables", "Fishing"]);
        assert!(body.contains("Some content here"));
    }

    #[test]
    fn test_map_wiki_category() {
        assert_eq!(map_wiki_category(&["Items".to_string()]), "items");
        assert_eq!(map_wiki_category(&["Quests".to_string()]), "quests");
        assert_eq!(
            map_wiki_category(&["Enemies".to_string(), "Bosses".to_string()]),
            "enemies"
        );
        assert_eq!(map_wiki_category(&["Unknown".to_string()]), "misc");

        // Previously miscategorized wiki categories
        assert_eq!(map_wiki_category(&["Characters".to_string()]), "npcs");
        assert_eq!(map_wiki_category(&["Simulated_Players".to_string()]), "npcs");
        assert_eq!(map_wiki_category(&["Vendors".to_string()]), "npcs");
        assert_eq!(map_wiki_category(&["Stances".to_string()]), "abilities");
        assert_eq!(map_wiki_category(&["Cosmetic_Items".to_string()]), "items");
        assert_eq!(map_wiki_category(&["Charms".to_string()]), "items");
        assert_eq!(map_wiki_category(&["Auras".to_string()]), "items");
        assert_eq!(map_wiki_category(&["Pets".to_string()]), "mechanics");
        assert_eq!(map_wiki_category(&["Bank".to_string()]), "mechanics");
        assert_eq!(map_wiki_category(&["Teleportation".to_string()]), "mechanics");
        assert_eq!(map_wiki_category(&["Locations".to_string()]), "zones");

        // Zone-name categories should still fall through to misc
        // (they're location metadata, not content categories)
        assert_eq!(map_wiki_category(&["Port_Azure".to_string()]), "misc");
    }

    #[test]
    fn test_clean_wiki_syntax() {
        let input = r#"[Consumable](</wiki/Consumables> "Consumables") found in [Port Azure](</wiki/Port_Azure> "Port Azure")."#;
        let cleaned = clean_wiki_syntax(input);
        assert_eq!(cleaned, "Consumable found in Port Azure.");
    }

    #[test]
    fn test_clean_image_references() {
        let input = r#"[](</wiki/File:A_Beaktooth.png>)"#;
        let cleaned = clean_wiki_syntax(input);
        assert_eq!(cleaned.trim(), "");
    }

    #[test]
    fn test_collapse_tables() {
        let input = "Stage  | Name  | Description\n---|---|---\n1  | Quest Part  | Do the thing";
        let collapsed = collapse_tables(input);
        assert!(collapsed.contains("Stage: Name: Description"));
        assert!(collapsed.contains("1: Quest Part: Do the thing"));
        assert!(!collapsed.contains("---|---|---"));
    }
}
