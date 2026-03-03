//! SillyTavern character card export (TavernAI Card V2 spec).
//!
//! Reads personality JSON files and generates one V2 character card JSON
//! per SimPlayer, suitable for direct import into SillyTavern or any
//! TavernAI-compatible frontend.
//!
//! CLI: `erenshor-llm --data-dir data export-tavern --output-dir dist/tavern --include-lorebook`

use anyhow::{Context, Result};
use serde::Serialize;
use std::path::PathBuf;
use tracing::info;

use super::training_exporter::{
    build_training_system_prompt, load_personalities, personality_type_name, PersonalityJson,
};

// ─── Configuration ──────────────────────────────────────────────────────────

/// Export configuration passed from CLI args.
pub struct TavernExportConfig {
    pub data_dir: PathBuf,
    pub output_dir: PathBuf,
    pub personality_types: Option<Vec<u8>>,
    pub include_lorebook: bool,
}

// ─── TavernAI Card V2 structures ────────────────────────────────────────────

#[derive(Serialize)]
struct TavernCardV2 {
    spec: &'static str,
    spec_version: &'static str,
    data: TavernCardData,
}

#[derive(Serialize)]
struct TavernCardData {
    name: String,
    description: String,
    personality: String,
    scenario: String,
    first_mes: String,
    mes_example: String,
    creator_notes: String,
    system_prompt: String,
    post_history_instructions: String,
    alternate_greetings: Vec<String>,
    tags: Vec<String>,
    creator: String,
    character_version: String,
    extensions: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    character_book: Option<CharacterBook>,
}

#[derive(Serialize)]
struct CharacterBook {
    name: String,
    entries: Vec<LorebookEntry>,
}

#[derive(Serialize)]
struct LorebookEntry {
    keys: Vec<String>,
    content: String,
    extensions: serde_json::Value,
    enabled: bool,
    insertion_order: u32,
    case_sensitive: bool,
    priority: u32,
    id: u32,
    comment: String,
    selective: bool,
    secondary_keys: Vec<String>,
    constant: bool,
    position: &'static str,
}

#[derive(Serialize)]
struct TavernManifest {
    total_cards: usize,
    characters: Vec<ManifestCharacter>,
    filters: ManifestFilters,
    include_lorebook: bool,
}

#[derive(Serialize)]
struct ManifestCharacter {
    name: String,
    file: String,
    archetype: String,
    personality_type: String,
    tags: Vec<String>,
}

#[derive(Serialize)]
struct ManifestFilters {
    personality_types: Option<Vec<u8>>,
}

// ─── Field builders ─────────────────────────────────────────────────────────

/// Build rich prose description from archetype, tone, and quirks.
fn build_description(p: &PersonalityJson) -> String {
    let mut desc = format!(
        "{} is a {} with a {} demeanor.",
        p.name, p.archetype, p.tone
    );

    if !p.quirks.is_empty() {
        desc.push_str(&format!(
            " Known for: {}.",
            p.quirks.join("; ")
        ));
    }

    if let Some(ref flags) = p.special_flags {
        if flags.rival {
            desc.push_str(
                " A member of Friends' Club, the elite rival guild. \
                 Arrogant, dismissive, and competitive toward non-members.",
            );
        }
        if flags.is_gm_character {
            desc.push_str(" A GM character with special authority over game events.");
        }
    }

    if let Some(ref guild) = p.guild_affinity {
        if !guild.is_empty() {
            desc.push_str(&format!(" Guild affinity: {}.", guild));
        }
    }

    desc
}

/// Build compact personality summary from tone and behavioral info.
fn build_personality_summary(p: &PersonalityJson) -> String {
    let mut parts: Vec<String> = Vec::new();

    parts.push(format!("Tone: {}", p.tone));

    if !p.speech_patterns.is_empty() {
        parts.push(format!("Speech: {}", p.speech_patterns.join(", ")));
    }

    if !p.vocabulary.is_empty() {
        let vocab_preview: Vec<&str> = p.vocabulary.iter().take(6).map(|s| s.as_str()).collect();
        parts.push(format!("Vocabulary: {}", vocab_preview.join(", ")));
    }

    let ptype = personality_type_name(p.personality_type);
    parts.push(format!("Type: {} ({})", ptype, p.personality_type));

    parts.join(". ")
}

