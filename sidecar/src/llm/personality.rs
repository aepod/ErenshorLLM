use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tracing::{info, warn};

/// Style quirks that affect how dialog text is formatted.
/// These correspond to fields on the game's SimPlayer component
/// (TypesInAllCaps, TypesInThirdPerson, TypoRate, etc.) which are
/// baked into Unity prefabs and only available at runtime for
/// zone-local sims. For cross-zone sims, the personality file's
/// values are used as the authoritative source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleQuirks {
    #[serde(default)]
    pub types_in_all_caps: bool,
    #[serde(default)]
    pub types_in_all_lowers: bool,
    #[serde(default)]
    pub types_in_third_person: bool,
    #[serde(default = "default_typo_rate")]
    pub typo_rate: f32,
    #[serde(default)]
    pub loves_emojis: bool,
    #[serde(default)]
    pub refers_to_self_as: String,
}

fn default_typo_rate() -> f32 {
    0.25
}

impl Default for StyleQuirks {
    fn default() -> Self {
        Self {
            types_in_all_caps: false,
            types_in_all_lowers: false,
            types_in_third_person: false,
            typo_rate: 0.25,
            loves_emojis: false,
            refers_to_self_as: String::new(),
        }
    }
}

/// A SimPlayer personality profile for LLM prompt construction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Personality {
    pub name: String,
    pub archetype: String,
    pub tone: String,
    pub vocabulary: Vec<String>,
    pub speech_patterns: Vec<String>,
    pub knowledge_areas: Vec<String>,
    pub quirks: Vec<String>,
    pub example_phrases: Vec<String>,
    #[serde(default)]
    pub style_quirks: Option<StyleQuirks>,
}

impl Personality {
    /// Derive template-compatible trait flags from archetype, tone, and knowledge.
    ///
    /// Maps keywords in the personality data to the 4 template traits:
    /// social, friendly, scholarly, aggressive.
    pub fn derive_traits(&self) -> HashMap<String, bool> {
        let haystack = format!(
            "{} {} {} {}",
            self.archetype.to_lowercase(),
            self.tone.to_lowercase(),
            self.knowledge_areas.join(" ").to_lowercase(),
            self.quirks.join(" ").to_lowercase(),
        );

        let mut traits = HashMap::new();

        // Social: group-oriented, chatty, community-focused
        let social = haystack.contains("social")
            || haystack.contains("group")
            || haystack.contains("guild")
            || haystack.contains("community")
            || haystack.contains("chat")
            || haystack.contains("party")
            || haystack.contains("team")
            || haystack.contains("leader");
        traits.insert("social".to_string(), social);

        // Friendly: warm, helpful, approachable
        let friendly = haystack.contains("friend")
            || haystack.contains("helpful")
            || haystack.contains("warm")
            || haystack.contains("kind")
            || haystack.contains("cheerful")
            || haystack.contains("welcom")
            || haystack.contains("casual")
            || haystack.contains("easy");
        traits.insert("friendly".to_string(), friendly);

        // Scholarly: lore-focused, intellectual, knowledge-driven
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

        // Aggressive: combat-focused, fierce, competitive
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

    /// Generate consistent trait flags from a sim name hash.
    /// Different names produce different but stable trait profiles.
    fn traits_from_name_hash(name: &str) -> HashMap<String, bool> {
        // Simple hash: sum of char values, produces consistent results per name
        let hash: u32 = name.bytes().map(|b| b as u32).sum();
        let mut traits = HashMap::new();
        traits.insert("social".to_string(), (hash % 4) == 0);
        traits.insert("friendly".to_string(), (hash % 3) == 0);
        traits.insert("scholarly".to_string(), (hash % 5) == 0);
        traits.insert("aggressive".to_string(), (hash % 7) == 0);
        traits
    }

    /// Hardcoded minimal default when no _default.json exists.
    fn hardcoded_default() -> Self {
        Self {
            name: "_default".to_string(),
            archetype: "adventurer".to_string(),
            tone: "friendly and casual".to_string(),
            vocabulary: vec![
                "quest".to_string(),
                "adventure".to_string(),
                "loot".to_string(),
                "grind".to_string(),
            ],
            speech_patterns: vec!["speaks casually like a fellow player".to_string()],
            knowledge_areas: vec!["general Erenshor world knowledge".to_string()],
            quirks: vec![],
            example_phrases: vec![
                "Hey, want to group up?".to_string(),
                "This zone is pretty tough.".to_string(),
            ],
            style_quirks: None,
        }
    }
}

/// Store of personality profiles loaded from JSON files.
pub struct PersonalityStore {
    personalities: HashMap<String, Personality>,
    default: Personality,
}

impl PersonalityStore {
    /// Load all `*.json` files from the given directory.
    /// Malformed files are skipped with a warning.
    pub fn load(dir: &Path) -> Self {
        let mut personalities = HashMap::new();
        let mut default = Personality::hardcoded_default();

        if !dir.exists() {
            warn!("Personality directory not found: {}", dir.display());
            return Self {
                personalities,
                default,
            };
        }

        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                warn!("Failed to read personality directory: {}", e);
                return Self {
                    personalities,
                    default,
                };
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                match load_personality_file(&path) {
                    Ok(personality) => {
                        let key = personality.name.to_lowercase();
                        if key == "_default" {
                            default = personality;
                        } else {
                            personalities.insert(key, personality);
                        }
                    }
                    Err(e) => {
                        warn!("Skipping malformed personality file {}: {}", path.display(), e);
                    }
                }
            }
        }

        info!(
            "Loaded {} personalities ({} named + default)",
            personalities.len() + 1,
            personalities.len()
        );

        Self {
            personalities,
            default,
        }
    }

    /// Look up a personality by SimPlayer name (case-insensitive).
    /// Returns the default personality if not found.
    pub fn get(&self, sim_name: &str) -> &Personality {
        self.personalities
            .get(&sim_name.to_lowercase())
            .unwrap_or(&self.default)
    }

    /// Whether a named personality exists (not the default fallback).
    pub fn has(&self, sim_name: &str) -> bool {
        self.personalities.contains_key(&sim_name.to_lowercase())
    }

    /// Total number of loaded personalities (including default).
    pub fn count(&self) -> usize {
        self.personalities.len() + 1
    }

    /// Derive template-compatible personality trait flags for a SimPlayer.
    ///
    /// Uses the personality's archetype, tone, and knowledge areas to infer
    /// which of the 4 template traits apply: social, friendly, scholarly, aggressive.
    /// If no personality file exists, derives consistent traits from a hash of
    /// the sim name so different sims get different trait profiles.
    pub fn derive_traits(&self, sim_name: &str) -> HashMap<String, bool> {
        let key = sim_name.to_lowercase();
        if let Some(personality) = self.personalities.get(&key) {
            personality.derive_traits()
        } else {
            // No personality file: derive from name hash for consistency
            Personality::traits_from_name_hash(sim_name)
        }
    }
}

fn load_personality_file(path: &Path) -> Result<Personality> {
    let content = std::fs::read_to_string(path)?;
    let personality: Personality = serde_json::from_str(&content)?;
    Ok(personality)
}
