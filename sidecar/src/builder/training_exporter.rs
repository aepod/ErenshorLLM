//! Training data export + fine-tune pipeline.
//!
//! Reads personality JSON, response templates, lore markdown, and grounding
//! data, then generates training pairs in ChatML, Alpaca, and ShareGPT
//! JSONL formats suitable for LoRA fine-tuning of 1-3B parameter models.
//!
//! Pair generation strategies:
//! - **Phrases**: Direct example_phrases from personality files
//! - **Crossover**: Personality × template combinations with zone augmentation
//! - **Lore**: Q&A pairs from lore passages with personality framing
//! - **Multi-turn**: Chained template conversations per personality
//!
//! CLI: `erenshor-llm --data-dir data export-training --format all`

use anyhow::{Context, Result};
use rand::prelude::*;
use rand::rngs::StdRng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::intelligence::lore::parse_lore_markdown;
use crate::intelligence::templates::{RawTemplate, RawTemplateFile};
use crate::llm::grounding::StaticGrounding;

// ─── Configuration ──────────────────────────────────────────────────────────

/// Output format for training data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    ChatML,
    Alpaca,
    ShareGPT,
}

/// Which pair generation strategies to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Strategy {
    Phrases,
    Crossover,
    Lore,
    MultiTurn,
}

impl Strategy {
    pub fn all() -> &'static [Strategy] {
        &[
            Strategy::Phrases,
            Strategy::Crossover,
            Strategy::Lore,
            Strategy::MultiTurn,
        ]
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "phrases" => Some(Strategy::Phrases),
            "crossover" => Some(Strategy::Crossover),
            "lore" => Some(Strategy::Lore),
            "multiturn" => Some(Strategy::MultiTurn),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Strategy::Phrases => "phrases",
            Strategy::Crossover => "crossover",
            Strategy::Lore => "lore",
            Strategy::MultiTurn => "multiturn",
        }
    }
}

/// Export configuration passed from CLI args.
pub struct ExportConfig {
    pub data_dir: PathBuf,
    pub output_dir: PathBuf,
    pub formats: Vec<OutputFormat>,
    pub strategies: Vec<Strategy>,
    pub seed: u64,
    pub max_per_strategy: usize,
    pub personality_types: Option<Vec<u8>>,
    pub categories: Option<Vec<String>>,
    pub zones: Option<Vec<String>>,
}

/// Fine-tune backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FineTuneBackend {
    ConfigOnly,
    Unsloth,
    Axolotl,
}

/// Fine-tune configuration.
pub struct FineTuneConfig {
    pub data_dir: PathBuf,
    pub output_dir: PathBuf,
    pub backend: FineTuneBackend,
    pub base_model: String,
    pub lora_rank: u32,
    pub epochs: u32,
    pub learning_rate: f64,
    pub seed: u64,
}

// ─── Data structures (local copies to avoid circular deps) ──────────────────

/// Personality parsed from JSON (decoupled from llm::personality).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct PersonalityJson {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) archetype: String,
    #[serde(default)]
    pub(crate) tone: String,
    #[serde(default)]
    pub(crate) vocabulary: Vec<String>,
    #[serde(default)]
    pub(crate) speech_patterns: Vec<String>,
    #[serde(default)]
    pub(crate) knowledge_areas: Vec<String>,
    #[serde(default)]
    pub(crate) quirks: Vec<String>,
    #[serde(default)]
    pub(crate) example_phrases: Vec<String>,
    #[serde(default = "default_personality_type")]
    pub(crate) personality_type: u8,
    #[serde(default)]
    pub(crate) chat_modifiers: Option<ChatModifiers>,
    #[serde(default)]
    pub(crate) special_flags: Option<SpecialFlags>,
    #[serde(default)]
    pub(crate) guild_affinity: Option<String>,
}

fn default_personality_type() -> u8 {
    5
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct ChatModifiers {
    #[serde(default)]
    pub(crate) types_in_all_caps: bool,
    #[serde(default)]
    pub(crate) types_in_all_lowers: bool,
    #[serde(default)]
    pub(crate) types_in_third_person: bool,
    #[serde(default = "default_typo_rate")]
    pub(crate) typo_rate: f32,
    #[serde(default)]
    pub(crate) loves_emojis: bool,
    #[serde(default)]
    pub(crate) refers_to_self_as: Option<String>,
}

fn default_typo_rate() -> f32 {
    0.25
}

#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)]
pub(crate) struct SpecialFlags {
    #[serde(default)]
    pub(crate) rival: bool,
    #[serde(default)]
    pub(crate) is_gm_character: bool,
}

// ─── Training pair formats ──────────────────────────────────────────────────

/// ChatML format: {"messages": [{"role": "system", ...}, {"role": "user", ...}, {"role": "assistant", ...}]}
#[derive(Serialize)]
struct ChatMLEntry {
    messages: Vec<ChatMLMessage>,
}

#[derive(Serialize)]
struct ChatMLMessage {
    role: String,
    content: String,
}

/// Alpaca format: {"instruction": ..., "input": ..., "output": ...}
#[derive(Serialize)]
struct AlpacaEntry {
    instruction: String,
    input: String,
    output: String,
}

/// ShareGPT format: {"conversations": [{"from": "system", ...}, {"from": "human", ...}, {"from": "gpt", ...}]}
#[derive(Serialize)]
struct ShareGPTEntry {
    conversations: Vec<ShareGPTMessage>,
}

#[derive(Serialize)]
struct ShareGPTMessage {
    from: String,
    value: String,
}

/// Internal canonical training pair (format-agnostic).
struct TrainingPair {
    system: String,
    turns: Vec<(String, String)>, // (user, assistant) pairs
}

// ─── Manifest ───────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct Manifest {
    seed: u64,
    formats: Vec<String>,
    strategies: Vec<String>,
    total_pairs: usize,
    strategy_counts: HashMap<String, usize>,
    filters: ManifestFilters,
    data_sources: ManifestDataSources,
}

#[derive(Serialize)]
struct ManifestFilters {
    personality_types: Option<Vec<u8>>,
    categories: Option<Vec<String>>,
    zones: Option<Vec<String>>,
    max_per_strategy: usize,
}

#[derive(Serialize)]
struct ManifestDataSources {
    personalities: usize,
    templates: usize,
    lore_passages: usize,
    grounding_zones: usize,
}

// ─── Helpers ────────────────────────────────────────────────────────────────

pub(crate) fn personality_type_name(pt: u8) -> &'static str {
    match pt {
        1 => "Nice",
        2 => "Tryhard",
        3 => "Mean",
        5 => "Neutral",
        _ => "Unknown",
    }
}