/// Build the static Erenshor scenario text.
fn build_scenario(p: &PersonalityJson) -> String {
    format!(
        "You are chatting in Erenshor, a single-player MMO simulator. \
         You are {}, a SimPlayer -- an AI-driven character who lives in this world. \
         You group, chat, trade, and adventure alongside other players. \
         Stay in character at all times. Respond as {} would in an MMO chat.",
        p.name, p.name
    )
}

/// Apply chat modifiers to a phrase for use as first_mes / greeting.
fn apply_chat_modifiers(phrase: &str, p: &PersonalityJson) -> String {
    let mut result = phrase.to_string();

    if let Some(ref cm) = p.chat_modifiers {
        if cm.types_in_all_caps {
            result = result.to_uppercase();
        } else if cm.types_in_all_lowers {
            result = result.to_lowercase();
        }
    }

    result
}

/// Build mes_example in SillyTavern format: <START> delimited with {{char}} markers.
fn build_mes_example(p: &PersonalityJson) -> String {
    if p.example_phrases.is_empty() {
        return String::new();
    }

    let mut examples = String::new();
    for phrase in &p.example_phrases {
        let modified = apply_chat_modifiers(phrase, p);
        examples.push_str("<START>\n");
        examples.push_str("{{user}}: Hey, what's going on?\n");
        examples.push_str(&format!("{{{{char}}}}: {}\n", modified));
    }

    examples
}

/// Build tags from archetype keywords + personality type.
fn build_tags(p: &PersonalityJson) -> Vec<String> {
    let mut tags: Vec<String> = vec!["erenshor".to_string(), "mmo".to_string(), "simplayer".to_string()];

    // Add personality type as tag
    let ptype = personality_type_name(p.personality_type).to_lowercase();
    if ptype != "unknown" {
        tags.push(ptype);
    }

    // Extract keywords from archetype (lowercase, split on common delimiters)
    for word in p.archetype.to_lowercase().split(|c: char| !c.is_alphanumeric()) {
        let word = word.trim();
        if word.len() >= 3 && !tags.contains(&word.to_string()) {
            // Skip very generic words
            let skip = ["the", "and", "who", "for", "with", "from", "that", "this", "but", "not"];
            if !skip.contains(&word) {
                tags.push(word.to_string());
            }
        }
    }

    tags
}

/// Build human-readable creator notes describing chat modifiers.
fn build_creator_notes(p: &PersonalityJson) -> String {
    let mut notes = vec![format!(
        "Erenshor SimPlayer character card for {}.",
        p.name
    )];

    if let Some(ref cm) = p.chat_modifiers {
        let mut modifiers: Vec<&str> = Vec::new();
        if cm.types_in_all_caps {
            modifiers.push("TYPES IN ALL CAPS");
        }
        if cm.types_in_all_lowers {
            modifiers.push("types in all lowercase");
        }
        if cm.types_in_third_person {
            modifiers.push("refers to self in third person");
        }
        if cm.typo_rate > 0.5 {
            modifiers.push("makes frequent typos");
        }
        if cm.loves_emojis {
            modifiers.push("uses emojis frequently");
        }
        if let Some(ref name) = cm.refers_to_self_as {
            if !name.is_empty() {
                notes.push(format!("Refers to self as \"{}\".", name));
            }
        }
        if !modifiers.is_empty() {
            notes.push(format!("Chat style: {}.", modifiers.join(", ")));
        }
    }

    if !p.knowledge_areas.is_empty() {
        notes.push(format!(
            "Knowledgeable about: {}.",
            p.knowledge_areas.join(", ")
        ));
    }

    notes.join("\n")
}

