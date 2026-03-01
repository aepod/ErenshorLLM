//! Response enrichment: placeholder substitution and lore injection.
//!
//! After the ranker selects the best template, the enricher:
//! 1. Replaces {player}, {sim}, {zone}, {mob}, {item} placeholders
//! 2. Strips unfilled placeholders
//! 3. Collapses double spaces
//! 4. Optionally appends a lore snippet

use crate::intelligence::lore::LoreSearchResult;
use regex::Regex;
use std::sync::LazyLock;

/// Context needed for placeholder substitution.
pub struct EnrichContext {
    pub player_name: String,
    pub sim_name: String,
    pub zone: String,
    pub mob_name: Option<String>,
    pub item_name: Option<String>,
}

/// A static regex for matching placeholders like {player}, {zone}, etc.
static PLACEHOLDER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{[a-zA-Z_]+\}").unwrap());

/// Enrich a template text with context substitution.
///
/// Replaces known placeholders and strips unknown ones.
pub fn enrich(text: &str, ctx: &EnrichContext, lore_results: &[LoreSearchResult]) -> String {
    let mut result = text.to_string();

    // Step 1: Replace known placeholders
    result = result.replace("{player}", &ctx.player_name);
    result = result.replace("{sim}", &ctx.sim_name);
    result = result.replace("{zone}", &ctx.zone);

    // Replace {mob} -- from context or from top lore result metadata
    let mob_name = ctx.mob_name.clone().or_else(|| {
        lore_results
            .iter()
            .find_map(|r| r.metadata.get("mob"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    });
    if let Some(mob) = mob_name {
        result = result.replace("{mob}", &mob);
    }

    // Replace {item} -- from context or from top lore result metadata
    let item_name = ctx.item_name.clone().or_else(|| {
        lore_results
            .iter()
            .find_map(|r| r.metadata.get("item"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    });
    if let Some(item) = item_name {
        result = result.replace("{item}", &item);
    }

    // Step 2: Strip any remaining unfilled placeholders
    result = PLACEHOLDER_RE.replace_all(&result, "").to_string();

    // Step 3: Collapse double (or more) spaces
    while result.contains("  ") {
        result = result.replace("  ", " ");
    }

    // Trim leading/trailing whitespace
    result = result.trim().to_string();

    result
}

/// Extract a short lore context snippet from the top lore search results.
///
/// Returns the first sentence (up to the first period) of each result text,
/// limited to the top N results. Returns empty vec if no results.
pub fn extract_lore_context(lore_results: &[LoreSearchResult], max_snippets: usize) -> Vec<String> {
    lore_results
        .iter()
        .take(max_snippets)
        .map(|r| {
            // Take the first sentence (up to first period + space, or first 120 chars)
            if let Some(pos) = r.text.find(". ") {
                r.text[..=pos].to_string()
            } else if r.text.len() > 120 {
                format!("{}...", &r.text[..120])
            } else {
                r.text.clone()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_substitution() {
        let ctx = EnrichContext {
            player_name: "Hero".to_string(),
            sim_name: "Sylphia".to_string(),
            zone: "Port Azure".to_string(),
            mob_name: None,
            item_name: None,
        };

        let result = enrich("Hey {player}! Welcome to {zone}.", &ctx, &[]);
        assert_eq!(result, "Hey Hero! Welcome to Port Azure.");
    }

    #[test]
    fn test_strip_unfilled() {
        let ctx = EnrichContext {
            player_name: "Hero".to_string(),
            sim_name: "Sylphia".to_string(),
            zone: "Port Azure".to_string(),
            mob_name: None,
            item_name: None,
        };

        let result = enrich("I found a {item} near {mob} in {zone}.", &ctx, &[]);
        assert_eq!(result, "I found a near in Port Azure.");
    }

    #[test]
    fn test_collapse_spaces() {
        let ctx = EnrichContext {
            player_name: "Hero".to_string(),
            sim_name: "Sylphia".to_string(),
            zone: "".to_string(),
            mob_name: None,
            item_name: None,
        };

        let result = enrich("Hey {player}, how is  {zone}  going?", &ctx, &[]);
        // {zone} replaced with "", spaces collapse
        assert_eq!(result, "Hey Hero, how is going?");
    }
}