/// Derive template-compatible traits from personality data.
fn derive_traits(p: &PersonalityJson) -> HashMap<String, bool> {
    let haystack = format!(
        "{} {} {} {}",
        p.archetype.to_lowercase(),
        p.tone.to_lowercase(),
        p.knowledge_areas.join(" ").to_lowercase(),
        p.quirks.join(" ").to_lowercase(),
    );

    let mut traits = HashMap::new();

    let social = haystack.contains("social")
        || haystack.contains("group")
        || haystack.contains("guild")
        || haystack.contains("community")
        || haystack.contains("chat")
        || haystack.contains("party")
        || haystack.contains("team")
        || haystack.contains("leader");
    traits.insert("social".to_string(), social);

    let friendly = haystack.contains("friend")
        || haystack.contains("helpful")
        || haystack.contains("warm")
        || haystack.contains("kind")
        || haystack.contains("cheerful")
        || haystack.contains("welcom")
        || haystack.contains("casual")
        || haystack.contains("easy");
    traits.insert("friendly".to_string(), friendly);

    let scholarly = haystack.contains("scholar")
        || haystack.contains("lore")
        || haystack.contains("knowledge")
        || haystack.contains("intellectual")
        || haystack.contains("sage")
        || haystack.contains("wisdom")
        || haystack.contains("study")
        || haystack.contains("magic")
        || haystack.contains("arcane");
    traits.insert("scholarly".to_string(), scholarly);

    let aggressive = haystack.contains("aggress")
        || haystack.contains("fierce")
        || haystack.contains("combat")
        || haystack.contains("fight")
        || haystack.contains("warrior")
        || haystack.contains("battle")
        || haystack.contains("fury")
        || haystack.contains("primal")
        || haystack.contains("hunt");
    traits.insert("aggressive".to_string(), aggressive);

    traits
}

/// Build a training system prompt adapted from build_system_section() in prompt.rs.
/// This version uses static personality data instead of live game state.
pub(crate) fn build_training_system_prompt(p: &PersonalityJson, zone: Option<&str>) -> String {
    let mut s = String::with_capacity(512);

    s.push_str(&format!(
        "You are {}, a {} in the world of Erenshor.\n",
        p.name, p.archetype
    ));
    s.push_str(&format!("Tone: {}\n", p.tone));

    if !p.vocabulary.is_empty() {
        s.push_str(&format!("Vocabulary: {}\n", p.vocabulary.join(", ")));
    }

    if !p.speech_patterns.is_empty() {
        s.push_str(&format!(
            "Speech patterns: {}\n",
            p.speech_patterns.join("; ")
        ));
    }

    if !p.quirks.is_empty() {
        s.push_str(&format!("Quirks: {}\n", p.quirks.join("; ")));
    }

    // Chat modifier style quirks
    if let Some(ref cm) = p.chat_modifiers {
        let mut style_notes: Vec<String> = Vec::new();
        if cm.types_in_all_caps {
            style_notes.push("TYPE EVERYTHING IN ALL CAPS".to_string());
        }
        if cm.types_in_all_lowers {
            style_notes.push("type everything in lowercase".to_string());
        }
        if cm.types_in_third_person {
            style_notes.push(
                "always refer to yourself in the third person by your name instead of I/me/my"
                    .to_string(),
            );
        }
        if cm.typo_rate > 1.0 {
            style_notes.push("make occasional typos and spelling mistakes".to_string());
        }
        if cm.loves_emojis {
            style_notes.push("use emojis frequently".to_string());
        }
        if let Some(ref name) = cm.refers_to_self_as {
            if !name.is_empty() {
                style_notes.push(format!("refer to yourself as \"{}\"", name));
            }
        }
        if !style_notes.is_empty() {
            s.push_str(&format!("Writing style: {}\n", style_notes.join("; ")));
        }
    }

    // Rival guild
    let is_rival = p
        .special_flags
        .as_ref()
        .map_or(false, |f| f.rival);
    if is_rival {
        s.push_str(concat!(
            "\nIMPORTANT: You are a member of Friends' Club, the elite rival guild. ",
            "You are arrogant, dismissive, and competitive toward non-members. ",
            "You look down on other players. Be snarky and condescending, ",
            "like a classic MMO uber-guild player who thinks they are better than everyone. ",
            "Reference your guild's superiority. You are an entertaining jerk, not cruel.\n",
        ));
    }

    // Zone context
    if let Some(z) = zone {
        s.push_str(&format!("\nCurrent zone: {}\n", z));
    }

    s
}

/// Expand placeholders in template text.
fn expand_placeholders(
    text: &str,
    personality_name: &str,
    zone: Option<&str>,
    rng: &mut StdRng,
    grounding: &StaticGrounding,
    lore_enemies: &[String],
    lore_items: &[String],
) -> String {
    static PLAYER_NAMES: &[&str] = &["Adventurer", "Hero", "Traveler", "Champion", "Wanderer"];

    let mut result = text.to_string();

    // {player}
    if result.contains("{player}") {
        let name = PLAYER_NAMES[rng.gen_range(0..PLAYER_NAMES.len())];
        result = result.replace("{player}", name);
    }

    // {sim}
    if result.contains("{sim}") {
        result = result.replace("{sim}", personality_name);
    }

    // {zone}
    if result.contains("{zone}") {
        let z = zone.unwrap_or_else(|| {
            if grounding.zones.is_empty() {
                "the wilds"
            } else {
                &grounding.zones[rng.gen_range(0..grounding.zones.len())]
            }
        });
        result = result.replace("{zone}", z);
    }

    // {mob}
    if result.contains("{mob}") {
        let mob = if !lore_enemies.is_empty() {
            &lore_enemies[rng.gen_range(0..lore_enemies.len())]
        } else if !grounding.enemies.is_empty() {
            &grounding.enemies[rng.gen_range(0..grounding.enemies.len())]
        } else {
            "a creature"
        };
        result = result.replace("{mob}", mob);
    }

    // {item}
    if result.contains("{item}") {
        let item = if !lore_items.is_empty() {
            &lore_items[rng.gen_range(0..lore_items.len())]
        } else if !grounding.items.is_empty() {
            &grounding.items[rng.gen_range(0..grounding.items.len())]
        } else {
            "a rare drop"
        };
        result = result.replace("{item}", item);
    }

    result
}

