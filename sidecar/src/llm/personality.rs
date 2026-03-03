use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;
use tracing::{debug, info, warn};

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

    /// Generate a personality on-the-fly from game trait flags and the sim name.
    ///
    /// Uses the name hash for deterministic variation and the game's trait flags
    /// to build a coherent archetype, tone, vocabulary, and speech patterns.
    /// This produces unique-feeling personalities for game-generated SimPlayers
    /// that don't have personality files.
    fn generate(name: &str, game_traits: &HashMap<String, bool>) -> Self {
        let hash: u32 = name.bytes().map(|b| b as u32).sum();

        // Determine active trait flags (from game or name hash fallback)
        let is_friendly = *game_traits.get("friendly").unwrap_or(&false)
            || *game_traits.get("helpful").unwrap_or(&false)
            || *game_traits.get("kind").unwrap_or(&false);
        let is_aggressive = *game_traits.get("aggressive").unwrap_or(&false)
            || *game_traits.get("brave").unwrap_or(&false)
            || *game_traits.get("fierce").unwrap_or(&false);
        let is_scholarly = *game_traits.get("scholarly").unwrap_or(&false)
            || *game_traits.get("wise").unwrap_or(&false)
            || *game_traits.get("intellectual").unwrap_or(&false);
        let is_social = *game_traits.get("social").unwrap_or(&false)
            || *game_traits.get("leader").unwrap_or(&false)
            || *game_traits.get("chatty").unwrap_or(&false);
        let is_grumpy = *game_traits.get("grumpy").unwrap_or(&false)
            || *game_traits.get("veteran").unwrap_or(&false);

        // If game sent no useful traits, derive from name hash
        let any_trait = is_friendly || is_aggressive || is_scholarly || is_social || is_grumpy;
        let (friendly, aggressive, scholarly, social) = if any_trait {
            (is_friendly, is_aggressive, is_scholarly, is_social)
        } else {
            (
                (hash % 3) == 0,
                (hash % 7) == 0,
                (hash % 5) == 0,
                (hash % 4) == 0,
            )
        };

        // Pick archetype based on dominant traits
        let archetype = if aggressive && social {
            "raid leader"
        } else if aggressive && scholarly {
            "battle mage"
        } else if aggressive {
            "warrior"
        } else if scholarly && friendly {
            "helpful sage"
        } else if scholarly {
            "lore keeper"
        } else if friendly && social {
            "social butterfly"
        } else if social {
            "guild organizer"
        } else if friendly {
            "friendly adventurer"
        } else if is_grumpy {
            "grizzled veteran"
        } else {
            // Use name hash to pick from a pool
            match hash % 6 {
                0 => "wandering explorer",
                1 => "casual adventurer",
                2 => "seasoned fighter",
                3 => "curious traveler",
                4 => "quiet observer",
                _ => "resourceful survivor",
            }
        };

        // Pick tone
        let tone = if is_grumpy {
            "blunt, impatient, speaks from experience"
        } else if friendly && social {
            "warm, enthusiastic, loves meeting people"
        } else if friendly {
            "approachable and encouraging"
        } else if aggressive {
            "direct, competitive, respects strength"
        } else if scholarly {
            "thoughtful and precise"
        } else if social {
            "chatty and outgoing"
        } else {
            match hash % 4 {
                0 => "laid-back and easygoing",
                1 => "cautiously friendly",
                2 => "matter-of-fact",
                _ => "dry wit, slightly sarcastic",
            }
        };

        // Build vocabulary from active traits
        let mut vocabulary = vec!["quest".to_string(), "zone".to_string()];
        if friendly {
            vocabulary.extend(["hey".to_string(), "awesome".to_string(), "nice".to_string()]);
        }
        if aggressive {
            vocabulary.extend(["DPS".to_string(), "pull".to_string(), "wipe".to_string()]);
        }
        if scholarly {
            vocabulary.extend(["lore".to_string(), "ancient".to_string(), "theory".to_string()]);
        }
        if social {
            vocabulary.extend(["group".to_string(), "guild".to_string(), "LFG".to_string()]);
        }
        if is_grumpy {
            vocabulary.extend(["back in my day".to_string(), "noob".to_string()]);
        }

        // Speech patterns
        let mut patterns = Vec::new();
        if friendly {
            patterns.push("uses encouraging language".to_string());
        }
        if aggressive {
            patterns.push("talks about combat and strategy".to_string());
        }
        if scholarly {
            patterns.push("references history and lore when relevant".to_string());
        }
        if social {
            patterns.push("asks others about themselves".to_string());
        }
        if is_grumpy {
            patterns.push("complains about how things used to be better".to_string());
        }
        if patterns.is_empty() {
            patterns.push("speaks casually like a fellow player".to_string());
        }

        // Knowledge areas
        let mut knowledge = vec!["general Erenshor world knowledge".to_string()];
        if aggressive {
            knowledge.push("combat tactics and enemy weaknesses".to_string());
        }
        if scholarly {
            knowledge.push("ancient lore and magical theory".to_string());
        }
        if social {
            knowledge.push("guild dynamics and group coordination".to_string());
        }

        Self {
            name: name.to_string(),
            archetype: archetype.to_string(),
            tone: tone.to_string(),
            vocabulary,
            speech_patterns: patterns,
            knowledge_areas: knowledge,
            quirks: Vec::new(),
            example_phrases: Vec::new(),
            style_quirks: None,
        }
    }
}