/// Build character_book from knowledge_areas as lorebook entries.
fn build_character_book(p: &PersonalityJson) -> Option<CharacterBook> {
    if p.knowledge_areas.is_empty() {
        return None;
    }

    let entries: Vec<LorebookEntry> = p
        .knowledge_areas
        .iter()
        .enumerate()
        .map(|(i, topic)| {
            // Derive trigger keywords from the topic
            let keys: Vec<String> = topic
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| w.len() >= 3)
                .map(|w| w.to_string())
                .collect();

            LorebookEntry {
                keys,
                content: format!(
                    "{} has knowledge about: {}. When this topic comes up, \
                     {} should draw on this expertise in their response.",
                    p.name, topic, p.name
                ),
                extensions: serde_json::json!({}),
                enabled: true,
                insertion_order: i as u32,
                case_sensitive: false,
                priority: 10,
                id: i as u32,
                comment: format!("{} - {}", p.name, topic),
                selective: false,
                secondary_keys: Vec::new(),
                constant: false,
                position: "before_char",
            }
        })
        .collect();

    Some(CharacterBook {
        name: format!("{} Lorebook", p.name),
        entries,
    })
}

/// Convert a single PersonalityJson to a TavernAI Card V2.
fn personality_to_card(p: &PersonalityJson, include_lorebook: bool) -> TavernCardV2 {
    let first_mes = if !p.example_phrases.is_empty() {
        apply_chat_modifiers(&p.example_phrases[0], p)
    } else {
        format!("Hey there! I'm {}.", p.name)
    };

    let alternate_greetings: Vec<String> = p
        .example_phrases
        .iter()
        .skip(1)
        .map(|phrase| apply_chat_modifiers(phrase, p))
        .collect();

    let character_book = if include_lorebook {
        build_character_book(p)
    } else {
        None
    };

    TavernCardV2 {
        spec: "chara_card_v2",
        spec_version: "2.0",
        data: TavernCardData {
            name: p.name.clone(),
            description: build_description(p),
            personality: build_personality_summary(p),
            scenario: build_scenario(p),
            first_mes,
            mes_example: build_mes_example(p),
            creator_notes: build_creator_notes(p),
            system_prompt: build_training_system_prompt(p, None),
            post_history_instructions: String::new(),
            alternate_greetings,
            tags: build_tags(p),
            creator: "ErenshorLLM".to_string(),
            character_version: "1.0".to_string(),
            extensions: serde_json::json!({}),
            character_book,
        },
    }
}

// ─── Public API ─────────────────────────────────────────────────────────────