/// Generate a user trigger message based on template category.
fn category_trigger(category: &str, rng: &mut StdRng) -> String {
    match category {
        "greetings" => {
            let options = &[
                "Hello!",
                "Hey there!",
                "Hi!",
                "What's up?",
                "Hey, how's it going?",
            ];
            options[rng.gen_range(0..options.len())].to_string()
        }
        "farewells" => {
            let options = &[
                "See you later!",
                "Gotta go, bye!",
                "Take care!",
                "Heading out, catch you later!",
            ];
            options[rng.gen_range(0..options.len())].to_string()
        }
        "combat" => {
            let options = &[
                "How's the fighting going?",
                "What's the pull strategy?",
                "Ready to fight?",
                "What should we watch out for?",
                "Any combat tips?",
            ];
            options[rng.gen_range(0..options.len())].to_string()
        }
        "lore_knowledge" => {
            let options = &[
                "What do you know about this place?",
                "Tell me about the lore here.",
                "Know anything interesting?",
                "What's the story behind this area?",
            ];
            options[rng.gen_range(0..options.len())].to_string()
        }
        "item_loot" => {
            let options = &[
                "Found any good drops lately?",
                "What gear are you looking for?",
                "Any loot worth mentioning?",
                "What's the best item you've found?",
            ];
            options[rng.gen_range(0..options.len())].to_string()
        }
        "group_party" => {
            let options = &[
                "Want to group up?",
                "Looking for a party?",
                "Need another for the group?",
                "How's the group going?",
            ];
            options[rng.gen_range(0..options.len())].to_string()
        }
        "humor_banter" => {
            let options = &[
                "Haha, anything funny happen today?",
                "Tell me something entertaining.",
                "What's the funniest thing you've seen?",
            ];
            options[rng.gen_range(0..options.len())].to_string()
        }
        "emotional" => {
            let options = &[
                "How are you feeling?",
                "Everything okay?",
                "What's on your mind?",
                "You seem different today.",
            ];
            options[rng.gen_range(0..options.len())].to_string()
        }
        "inquiry" => {
            let options = &[
                "Got a question for you.",
                "Can I ask you something?",
                "What do you think about this?",
                "Know anything about this?",
            ];
            options[rng.gen_range(0..options.len())].to_string()
        }
        "zone_commentary" => {
            let options = &[
                "What do you think of this zone?",
                "How's this area treating you?",
                "Ever been here before?",
                "What's this place like?",
            ];
            options[rng.gen_range(0..options.len())].to_string()
        }
        _ => {
            let options = &[
                "Hey, what's up?",
                "What's going on?",
                "Anything new?",
                "How's it going?",
            ];
            options[rng.gen_range(0..options.len())].to_string()
        }
    }
}

/// Generate a phrase-strategy trigger based on content heuristics.
fn phrase_trigger(phrase: &str, rng: &mut StdRng) -> String {
    let lower = phrase.to_lowercase();

    if lower.contains("dps")
        || lower.contains("damage")
        || lower.contains("tank")
        || lower.contains("heal")
        || lower.contains("pull")
        || lower.contains("fight")
        || lower.contains("aggro")
    {
        let options = &[
            "How's the fighting?",
            "What's our strategy?",
            "How's the damage looking?",
        ];
        return options[rng.gen_range(0..options.len())].to_string();
    }

    if lower.contains("lore")
        || lower.contains("history")
        || lower.contains("ancient")
        || lower.contains("legend")
    {
        let options = &[
            "Know anything about this place?",
            "Tell me about the lore.",
            "What's the history here?",
        ];
        return options[rng.gen_range(0..options.len())].to_string();
    }

    if lower.contains("group")
        || lower.contains("party")
        || lower.contains("guild")
        || lower.contains("team")
    {
        let options = &[
            "Want to group up?",
            "Looking for more?",
            "How's the group?",
        ];
        return options[rng.gen_range(0..options.len())].to_string();
    }

    if lower.contains("item")
        || lower.contains("loot")
        || lower.contains("drop")
        || lower.contains("gear")
        || lower.contains("equip")
    {
        let options = &[
            "Find anything good?",
            "What gear are you chasing?",
            "Any good drops?",
        ];
        return options[rng.gen_range(0..options.len())].to_string();
    }

    let options = &[
        "Hey, what's up?",
        "What's going on?",
        "How's it going?",
        "Anything new?",
    ];
    options[rng.gen_range(0..options.len())].to_string()
}

/// Generate a lore question based on topic metadata.
fn lore_question(
    category: &str,
    page: &str,
    _passage_text: &str,
    rng: &mut StdRng,
) -> String {
    let page_title = page
        .replace('-', " ")
        .split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    match category {
        "zones" => {
            let options = &[
                format!("What can you tell me about {}?", page_title),
                format!("What's {} like?", page_title),
                format!("Ever been to {}?", page_title),
                format!("What should I know about {}?", page_title),
            ];
            options[rng.gen_range(0..options.len())].clone()
        }
        "npcs" => {
            let options = &[
                format!("Who is {}?", page_title),
                format!("What do you know about {}?", page_title),
                format!("Tell me about {}.", page_title),
            ];
            options[rng.gen_range(0..options.len())].clone()
        }
        "enemies" => {
            let options = &[
                format!("What can you tell me about {}?", page_title),
                format!("How dangerous is {}?", page_title),
                format!("What should I know about fighting {}?", page_title),
            ];
            options[rng.gen_range(0..options.len())].clone()
        }
        "items" => {
            let options = &[
                format!("Have you heard of {}?", page_title),
                format!("What do you know about {}?", page_title),
                format!("Is {} any good?", page_title),
            ];
            options[rng.gen_range(0..options.len())].clone()
        }
        "classes" | "abilities" => {
            let options = &[
                format!("What can you tell me about {}?", page_title),
                format!("How does {} work?", page_title),
            ];
            options[rng.gen_range(0..options.len())].clone()
        }
        _ => {
            let options = &[
                format!("What do you know about {}?", page_title),
                format!("Tell me about {}.", page_title),
            ];
            options[rng.gen_range(0..options.len())].clone()
        }
    }
}

/// Title-case a filename stem: "brown-bear" → "Brown Bear"
fn title_case_stem(stem: &str) -> String {
    stem.replace('-', " ")
        .split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ─── Data loading ───────────────────────────────────────────────────────────

pub(crate) fn load_personalities(data_dir: &Path, type_filter: Option<&[u8]>) -> Result<Vec<PersonalityJson>> {
    let dir = data_dir.join("personalities");
    if !dir.exists() {
        anyhow::bail!("Personality directory not found: {:?}", dir);
    }

    let mut json_files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .with_context(|| format!("Failed to read {:?}", dir))?
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

    let mut personalities = Vec::new();
    for path in &json_files {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {:?}", path))?;
        match serde_json::from_str::<PersonalityJson>(&content) {
            Ok(p) => {
                if let Some(filter) = type_filter {
                    if !filter.contains(&p.personality_type) {
                        continue;
                    }
                }
                personalities.push(p);
            }
            Err(e) => {
                warn!("Skipping malformed personality {:?}: {}", path, e);
            }
        }
    }

    info!("Loaded {} personalities", personalities.len());
    Ok(personalities)
}

fn load_templates(
    data_dir: &Path,
    category_filter: Option<&[String]>,
) -> Result<Vec<(String, RawTemplate)>> {
    let dir = data_dir.join("templates");
    if !dir.exists() {
        anyhow::bail!("Templates directory not found: {:?}", dir);
    }

    let mut templates = Vec::new();

    let mut json_files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .with_context(|| format!("Failed to read {:?}", dir))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |e| e == "json"))
        .collect();

    json_files.sort();

    for path in &json_files {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {:?}", path))?;
        match serde_json::from_str::<RawTemplateFile>(&content) {
            Ok(file) => {
                if let Some(filter) = category_filter {
                    if !filter.iter().any(|f| f.eq_ignore_ascii_case(&file.category)) {
                        continue;
                    }
                }
                let cat = file.category.clone();
                for tmpl in file.templates {
                    templates.push((cat.clone(), tmpl));
                }
            }
            Err(e) => {
                warn!("Skipping malformed template {:?}: {}", path, e);
            }
        }
    }

    info!("Loaded {} templates", templates.len());
    Ok(templates)
}

