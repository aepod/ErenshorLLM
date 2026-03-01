//! Game Entity Prompt Anchor (GEPA) grounding.
//!
//! Extracts real game entity names from lore search results and static lists,
//! producing a GroundingContext that the prompt builder injects as a GEPA section.
//! This prevents the LLM from hallucinating zone, item, NPC, or class names.

use crate::llm::prompt::LoreContext;
use crate::routes::respond::RespondRequest;
use serde::Deserialize;
use std::path::Path;
use tracing::{info, warn};

/// Static game entity names loaded from grounding.json.
#[derive(Debug, Clone, Deserialize)]
pub struct StaticGrounding {
    pub zones: Vec<String>,
    pub classes: Vec<String>,
    pub factions: Vec<String>,
}

impl StaticGrounding {
    /// Load static grounding data from a JSON file.
    pub fn load(path: &Path) -> Option<Self> {
        if !path.exists() {
            warn!("Grounding file not found at {:?}", path);
            return None;
        }

        match std::fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(data) => {
                    let sg: Self = data;
                    info!(
                        "Grounding data loaded: {} zones, {} classes, {} factions",
                        sg.zones.len(),
                        sg.classes.len(),
                        sg.factions.len()
                    );
                    Some(sg)
                }
                Err(e) => {
                    warn!("Failed to parse grounding.json: {}", e);
                    None
                }
            },
            Err(e) => {
                warn!("Failed to read grounding.json: {}", e);
                None
            }
        }
    }
}

/// Per-request grounding context built from search results + static data.
///
/// Contains the specific entity names relevant to this request that the
/// LLM should use (and never invent alternatives for).
#[derive(Debug, Clone)]
pub struct GroundingContext {
    pub zones: Vec<String>,
    pub items: Vec<String>,
    pub npcs: Vec<String>,
    pub classes: Vec<String>,
}

impl GroundingContext {
    /// Build grounding context from lore search results and request context.
    ///
    /// Extracts entity names from:
    /// - Lore search result text (proper nouns that match known entities)
    /// - Request context (current zone, group members)
    /// - Static lists (classes are always included)
    pub fn from_search_results(
        lore_results: &[LoreContext],
        request: &RespondRequest,
        static_data: &StaticGrounding,
    ) -> Self {
        let mut zones: Vec<String> = Vec::new();
        let mut items: Vec<String> = Vec::new();
        let mut npcs: Vec<String> = Vec::new();

        // Always include the player's current zone
        if !request.zone.is_empty() {
            zones.push(request.zone.clone());
        }

        // Include group members as NPCs
        for member in &request.group_members {
            if !member.is_empty() {
                npcs.push(member.clone());
            }
        }

        // Extract entity names from lore results by matching against static lists
        for lore in lore_results {
            // Check if any known zone names appear in the lore text
            for zone in &static_data.zones {
                if lore.text.contains(zone.as_str()) && !zones.contains(zone) {
                    zones.push(zone.clone());
                }
            }

            // Extract item names: look for capitalized multi-word phrases
            // that appear at the start of lore entries (item reviews start
            // with the item name)
            let text = &lore.text;
            if let Some(first_line) = text.lines().next() {
                let trimmed = first_line.trim().trim_start_matches("- ");
                // If the first word is capitalized and it looks like an item name
                if trimmed.len() > 3
                    && trimmed.chars().next().map_or(false, |c| c.is_uppercase())
                    && !trimmed.starts_with("You ")
                    && !trimmed.starts_with("The ")
                    && !trimmed.starts_with("In ")
                {
                    // Take up to the first verb/preposition as the item name
                    let name = extract_entity_name(trimmed);
                    if !name.is_empty() && !items.contains(&name) {
                        items.push(name);
                    }
                }
            }
        }

        // Add some static zone context if we have few from search
        if zones.len() < 3 {
            // Add zones adjacent to the current zone (first 3 from static list
            // not already included)
            for zone in &static_data.zones {
                if !zones.contains(zone) {
                    zones.push(zone.clone());
                    if zones.len() >= 5 {
                        break;
                    }
                }
            }
        }

        // Limit sizes
        zones.truncate(8);
        items.truncate(8);
        npcs.truncate(5);

        GroundingContext {
            zones,
            items,
            npcs,
            classes: static_data.classes.clone(),
        }
    }

