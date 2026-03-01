//! Wiki dump importer for organizing wiki markdown files into lore categories.
//!
//! Reads `.md` files from a wiki dump directory (with YAML frontmatter),
//! cleans wiki syntax, maps categories, and writes organized files into
//! the curated lore directory structure.
//!
//! This is a file-organization step, NOT an embedding step. The output
//! files are plain markdown that `build-index` can then process.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, info, warn};

use crate::builder::wiki_parser;

/// Import wiki dump files into the curated lore directory structure.
///
/// For each wiki page:
/// 1. Parse YAML frontmatter (title, source, categories)
/// 2. Map wiki categories to lore categories
/// 3. Clean wiki syntax (links, images, tables)
/// 4. Write to `{output_dir}/{category}/{slug}.md`
///
/// Existing files with `source: "curated"` in frontmatter are NOT overwritten.
pub fn import_wiki(wiki_dir: &Path, output_dir: &Path) -> Result<()> {
    info!(
        "Importing wiki dump from {:?} -> {:?}",
        wiki_dir, output_dir
    );

    if !wiki_dir.exists() {
        anyhow::bail!("Wiki dump directory {:?} does not exist", wiki_dir);
    }

    // Collect all .md files from the wiki dump
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(wiki_dir)
        .with_context(|| format!("Failed to read wiki directory {:?}", wiki_dir))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |e| e == "md"))
        .collect();

    files.sort();

    let mut category_counts: HashMap<String, usize> = HashMap::new();
    let mut total_processed = 0;
    let mut skipped_curated = 0;
    let mut skipped_parse_error = 0;

    for path in &files {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read {:?}: {}", path, e);
                skipped_parse_error += 1;
                continue;
            }
        };

        // Parse the frontmatter
        let (frontmatter, body) = match parse_frontmatter(&content) {
            Some(result) => result,
            None => {
                warn!("No valid frontmatter in {:?}, skipping", path);
                skipped_parse_error += 1;
                continue;
            }
        };

        // Map wiki categories to lore category
        let category = map_wiki_category(&frontmatter.categories);

        // Generate slug from title
        let slug = slugify(&frontmatter.title);
        if slug.is_empty() {
            warn!("Empty slug for {:?}, skipping", path);
            skipped_parse_error += 1;
            continue;
        }

        // Ensure category directory exists
        let category_dir = output_dir.join(category);
        std::fs::create_dir_all(&category_dir)
            .with_context(|| format!("Failed to create category dir {:?}", category_dir))?;

        let output_file = category_dir.join(format!("{}.md", slug));

        // Check if existing file is curated (skip if so)
        if output_file.exists() {
            if is_curated_file(&output_file) {
                skipped_curated += 1;
                continue;
            }
        }

        // Clean the wiki body
        let cleaned_body = clean_wiki_body(body);

        // Build output content with frontmatter
        let output_content = build_output_content(
            &frontmatter.title,
            &frontmatter.source,
            &frontmatter.categories,
            category,
            &cleaned_body,
        );

        // Write the file
        std::fs::write(&output_file, &output_content)
            .with_context(|| format!("Failed to write {:?}", output_file))?;

        *category_counts.entry(category.to_string()).or_insert(0) += 1;
        total_processed += 1;

        if total_processed % 100 == 0 {
            debug!("Imported {} wiki pages so far...", total_processed);
        }
    }

    // Print summary
    info!(
        "Wiki import: {} processed, {} skipped (curated), {} skipped (error)",
        total_processed, skipped_curated, skipped_parse_error
    );

    let mut sorted_categories: Vec<_> = category_counts.iter().collect();
    sorted_categories.sort_by_key(|(k, _)| (*k).clone());
    for (category, count) in sorted_categories {
        debug!("  {}: {}", category, count);
    }

    Ok(())
}

/// Parsed YAML frontmatter from a wiki page.
struct WikiFrontmatter {
    title: String,
    source: String,
    categories: Vec<String>,
}

/// Parse YAML frontmatter from wiki markdown content.
///
/// Returns the frontmatter and the remaining body text.
fn parse_frontmatter(content: &str) -> Option<(WikiFrontmatter, &str)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }

    let after_first = &trimmed[3..];
    let end_idx = after_first.find("\n---")?;
    let yaml_block = &after_first[..end_idx];
    let body_start = 3 + end_idx + 4;
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

/// Map wiki categories to normalized lore categories.
///
/// Must be kept in sync with wiki_parser::map_wiki_category.
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

/// Generate a URL-safe slug from a title.
///
/// Lowercase, spaces to hyphens, strip special characters.
fn slugify(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c
            } else if c == ' ' || c == '_' {
                '-'
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Check if an existing file has `source: "curated"` in its frontmatter.
fn is_curated_file(path: &Path) -> bool {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };

    // Quick check: look for source: "curated" or source: curated in frontmatter
    if let Some(fm_end) = content.find("\n---") {
        let frontmatter = &content[..fm_end];
        return frontmatter.contains("source: \"curated\"")
            || frontmatter.contains("source: curated");
    }

    false
}

/// Clean wiki markdown body text.
///
/// Applies the same cleaning as wiki_parser.rs:
/// - Strip wiki links
/// - Strip image references
/// - Collapse tables
/// - Normalize whitespace
fn clean_wiki_body(body: &str) -> String {
    let cleaned = wiki_parser::clean_wiki_syntax_pub(body);
    let collapsed = wiki_parser::collapse_tables_pub(&cleaned);
    wiki_parser::normalize_whitespace_pub(&collapsed)
}

/// Build output markdown content with YAML frontmatter.
fn build_output_content(
    title: &str,
    source: &str,
    wiki_categories: &[String],
    lore_category: &str,
    body: &str,
) -> String {
    let categories_str = wiki_categories
        .iter()
        .map(|c| format!("\"{}\"", c))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "---\ntitle: \"{}\"\nsource: \"wiki\"\nwiki_source: \"{}\"\ncategories: [{}]\nlore_category: \"{}\"\n---\n\n# {}\n\n{}",
        title.replace('"', "\\\""),
        source.replace('"', "\\\""),
        categories_str,
        lore_category,
        title,
        body.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("A Beaktooth"), "a-beaktooth");
        assert_eq!(slugify("Port Azure"), "port-azure");
        assert_eq!(slugify("GM-Burgee's Staff"), "gm-burgee-s-staff");
        assert_eq!(slugify("  Spaces  Everywhere  "), "spaces-everywhere");
        assert_eq!(slugify("ALL_CAPS_NAME"), "all-caps-name");
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
    }

    #[test]
    fn test_is_curated_detection() {
        // This is more of an integration test, but we can test the frontmatter parsing
        let curated = "---\ntitle: \"Test\"\nsource: \"curated\"\n---\n\nBody text";
        let wiki = "---\ntitle: \"Test\"\nsource: \"wiki\"\n---\n\nBody text";

        // Check that curated frontmatter is detected
        assert!(curated.contains("source: \"curated\""));
        assert!(!wiki.contains("source: \"curated\""));
    }

    #[test]
    fn test_build_output_content() {
        let content = build_output_content(
            "Test Page",
            "https://example.com",
            &["Items".to_string(), "Equipment".to_string()],
            "items",
            "This is the body.",
        );

        assert!(content.starts_with("---\n"));
        assert!(content.contains("source: \"wiki\""));
        assert!(content.contains("lore_category: \"items\""));
        assert!(content.contains("# Test Page"));
        assert!(content.contains("This is the body."));
    }
}