/// Load lore passages from markdown files.
/// Returns (text, category, page_name) tuples.
fn load_lore_passages(data_dir: &Path) -> Result<Vec<(String, String, String)>> {
    let lore_dir = data_dir.join("lore");
    if !lore_dir.exists() {
        anyhow::bail!("Lore directory not found: {:?}", lore_dir);
    }

    let mut passages = Vec::new();

    // Walk all subdirectories
    let subdirs: Vec<PathBuf> = std::fs::read_dir(&lore_dir)
        .with_context(|| format!("Failed to read {:?}", lore_dir))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();

    for subdir in &subdirs {
        let category = subdir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("misc")
            .to_string();

        let md_files: Vec<PathBuf> = std::fs::read_dir(subdir)
            .with_context(|| format!("Failed to read {:?}", subdir))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map_or(false, |e| e == "md"))
            .collect();

        for path in &md_files {
            let page = path
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(e) => {
                    warn!("Failed to read {:?}: {}", path, e);
                    continue;
                }
            };

            let parsed = parse_lore_markdown(&content, &category, &page);
            for (text, _metadata) in parsed {
                // Skip very short or stat-table-heavy passages
                if text.len() < 50 {
                    continue;
                }
                // Heuristic: skip passages that are mostly pipes (table data)
                let pipe_count = text.chars().filter(|&c| c == '|').count();
                if pipe_count > 5 && pipe_count as f32 / text.len() as f32 > 0.02 {
                    continue;
                }
                passages.push((text, category.clone(), page.clone()));
            }
        }
    }

    info!("Loaded {} lore passages", passages.len());
    Ok(passages)
}

/// Collect enemy names from lore/enemies/ directory.
fn load_lore_enemy_names(data_dir: &Path) -> Vec<String> {
    let dir = data_dir.join("lore/enemies");
    if !dir.exists() {
        return Vec::new();
    }
    std::fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map_or(false, |ext| ext == "md")
        })
        .filter_map(|e| {
            e.path()
                .file_stem()
                .and_then(|n| n.to_str())
                .map(|s| title_case_stem(s))
        })
        .collect()
}

/// Collect item names from lore/items/ directory.
fn load_lore_item_names(data_dir: &Path) -> Vec<String> {
    let dir = data_dir.join("lore/items");
    if !dir.exists() {
        return Vec::new();
    }
    std::fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map_or(false, |ext| ext == "md")
        })
        .filter_map(|e| {
            e.path()
                .file_stem()
                .and_then(|n| n.to_str())
                .map(|s| title_case_stem(s))
        })
        .collect()
}

// ─── Strategy implementations ───────────────────────────────────────────────

/// Strategy A: Personality example phrases → direct training pairs.
fn generate_phrase_pairs(
    personalities: &[PersonalityJson],
    rng: &mut StdRng,
    max: usize,
) -> Vec<TrainingPair> {
    let mut pairs = Vec::new();

    for p in personalities {
        let system = build_training_system_prompt(p, None);

        for phrase in &p.example_phrases {
            let user = phrase_trigger(phrase, rng);
            pairs.push(TrainingPair {
                system: system.clone(),
                turns: vec![(user, phrase.clone())],
            });
        }
    }

    if max > 0 && pairs.len() > max {
        pairs.shuffle(rng);
        pairs.truncate(max);
    }

    info!("Strategy A (phrases): {} pairs", pairs.len());
    pairs
}

/// Strategy B: Personality × Template crossover with zone augmentation.
fn generate_crossover_pairs(
    personalities: &[PersonalityJson],
    templates: &[(String, RawTemplate)],
    grounding: &StaticGrounding,
    lore_enemies: &[String],
    lore_items: &[String],
    zone_filter: Option<&[String]>,
    rng: &mut StdRng,
    max: usize,
) -> Vec<TrainingPair> {
    let mut pairs = Vec::new();

    let available_zones: Vec<&str> = if let Some(filter) = zone_filter {
        filter.iter().map(|s| s.as_str()).collect()
    } else {
        grounding.zones.iter().map(|s| s.as_str()).collect()
    };

    for p in personalities {
        let traits = derive_traits(p);

        for (category, tmpl) in templates {
            // Check personality affinity match
            let affinity_match = if tmpl.personality_affinity.is_empty() {
                true // universal template
            } else {
                tmpl.personality_affinity.iter().any(|aff| {
                    traits.get(aff).copied().unwrap_or(false)
                })
            };

            if !affinity_match {
                continue;
            }

            // Determine zones for this template
            let zones_to_use: Vec<Option<&str>> = if tmpl.zone_affinity.is_empty() {
                // Zone-generic: augment with 1-3 random zones
                let n = rng.gen_range(1..=3usize).min(available_zones.len());
                let mut zone_indices: Vec<usize> =
                    (0..available_zones.len()).collect();
                zone_indices.shuffle(rng);
                zone_indices
                    .into_iter()
                    .take(n)
                    .map(|i| Some(available_zones[i]))
                    .collect()
            } else {
                // Zone-specific: use template's zones
                tmpl.zone_affinity
                    .iter()
                    .map(|z| Some(z.as_str()))
                    .collect()
            };

            for zone in &zones_to_use {
                let system = build_training_system_prompt(p, *zone);
                let user = category_trigger(category, rng);
                let assistant = expand_placeholders(
                    &tmpl.text,
                    &p.name,
                    *zone,
                    rng,
                    grounding,
                    lore_enemies,
                    lore_items,
                );

                pairs.push(TrainingPair {
                    system,
                    turns: vec![(user, assistant)],
                });
            }
        }
    }

    if max > 0 && pairs.len() > max {
        pairs.shuffle(rng);
        pairs.truncate(max);
    }

    info!("Strategy B (crossover): {} pairs", pairs.len());
    pairs
}