    /// Format the GEPA prompt section.
    pub fn format_prompt_section(&self) -> String {
        let mut s = String::with_capacity(300);
        s.push_str("\nGROUNDING (use ONLY these real names from Erenshor, never invent others):\n");

        if !self.zones.is_empty() {
            s.push_str(&format!("Zones: {}\n", self.zones.join(", ")));
        }
        if !self.items.is_empty() {
            s.push_str(&format!("Items: {}\n", self.items.join(", ")));
        }
        if !self.npcs.is_empty() {
            s.push_str(&format!("NPCs: {}\n", self.npcs.join(", ")));
        }
        s.push_str(&format!("Classes: {}\n", self.classes.join(", ")));
        s.push_str("Do NOT reference any zone, item, NPC, or class not listed above.\n");

        s
    }

    /// Get all known entity names as a flat list (for validation).
    pub fn all_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = Vec::new();
        for z in &self.zones {
            names.push(z.as_str());
        }
        for i in &self.items {
            names.push(i.as_str());
        }
        for n in &self.npcs {
            names.push(n.as_str());
        }
        for c in &self.classes {
            names.push(c.as_str());
        }
        names
    }
}

/// Extract a probable entity name from the beginning of a text line.
/// Stops at common sentence-continuing words.
fn extract_entity_name(text: &str) -> String {
    let stop_words = [
        " is ", " are ", " was ", " were ", " has ", " can ", " will ",
        " drops ", " costs ", " from ", " for ", " with ", " at ",
    ];

    let mut end = text.len();
    for stop in &stop_words {
        if let Some(pos) = text.find(stop) {
            if pos < end && pos > 0 {
                end = pos;
            }
        }
    }

    text[..end].trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_static() -> StaticGrounding {
        StaticGrounding {
            zones: vec![
                "Port Azure".to_string(),
                "Hidden Hills".to_string(),
                "Abyssal Lake".to_string(),
            ],
            classes: vec![
                "Arcanist".to_string(),
                "Paladin".to_string(),
                "Windblade".to_string(),
            ],
            factions: vec!["The Azure Guard".to_string()],
        }
    }

    fn test_request(zone: &str, group: Vec<String>) -> RespondRequest {
        use std::collections::HashMap;
        RespondRequest {
            player_message: "hello".to_string(),
            channel: "say".to_string(),
            sim_name: "TestSim".to_string(),
            personality: HashMap::new(),
            zone: zone.to_string(),
            relationship: 5.0,
            player_name: "Hero".to_string(),
            player_level: 10,
            player_class: "Paladin".to_string(),
            player_guild: String::new(),
            sim_guild: String::new(),
            sim_is_rival: false,
            group_members: group,
            template_candidates: None,
            lore_context_count: None,
            memory_context_count: None,
            w_semantic: None,
            w_channel: None,
            w_zone: None,
            w_personality: None,
            w_relationship: None,
        }
    }

    #[test]
    fn test_grounding_includes_current_zone() {
        let request = test_request("Hidden Hills", vec![]);
        let ctx = GroundingContext::from_search_results(&[], &request, &test_static());
        assert!(ctx.zones.contains(&"Hidden Hills".to_string()));
    }

    #[test]
    fn test_grounding_includes_group_members() {
        let request = test_request("Port Azure", vec!["Bumknee".to_string(), "Drakkal".to_string()]);

        let ctx = GroundingContext::from_search_results(&[], &request, &test_static());
        assert!(ctx.npcs.contains(&"Bumknee".to_string()));
        assert!(ctx.npcs.contains(&"Drakkal".to_string()));
    }

    #[test]
    fn test_format_prompt_section() {
        let ctx = GroundingContext {
            zones: vec!["Port Azure".to_string(), "Hidden Hills".to_string()],
            items: vec!["Abyssal Plate".to_string()],
            npcs: vec!["Drakkal".to_string()],
            classes: vec!["Paladin".to_string(), "Reaver".to_string()],
        };

        let section = ctx.format_prompt_section();
        assert!(section.contains("Port Azure"));
        assert!(section.contains("Abyssal Plate"));
        assert!(section.contains("Drakkal"));
        assert!(section.contains("Paladin"));
        assert!(section.contains("never invent others"));
    }

    #[test]
    fn test_extract_entity_name() {
        assert_eq!(extract_entity_name("Abyssal Plate is a level 38 endgame chest"), "Abyssal Plate");
        assert_eq!(extract_entity_name("Eon Blade of Time drops from bosses"), "Eon Blade of Time");
    }
}