/// Export SillyTavern character cards from personality JSON files.
pub fn export_tavern_cards(config: &TavernExportConfig) -> Result<()> {
    info!("Loading personalities from {:?}", config.data_dir);

    let type_filter = config.personality_types.as_deref();
    let personalities = load_personalities(&config.data_dir, type_filter)?;

    if personalities.is_empty() {
        anyhow::bail!("No personalities found to export");
    }

    std::fs::create_dir_all(&config.output_dir)
        .with_context(|| format!("Failed to create output dir {:?}", config.output_dir))?;

    let mut manifest_chars: Vec<ManifestCharacter> = Vec::new();

    for p in &personalities {
        let card = personality_to_card(p, config.include_lorebook);
        let filename = format!("{}.json", p.name.to_lowercase().replace(' ', "_"));
        let output_path = config.output_dir.join(&filename);

        let json = serde_json::to_string_pretty(&card)
            .with_context(|| format!("Failed to serialize card for {}", p.name))?;
        std::fs::write(&output_path, &json)
            .with_context(|| format!("Failed to write {:?}", output_path))?;

        manifest_chars.push(ManifestCharacter {
            name: p.name.clone(),
            file: filename,
            archetype: p.archetype.clone(),
            personality_type: personality_type_name(p.personality_type).to_string(),
            tags: build_tags(p),
        });
    }

    // Write manifest
    let manifest = TavernManifest {
        total_cards: manifest_chars.len(),
        characters: manifest_chars,
        filters: ManifestFilters {
            personality_types: config.personality_types.clone(),
        },
        include_lorebook: config.include_lorebook,
    };

    let manifest_path = config.output_dir.join("manifest.json");
    let manifest_json =
        serde_json::to_string_pretty(&manifest).context("Failed to serialize manifest")?;
    std::fs::write(&manifest_path, &manifest_json)
        .with_context(|| format!("Failed to write {:?}", manifest_path))?;

    info!(
        "Exported {} SillyTavern character cards to {:?}",
        personalities.len(),
        config.output_dir
    );
    info!("Manifest written to {:?}", manifest_path);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::training_exporter::{ChatModifiers, SpecialFlags};

    fn test_personality() -> PersonalityJson {
        PersonalityJson {
            name: "Fugs".to_string(),
            archetype: "General - Absolute gremlin troll who lives for chaos".to_string(),
            tone: "Chaotic and mischievous".to_string(),
            vocabulary: vec!["lmao".to_string(), "yeet".to_string(), "rekt".to_string()],
            speech_patterns: vec!["Types in all lowercase".to_string()],
            knowledge_areas: vec!["Meme culture".to_string(), "Mob aggro radius limits".to_string()],
            quirks: vec!["Pulls extra mobs on purpose".to_string()],
            example_phrases: vec![
                "lmao watch this im gonna pull EVERYTHING".to_string(),
                "bruh i just got rekt by a lvl 1 rat KEKW".to_string(),
            ],
            personality_type: 5,
            chat_modifiers: Some(ChatModifiers {
                types_in_all_caps: false,
                types_in_all_lowers: true,
                types_in_third_person: false,
                typo_rate: 0.4,
                loves_emojis: false,
                refers_to_self_as: None,
            }),
            special_flags: Some(SpecialFlags {
                rival: false,
                is_gm_character: false,
            }),
            guild_affinity: Some("76584432".to_string()),
        }
    }

    #[test]
    fn test_card_has_correct_spec() {
        let p = test_personality();
        let card = personality_to_card(&p, false);
        assert_eq!(card.spec, "chara_card_v2");
        assert_eq!(card.spec_version, "2.0");
        assert_eq!(card.data.name, "Fugs");
        assert_eq!(card.data.creator, "ErenshorLLM");
    }

    #[test]
    fn test_first_mes_applies_modifiers() {
        let p = test_personality();
        let card = personality_to_card(&p, false);
        // types_in_all_lowers = true, so first_mes should be lowercase
        assert_eq!(
            card.data.first_mes,
            "lmao watch this im gonna pull everything"
        );
    }

    #[test]
    fn test_alternate_greetings() {
        let p = test_personality();
        let card = personality_to_card(&p, false);
        assert_eq!(card.data.alternate_greetings.len(), 1);
        assert_eq!(
            card.data.alternate_greetings[0],
            "bruh i just got rekt by a lvl 1 rat kekw"
        );
    }

    #[test]
    fn test_tags_include_erenshor() {
        let p = test_personality();
        let tags = build_tags(&p);
        assert!(tags.contains(&"erenshor".to_string()));
        assert!(tags.contains(&"mmo".to_string()));
        assert!(tags.contains(&"neutral".to_string())); // personality_type 5
    }

    #[test]
    fn test_lorebook_generation() {
        let p = test_personality();
        let book = build_character_book(&p);
        assert!(book.is_some());
        let book = book.unwrap();
        assert_eq!(book.entries.len(), 2);
        assert!(book.entries[0].content.contains("Fugs"));
    }

    #[test]
    fn test_description_includes_archetype() {
        let p = test_personality();
        let desc = build_description(&p);
        assert!(desc.contains("gremlin troll"));
        assert!(desc.contains("Chaotic and mischievous"));
    }

    #[test]
    fn test_system_prompt_present() {
        let p = test_personality();
        let card = personality_to_card(&p, false);
        assert!(card.data.system_prompt.contains("Fugs"));
        assert!(card.data.system_prompt.contains("Chaotic and mischievous"));
    }

    #[test]
    fn test_mes_example_format() {
        let p = test_personality();
        let mes = build_mes_example(&p);
        assert!(mes.contains("<START>"));
        assert!(mes.contains("{{char}}"));
        assert!(mes.contains("{{user}}"));
    }

    #[test]
    fn test_card_serializes_to_valid_json() {
        let p = test_personality();
        let card = personality_to_card(&p, true);
        let json = serde_json::to_string_pretty(&card).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["spec"], "chara_card_v2");
        assert_eq!(parsed["data"]["name"], "Fugs");
        assert!(parsed["data"]["character_book"].is_object());
    }
}