/// Strategy C: Lore Q&A pairs with personality framing.
fn generate_lore_pairs(
    personalities: &[PersonalityJson],
    passages: &[(String, String, String)], // (text, category, page)
    rng: &mut StdRng,
    max: usize,
) -> Vec<TrainingPair> {
    let mut pairs = Vec::new();

    if personalities.is_empty() || passages.is_empty() {
        return pairs;
    }

    // Weight personalities toward scholarly/knowledge-heavy ones
    let scholarly_indices: Vec<usize> = personalities
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            let traits = derive_traits(p);
            *traits.get("scholarly").unwrap_or(&false)
        })
        .map(|(i, _)| i)
        .collect();

    for (text, category, page) in passages {
        // Pick 1-2 personalities per passage
        let n_personalities = rng.gen_range(1..=2usize);

        for _ in 0..n_personalities {
            let p_idx = if !scholarly_indices.is_empty() && rng.gen_bool(0.6) {
                scholarly_indices[rng.gen_range(0..scholarly_indices.len())]
            } else {
                rng.gen_range(0..personalities.len())
            };

            let p = &personalities[p_idx];
            let mut system = build_training_system_prompt(p, None);
            // Inject lore passage as world knowledge
            system.push_str(&format!("\nWORLD KNOWLEDGE:\n- {}\n", text));

            let user = lore_question(category, page, text, rng);

            // Build a natural answer from the passage text
            // Trim to first 2-3 sentences for conciseness
            let sentences: Vec<&str> = text
                .split(|c: char| c == '.' || c == '!' || c == '?')
                .filter(|s| s.trim().len() > 10)
                .collect();
            let answer = if sentences.len() <= 3 {
                text.clone()
            } else {
                let n = rng.gen_range(2..=3usize).min(sentences.len());
                sentences[..n]
                    .iter()
                    .map(|s| s.trim())
                    .collect::<Vec<_>>()
                    .join(". ")
                    + "."
            };

            pairs.push(TrainingPair {
                system,
                turns: vec![(user, answer)],
            });
        }
    }

    if max > 0 && pairs.len() > max {
        pairs.shuffle(rng);
        pairs.truncate(max);
    }

    info!("Strategy C (lore): {} pairs", pairs.len());
    pairs
}

/// Strategy D: Multi-turn conversations per personality.
/// Chains 2-4 compatible templates into coherent conversations.
fn generate_multiturn_pairs(
    personalities: &[PersonalityJson],
    templates: &[(String, RawTemplate)],
    grounding: &StaticGrounding,
    lore_enemies: &[String],
    lore_items: &[String],
    rng: &mut StdRng,
    max: usize,
) -> Vec<TrainingPair> {
    let mut pairs = Vec::new();

    for p in personalities {
        let traits = derive_traits(p);

        // Find compatible templates (match personality affinity)
        let compatible: Vec<&(String, RawTemplate)> = templates
            .iter()
            .filter(|(_, tmpl)| {
                if tmpl.personality_affinity.is_empty() {
                    true
                } else {
                    tmpl.personality_affinity
                        .iter()
                        .any(|aff| traits.get(aff).copied().unwrap_or(false))
                }
            })
            .collect();

        if compatible.len() < 2 {
            continue;
        }

        // Pick a zone for this conversation
        let zone = if !grounding.zones.is_empty() {
            Some(grounding.zones[rng.gen_range(0..grounding.zones.len())].as_str())
        } else {
            None
        };

        let system = build_training_system_prompt(p, zone);

        // Build 2-4 turn conversation
        let n_turns = rng.gen_range(2..=4usize).min(compatible.len());

        // Try to order by conversation flow: greeting → topic → farewell
        let mut selected: Vec<&(String, RawTemplate)> = Vec::new();

        // Try to get a greeting first
        if let Some(greeting) = compatible
            .iter()
            .filter(|(cat, _)| cat == "greetings")
            .choose(rng)
        {
            selected.push(greeting);
        }

        // Fill middle with topic templates
        let middle_cats: Vec<&&(String, RawTemplate)> = compatible
            .iter()
            .filter(|(cat, _)| cat != "greetings" && cat != "farewells")
            .collect();
        let middle_needed = n_turns.saturating_sub(selected.len()).saturating_sub(1); // leave room for farewell
        for tmpl in middle_cats.choose_multiple(rng, middle_needed) {
            selected.push(tmpl);
        }

        // Try to end with a farewell
        if selected.len() < n_turns {
            if let Some(farewell) = compatible
                .iter()
                .filter(|(cat, _)| cat == "farewells")
                .choose(rng)
            {
                selected.push(farewell);
            }
        }

        // Fill remaining if needed
        while selected.len() < n_turns {
            if let Some(any) = compatible.choose(rng) {
                selected.push(any);
            } else {
                break;
            }
        }

        if selected.len() < 2 {
            continue;
        }

        let mut turns = Vec::new();
        for (category, tmpl) in &selected {
            let user = category_trigger(category, rng);
            let assistant = expand_placeholders(
                &tmpl.text,
                &p.name,
                zone,
                rng,
                grounding,
                lore_enemies,
                lore_items,
            );
            turns.push((user, assistant));
        }

        pairs.push(TrainingPair {
            system,
            turns,
        });
    }

    if max > 0 && pairs.len() > max {
        pairs.shuffle(rng);
        pairs.truncate(max);
    }

    info!("Strategy D (multiturn): {} pairs", pairs.len());
    pairs
}

// ─── Format serializers ─────────────────────────────────────────────────────

fn pair_to_chatml(pair: &TrainingPair) -> String {
    let mut messages = vec![ChatMLMessage {
        role: "system".to_string(),
        content: pair.system.clone(),
    }];
    for (user, assistant) in &pair.turns {
        messages.push(ChatMLMessage {
            role: "user".to_string(),
            content: user.clone(),
        });
        messages.push(ChatMLMessage {
            role: "assistant".to_string(),
            content: assistant.clone(),
        });
    }
    serde_json::to_string(&ChatMLEntry { messages }).unwrap_or_default()
}

fn pair_to_alpaca(pair: &TrainingPair) -> String {
    // For multi-turn, concatenate turns; Alpaca is single-turn by nature
    let (user, assistant) = if pair.turns.len() == 1 {
        (pair.turns[0].0.clone(), pair.turns[0].1.clone())
    } else {
        let user_parts: Vec<String> = pair.turns.iter().map(|(u, _)| u.clone()).collect();
        let asst_parts: Vec<String> = pair.turns.iter().map(|(_, a)| a.clone()).collect();
        (user_parts.join("\n"), asst_parts.join("\n"))
    };

    serde_json::to_string(&AlpacaEntry {
        instruction: pair.system.clone(),
        input: user,
        output: assistant,
    })
    .unwrap_or_default()
}

fn pair_to_sharegpt(pair: &TrainingPair) -> String {
    let mut conversations = vec![ShareGPTMessage {
        from: "system".to_string(),
        value: pair.system.clone(),
    }];
    for (user, assistant) in &pair.turns {
        conversations.push(ShareGPTMessage {
            from: "human".to_string(),
            value: user.clone(),
        });
        conversations.push(ShareGPTMessage {
            from: "gpt".to_string(),
            value: assistant.clone(),
        });
    }
    serde_json::to_string(&ShareGPTEntry { conversations }).unwrap_or_default()
}