/// Store of personality profiles loaded from JSON files.
///
/// For sims without a personality file, generates a personality on-the-fly
/// from game trait flags + name hash and caches it for the session.
pub struct PersonalityStore {
    personalities: HashMap<String, Personality>,
    /// Cache of dynamically generated personalities for unknown sims.
    generated: RwLock<HashMap<String, Personality>>,
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
                generated: RwLock::new(HashMap::new()),
                default,
            };
        }

        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                warn!("Failed to read personality directory: {}", e);
                return Self {
                    personalities,
                    generated: RwLock::new(HashMap::new()),
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
            generated: RwLock::new(HashMap::new()),
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

    /// Look up or dynamically generate a personality for a SimPlayer.
    ///
    /// 1. Returns the file-loaded personality if one exists.
    /// 2. Otherwise, generates a unique personality from the game's trait flags
    ///    and the sim name hash, caches it, and returns a clone.
    ///
    /// This ensures game-created SimPlayers (which don't have personality files)
    /// still get distinct, consistent personalities based on the traits the game
    /// assigns them.
    pub fn get_or_generate(
        &self,
        sim_name: &str,
        game_traits: &HashMap<String, bool>,
    ) -> Personality {
        let key = sim_name.to_lowercase();

        // 1. Check file-loaded personalities
        if let Some(p) = self.personalities.get(&key) {
            return p.clone();
        }

        // 2. Check generation cache
        if let Ok(cache) = self.generated.read() {
            if let Some(p) = cache.get(&key) {
                return p.clone();
            }
        }

        // 3. Generate from game traits + name hash
        let personality = Personality::generate(sim_name, game_traits);
        debug!(
            "Generated personality for unknown sim '{}': archetype='{}', tone='{}'",
            sim_name, personality.archetype, personality.tone
        );

        // 4. Cache it
        if let Ok(mut cache) = self.generated.write() {
            cache.insert(key, personality.clone());
        }

        personality
    }

    /// Whether a named personality exists (not the default fallback).
    pub fn has(&self, sim_name: &str) -> bool {
        self.personalities.contains_key(&sim_name.to_lowercase())
    }

    /// Total number of loaded personalities (including default).
    pub fn count(&self) -> usize {
        self.personalities.len() + 1
    }

    /// Number of dynamically generated personalities in the cache.
    pub fn generated_count(&self) -> usize {
        self.generated.read().map(|c| c.len()).unwrap_or(0)
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