// ─── Public API ─────────────────────────────────────────────────────────────

/// Export training data to JSONL files.
pub fn export_training(config: &ExportConfig) -> Result<()> {
    let mut rng = StdRng::seed_from_u64(config.seed);

    info!("Loading training data from {:?}", config.data_dir);

    // Load all data sources
    let type_filter = config.personality_types.as_deref();
    let personalities = load_personalities(&config.data_dir, type_filter)?;

    let cat_filter = config.categories.as_deref();
    let templates = load_templates(&config.data_dir, cat_filter)?;

    let passages = load_lore_passages(&config.data_dir)?;

    let grounding = StaticGrounding::load(&config.data_dir.join("grounding.json"))
        .unwrap_or_else(|| {
            warn!("grounding.json not found, using empty grounding");
            StaticGrounding {
                zones: Vec::new(),
                classes: Vec::new(),
                factions: Vec::new(),
                items: Vec::new(),
                quests: Vec::new(),
                npcs: Vec::new(),
                enemies: Vec::new(),
            }
        });

    let lore_enemies = load_lore_enemy_names(&config.data_dir);
    let lore_items = load_lore_item_names(&config.data_dir);

    info!(
        "Data: {} personalities, {} templates, {} lore passages, {} zones",
        personalities.len(),
        templates.len(),
        passages.len(),
        grounding.zones.len(),
    );

    // Generate pairs per strategy
    let mut all_pairs: Vec<TrainingPair> = Vec::new();
    let mut strategy_counts: HashMap<String, usize> = HashMap::new();

    let zone_filter = config.zones.as_deref();

    for strategy in &config.strategies {
        let max = config.max_per_strategy;
        let pairs = match strategy {
            Strategy::Phrases => {
                generate_phrase_pairs(&personalities, &mut rng, max)
            }
            Strategy::Crossover => {
                generate_crossover_pairs(
                    &personalities,
                    &templates,
                    &grounding,
                    &lore_enemies,
                    &lore_items,
                    zone_filter,
                    &mut rng,
                    max,
                )
            }
            Strategy::Lore => {
                generate_lore_pairs(&personalities, &passages, &mut rng, max)
            }
            Strategy::MultiTurn => {
                generate_multiturn_pairs(
                    &personalities,
                    &templates,
                    &grounding,
                    &lore_enemies,
                    &lore_items,
                    &mut rng,
                    max,
                )
            }
        };
        strategy_counts.insert(strategy.name().to_string(), pairs.len());
        all_pairs.extend(pairs);
    }

    info!("Total training pairs: {}", all_pairs.len());

    // Shuffle all pairs together
    all_pairs.shuffle(&mut rng);

    // Write output files per format
    for format in &config.formats {
        let format_dir = match format {
            OutputFormat::ChatML => config.output_dir.join("chatml"),
            OutputFormat::Alpaca => config.output_dir.join("alpaca"),
            OutputFormat::ShareGPT => config.output_dir.join("sharegpt"),
        };
        std::fs::create_dir_all(&format_dir)
            .with_context(|| format!("Failed to create {:?}", format_dir))?;

        let output_path = format_dir.join("combined.jsonl");
        let mut lines: Vec<String> = Vec::with_capacity(all_pairs.len());

        for pair in &all_pairs {
            let line = match format {
                OutputFormat::ChatML => pair_to_chatml(pair),
                OutputFormat::Alpaca => pair_to_alpaca(pair),
                OutputFormat::ShareGPT => pair_to_sharegpt(pair),
            };
            lines.push(line);
        }

        let content = lines.join("\n") + "\n";
        std::fs::write(&output_path, content)
            .with_context(|| format!("Failed to write {:?}", output_path))?;

        let format_name = match format {
            OutputFormat::ChatML => "chatml",
            OutputFormat::Alpaca => "alpaca",
            OutputFormat::ShareGPT => "sharegpt",
        };
        info!(
            "Wrote {}/{}/combined.jsonl ({} lines)",
            config.output_dir.display(),
            format_name,
            all_pairs.len()
        );
    }

    // Write manifest
    let manifest = Manifest {
        seed: config.seed,
        formats: config
            .formats
            .iter()
            .map(|f| match f {
                OutputFormat::ChatML => "chatml".to_string(),
                OutputFormat::Alpaca => "alpaca".to_string(),
                OutputFormat::ShareGPT => "sharegpt".to_string(),
            })
            .collect(),
        strategies: config
            .strategies
            .iter()
            .map(|s| s.name().to_string())
            .collect(),
        total_pairs: all_pairs.len(),
        strategy_counts,
        filters: ManifestFilters {
            personality_types: config.personality_types.clone(),
            categories: config.categories.clone(),
            zones: config.zones.clone(),
            max_per_strategy: config.max_per_strategy,
        },
        data_sources: ManifestDataSources {
            personalities: personalities.len(),
            templates: templates.len(),
            lore_passages: passages.len(),
            grounding_zones: grounding.zones.len(),
        },
    };

    let manifest_path = config.output_dir.join("manifest.json");
    let manifest_json =
        serde_json::to_string_pretty(&manifest).context("Failed to serialize manifest")?;
    std::fs::write(&manifest_path, manifest_json)
        .with_context(|| format!("Failed to write {:?}", manifest_path))?;

    info!("Manifest written to {:?}", manifest_path);
    info!(
        "Export complete: {} pairs across {} format(s)",
        all_pairs.len(),
        config.formats.len()
    );

    Ok(())
}

/// Run the fine-tune pipeline: export data + generate/run training scripts.
pub fn fine_tune(config: &FineTuneConfig) -> Result<()> {
    // Step 1: Export training data (ChatML format for fine-tuning)
    let export_config = ExportConfig {
        data_dir: config.data_dir.clone(),
        output_dir: config.output_dir.clone(),
        formats: vec![OutputFormat::ChatML],
        strategies: Strategy::all().to_vec(),
        seed: config.seed,
        max_per_strategy: 0,
        personality_types: None,
        categories: None,
        zones: None,
    };

    info!("Step 1: Exporting training data...");
    export_training(&export_config)?;

    let training_file = config.output_dir.join("chatml/combined.jsonl");
    if !training_file.exists() {
        anyhow::bail!("Training data not found at {:?}", training_file);
    }

    // Step 2: Generate training scripts
    let output_model_dir = config.output_dir.join("output");
    std::fs::create_dir_all(&output_model_dir)
        .context("Failed to create output model directory")?;

    info!("Step 2: Generating training configuration...");

    // Generate unsloth training script
    let unsloth_script = generate_unsloth_script(config, &training_file, &output_model_dir);
    let unsloth_path = config.output_dir.join("train_unsloth.py");
    std::fs::write(&unsloth_path, &unsloth_script)
        .with_context(|| format!("Failed to write {:?}", unsloth_path))?;
    info!("Generated {:?}", unsloth_path);

    // Generate axolotl config
    let axolotl_config = generate_axolotl_config(config, &training_file, &output_model_dir);
    let axolotl_path = config.output_dir.join("axolotl.yml");
    std::fs::write(&axolotl_path, &axolotl_config)
        .with_context(|| format!("Failed to write {:?}", axolotl_path))?;
    info!("Generated {:?}", axolotl_path);

    // Generate README
    let readme = generate_training_readme(config);
    let readme_path = config.output_dir.join("TRAINING_README.md");
    std::fs::write(&readme_path, &readme)
        .with_context(|| format!("Failed to write {:?}", readme_path))?;

    // Step 3: Run training if requested
    match config.backend {
        FineTuneBackend::ConfigOnly => {
            info!("Config-only mode: training scripts generated. Run manually:");
            info!("  Unsloth: python3 {}", unsloth_path.display());
            info!("  Axolotl: axolotl train {}", axolotl_path.display());
        }
        FineTuneBackend::Unsloth => {
            info!("Step 3: Running unsloth training...");
            let status = std::process::Command::new("python3")
                .arg(&unsloth_path)
                .status()
                .context("Failed to run python3 train_unsloth.py")?;

            if !status.success() {
                anyhow::bail!("Unsloth training failed with exit code: {:?}", status.code());
            }
            info!("Unsloth training complete. LoRA adapter at {:?}", output_model_dir);
        }
        FineTuneBackend::Axolotl => {
            info!("Step 3: Running axolotl training...");
            let status = std::process::Command::new("axolotl")
                .arg("train")
                .arg(&axolotl_path)
                .status()
                .context("Failed to run axolotl train")?;

            if !status.success() {
                anyhow::bail!("Axolotl training failed with exit code: {:?}", status.code());
            }
            info!("Axolotl training complete. LoRA adapter at {:?}", output_model_dir);
        }
    }

    Ok(())
}

// ─── Training script generation ─────────────────────────────────────────────

fn generate_unsloth_script(
    config: &FineTuneConfig,
    training_file: &Path,
    output_dir: &Path,
) -> String {
    format!(
        r#"\"\"\"Erenshor NPC Fine-Tuning Script (Unsloth)

Auto-generated by erenshor-llm export-training pipeline.
Targets: {base_model} with QLoRA 4-bit quantization.

Usage: python3 train_unsloth.py
Requirements: pip install unsloth datasets
\"\"\"

import os
os.environ["TOKENIZERS_PARALLELISM"] = "false"
os.environ["TORCHDYNAMO_DISABLE"] = "1"

from unsloth import FastLanguageModel
from datasets import load_dataset
from trl import SFTTrainer
from transformers import TrainingArguments

# ─── Config ──────────────────────────────────────────────────────────────
BASE_MODEL = "{base_model}"
TRAINING_FILE = "{training_file}"
OUTPUT_DIR = "{output_dir}"
LORA_RANK = {lora_rank}
EPOCHS = {epochs}
LEARNING_RATE = {learning_rate}
SEED = {seed}
MAX_SEQ_LENGTH = 2048
BATCH_SIZE = 2
GRADIENT_ACCUMULATION = 8  # Effective batch = 16

# ─── Load model with 4-bit quantization ─────────────────────────────────
model, tokenizer = FastLanguageModel.from_pretrained(
    model_name=BASE_MODEL,
    max_seq_length=MAX_SEQ_LENGTH,
    dtype=None,  # auto-detect
    load_in_4bit=True,
)

# ─── Apply LoRA adapters ────────────────────────────────────────────────
model = FastLanguageModel.get_peft_model(
    model,
    r={lora_rank},
    target_modules=["q_proj", "k_proj", "v_proj", "o_proj",
                     "gate_proj", "up_proj", "down_proj"],
    lora_alpha={lora_alpha},
    lora_dropout=0.05,
    bias="none",
    use_gradient_checkpointing="unsloth",
    random_state={seed},
)

# ─── Load dataset ───────────────────────────────────────────────────────
dataset = load_dataset("json", data_files=TRAINING_FILE, split="train")

def format_chat(example):
    """Format messages using the tokenizer's native chat template."""
    messages = example["messages"]
    if hasattr(tokenizer, "apply_chat_template"):
        text = tokenizer.apply_chat_template(
            messages, tokenize=False, add_generation_prompt=False,
        )
    else:
        text = ""
        for msg in messages:
            role = msg["role"]
            content = msg["content"]
            text += f"<|im_start|>{{role}}\n{{content}}<|im_end|>\n"
    return {{"text": text}}

dataset = dataset.map(format_chat)

# ─── Train ──────────────────────────────────────────────────────────────
trainer = SFTTrainer(
    model=model,
    tokenizer=tokenizer,
    train_dataset=dataset,
    dataset_text_field="text",
    max_seq_length=MAX_SEQ_LENGTH,
    args=TrainingArguments(
        output_dir=OUTPUT_DIR,
        per_device_train_batch_size=BATCH_SIZE,
        gradient_accumulation_steps=GRADIENT_ACCUMULATION,
        num_train_epochs=EPOCHS,
        learning_rate=LEARNING_RATE,
        bf16=True,
        logging_steps=10,
        save_strategy="epoch",
        seed=SEED,
        warmup_ratio=0.03,
        lr_scheduler_type="cosine",
        optim="adamw_8bit",
        report_to="none",
    ),
)

print("Starting training...")
stats = trainer.train()
print(f"Training complete! Steps: {{stats.global_step}}, Loss: {{stats.training_loss:.4f}}")

# ─── Save LoRA adapter ──────────────────────────────────────────────────
model.save_pretrained(OUTPUT_DIR)
tokenizer.save_pretrained(OUTPUT_DIR)
print(f"LoRA adapter saved to {{OUTPUT_DIR}}")
"#,
        base_model = config.base_model,
        training_file = training_file.display(),
        output_dir = output_dir.display(),
        lora_rank = config.lora_rank,
        lora_alpha = config.lora_rank * 2,
        epochs = config.epochs,
        learning_rate = config.learning_rate,
        seed = config.seed,
    )
}

fn generate_axolotl_config(
    config: &FineTuneConfig,
    training_file: &Path,
    output_dir: &Path,
) -> String {
    format!(
        r#"# Erenshor NPC Fine-Tuning Config (Axolotl)
# Auto-generated by erenshor-llm export-training pipeline.
# Usage: axolotl train axolotl.yml

base_model: {base_model}
model_type: AutoModelForCausalLM
tokenizer_type: AutoTokenizer

load_in_4bit: true
adapter: qlora
lora_r: {lora_rank}
lora_alpha: {lora_alpha}
lora_dropout: 0.05
lora_target_modules:
  - q_proj
  - k_proj
  - v_proj
  - o_proj
  - gate_proj
  - up_proj
  - down_proj

datasets:
  - path: {training_file}
    type: chatml
    ds_type: json

sequence_len: 2048
sample_packing: true
pad_to_sequence_len: true

output_dir: {output_dir}
num_epochs: {epochs}
micro_batch_size: 4
gradient_accumulation_steps: 4
learning_rate: {learning_rate}
optimizer: adamw_bnb_8bit
lr_scheduler: cosine
warmup_ratio: 0.03

bf16: true
logging_steps: 10
save_strategy: epoch
seed: {seed}

wandb_project: erenshor-npc-finetune
"#,
        base_model = config.base_model,
        training_file = training_file.display(),
        output_dir = output_dir.display(),
        lora_rank = config.lora_rank,
        lora_alpha = config.lora_rank * 2,
        epochs = config.epochs,
        learning_rate = config.learning_rate,
        seed = config.seed,
    )
}

fn generate_training_readme(config: &FineTuneConfig) -> String {
    format!(
        r#"# Erenshor NPC Fine-Tuning

Auto-generated training pipeline for Erenshor NPC dialog model.

## Configuration

- **Base model**: {}
- **Backend**: {:?}
- **LoRA rank**: {}
- **Epochs**: {}
- **Learning rate**: {}
- **Seed**: {}

## Files

- `chatml/combined.jsonl` - Training data in ChatML format
- `train_unsloth.py` - Unsloth training script (QLoRA 4-bit)
- `axolotl.yml` - Axolotl training config
- `manifest.json` - Export metadata and statistics
- `output/` - LoRA adapter output (after training)

## Quick Start

### Using Unsloth (recommended for single GPU)

```bash
pip install unsloth datasets trl
python3 train_unsloth.py
```

### Using Axolotl

```bash
pip install axolotl
axolotl train axolotl.yml
```

## Using the LoRA Adapter

After training, the LoRA adapter will be in `output/`. Load it with:

```python
from unsloth import FastLanguageModel

model, tokenizer = FastLanguageModel.from_pretrained(
    model_name="output/",
    max_seq_length=2048,
    load_in_4bit=True,
)
FastLanguageModel.for_inference(model)
```

Or convert to GGUF for local inference:

```python
model.save_pretrained_gguf("output-gguf", tokenizer, quantization_method="q4_k_m")
```
"#,
        config.base_model,
        config.backend,
        config.lora_rank,
        config.epochs,
        config.learning_rate,
        config.seed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_personality() -> PersonalityJson {
        PersonalityJson {
            name: "TestSim".to_string(),
            archetype: "scholarly mage".to_string(),
            tone: "wise and measured".to_string(),
            vocabulary: vec!["arcane".to_string(), "tome".to_string()],
            speech_patterns: vec!["speaks formally".to_string()],
            knowledge_areas: vec!["magic".to_string(), "lore".to_string()],
            quirks: vec!["always reading".to_string()],
            example_phrases: vec![
                "The arcane arts require patience.".to_string(),
                "I've read about that in the old tomes.".to_string(),
            ],
            personality_type: 5,
            chat_modifiers: None,
            special_flags: None,
            guild_affinity: None,
        }
    }

    #[test]
    fn test_derive_traits() {
        let p = test_personality();
        let traits = derive_traits(&p);
        assert!(traits["scholarly"]);
        assert!(!traits["aggressive"]);
    }

    #[test]
    fn test_build_system_prompt() {
        let p = test_personality();
        let prompt = build_training_system_prompt(&p, Some("Port Azure"));
        assert!(prompt.contains("TestSim"));
        assert!(prompt.contains("scholarly mage"));
        assert!(prompt.contains("Port Azure"));
    }

    #[test]
    fn test_expand_placeholders() {
        let grounding = StaticGrounding {
            zones: vec!["Port Azure".to_string()],
            classes: vec!["Arcanist".to_string()],
            factions: vec![],
            items: vec!["Diamond Claymore".to_string()],
            quests: vec![],
            npcs: vec![],
            enemies: vec!["A Brown Bear".to_string()],
        };

        let mut rng = StdRng::seed_from_u64(42);
        let result = expand_placeholders(
            "Hey {player}, welcome to {zone}!",
            "TestSim",
            Some("Port Azure"),
            &mut rng,
            &grounding,
            &["Brown Bear".to_string()],
            &["Diamond Claymore".to_string()],
        );
        assert!(result.contains("Port Azure"));
        assert!(!result.contains("{player}"));
        assert!(!result.contains("{zone}"));
    }

    #[test]
    fn test_chatml_format() {
        let pair = TrainingPair {
            system: "You are TestSim.".to_string(),
            turns: vec![("Hello!".to_string(), "Hey there!".to_string())],
        };
        let json = pair_to_chatml(&pair);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["messages"].as_array().unwrap().len(), 3);
        assert_eq!(parsed["messages"][0]["role"], "system");
    }

    #[test]
    fn test_alpaca_format() {
        let pair = TrainingPair {
            system: "You are TestSim.".to_string(),
            turns: vec![("Hello!".to_string(), "Hey there!".to_string())],
        };
        let json = pair_to_alpaca(&pair);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["instruction"].as_str().unwrap().contains("TestSim"));
        assert_eq!(parsed["input"], "Hello!");
        assert_eq!(parsed["output"], "Hey there!");
    }

    #[test]
    fn test_sharegpt_format() {
        let pair = TrainingPair {
            system: "You are TestSim.".to_string(),
            turns: vec![("Hello!".to_string(), "Hey there!".to_string())],
        };
        let json = pair_to_sharegpt(&pair);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["conversations"].as_array().unwrap().len(), 3);
        assert_eq!(parsed["conversations"][0]["from"], "system");
        assert_eq!(parsed["conversations"][1]["from"], "human");
        assert_eq!(parsed["conversations"][2]["from"], "gpt");
    }

    #[test]
    fn test_phrase_pairs_generation() {
        let personalities = vec![test_personality()];
        let mut rng = StdRng::seed_from_u64(42);
        let pairs = generate_phrase_pairs(&personalities, &mut rng, 0);
        assert_eq!(pairs.len(), 2); // 2 example phrases
    }

    #[test]
    fn test_personality_type_name() {
        assert_eq!(personality_type_name(1), "Nice");
        assert_eq!(personality_type_name(2), "Tryhard");
        assert_eq!(personality_type_name(3), "Mean");
        assert_eq!(personality_type_name(5), "Neutral");
    }

    #[test]
    fn test_title_case_stem() {
        assert_eq!(title_case_stem("brown-bear"), "Brown Bear");
        assert_eq!(title_case_stem("abyssal-plate"), "Abyssal Plate");
    }
}
