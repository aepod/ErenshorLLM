//! Item file cleaner: transforms wiki-imported item stat dumps into
//! conversational prose suitable for LLM context injection.
//!
//! Reads each wiki item .md file, extracts structured data (stats, drops,
//! classes, flavor text), cross-references enemies/zones, and generates
//! a 3-6 sentence review-style summary.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, info};

/// Known player classes for parsing concatenated class restrictions.
const KNOWN_CLASSES: &[&str] = &[
    "Arcanist",
    "Druid",
    "Paladin",
    "Reaver",
    "Stormcaller",
    "Windblade",
];

/// Curated files that should not be cleaned (already good prose).
const SKIP_FILES: &[&str] = &[
    "weapons.md",
    "armor.md",
    "auras.md",
    "descriptions.md",
    "general.md",
];

/// Enemy info extracted from enemy .md files.
#[derive(Debug, Clone)]
struct EnemyInfo {
    #[allow(dead_code)]
    name: String,
    zone: String,
    level: u32,
    is_boss: bool,
    #[allow(dead_code)]
    faction: String,
}

/// Parsed item data extracted from a wiki item file.
#[derive(Debug, Default)]
struct ParsedItem {
    name: String,
    slot: String,
    // Stats (base tier only)
    str_val: i32,
    end_val: i32,
    dex_val: i32,
    agi_val: i32,
    int_val: i32,
    wis_val: i32,
    cha_val: i32,
    res_val: i32,
    // Weapon stats
    damage: i32,
    delay: f32,
    base_dps: i32,
    // Vitals
    health: i32,
    mana: i32,
    armor: i32,
    // Resists
    magic_resist: i32,
    poison_resist: i32,
    elemental_resist: i32,
    void_resist: i32,
    // Metadata
    flavor_text: String,
    classes: Vec<String>,
    drop_sources: Vec<(String, f32)>, // (enemy_name, drop_rate%)
    buy_price: u32,
    sell_price: u32,
    is_charm: bool,
    charm_mods: Vec<String>,
    is_consumable: bool,
    item_type: String, // "Chest", "Primary or Secondary", "Charm Item", etc.
    proc_info: String, // e.g. "5% chance on ATTACK: Dark Haste"
}

/// Parse YAML frontmatter from a wiki item file.
/// Returns (title, source, categories, lore_category, body).
fn parse_frontmatter(content: &str) -> Option<(String, String, Vec<String>, String, String)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }

    let after_first = &trimmed[3..];
    let end_idx = after_first.find("\n---")?;
    let yaml_block = &after_first[..end_idx];
    let body_start = 3 + end_idx + 4;
    let body = if body_start < trimmed.len() {
        trimmed[body_start..].to_string()
    } else {
        String::new()
    };

    let mut title = String::new();
    let mut source = String::new();
    let mut categories = Vec::new();
    let mut lore_category = String::new();

    for line in yaml_block.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("title:") {
            title = val.trim().trim_matches('"').to_string();
        } else if let Some(val) = line.strip_prefix("source:") {
            source = val.trim().trim_matches('"').to_string();
        } else if let Some(val) = line.strip_prefix("lore_category:") {
            lore_category = val.trim().trim_matches('"').to_string();
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

    Some((title, source, categories, lore_category, body))
}

/// Parse concatenated class string like "PaladinReaverWindblade" into individual class names.
fn parse_classes(class_str: &str) -> Vec<String> {
    let trimmed = class_str.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let mut classes = Vec::new();
    let mut remaining = trimmed;

    while !remaining.is_empty() {
        let mut matched = false;
        for &class in KNOWN_CLASSES {
            if remaining.starts_with(class) {
                classes.push(class.to_string());
                remaining = &remaining[class.len()..];
                matched = true;
                break;
            }
        }
        if !matched {
            // Unknown content, skip one character (char-boundary safe for UTF-8)
            let skip = remaining.chars().next().map_or(1, |c| c.len_utf8());
            remaining = &remaining[skip..];
        }
    }

    classes
}

/// Parse an item file body into structured data.
fn parse_item(name: &str, body: &str) -> ParsedItem {
    let mut item = ParsedItem {
        name: name.to_string(),
        ..Default::default()
    };

    let lines: Vec<&str> = body.lines().collect();
    let mut i = 0;
    let mut in_first_tier = true; // Only parse the base tier stats
    let mut tier_count = 0;
    let mut found_stats_section = false;
    let mut candidate_flavor_lines: Vec<String> = Vec::new();

    while i < lines.len() {
        let line = lines[i].trim();
        let line_lower = line.to_lowercase();

        // Track tier boundaries: the item name repeated signals a new tier
        if i > 0 && line == name && found_stats_section {
            tier_count += 1;
            if tier_count >= 1 {
                in_first_tier = false;
            }
        }

        // Drop sources
        if line == "### Dropped by" {
            i += 1;
            // Collect all drop sources until next section
            while i < lines.len() {
                let drop_line = lines[i].trim();
                if drop_line.starts_with("###") || drop_line.starts_with("##") {
                    break;
                }
                if !drop_line.is_empty() {
                    // Parse "Enemy Name (X.X%)"
                    if let Some(paren_start) = drop_line.rfind('(') {
                        let enemy = drop_line[..paren_start].trim().to_string();
                        let rate_str = drop_line[paren_start + 1..]
                            .trim_end_matches(')')
                            .trim_end_matches('%')
                            .trim();
                        let rate = rate_str.parse::<f32>().unwrap_or(0.0);
                        if !enemy.is_empty() {
                            item.drop_sources.push((enemy, rate));
                        }
                    } else if !drop_line.is_empty() {
                        item.drop_sources.push((drop_line.to_string(), 0.0));
                    }
                }
                i += 1;
            }
            continue;
        }

        // Buy/Sell prices
        if line == "### Buy" {
            i += 1;
            while i < lines.len() {
                let val = lines[i].trim();
                if val.starts_with("###") || val.starts_with("##") { break; }
                if !val.is_empty() {
                    item.buy_price = val.parse().unwrap_or(0);
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }

        if line == "### Sell" {
            i += 1;
            while i < lines.len() {
                let val = lines[i].trim();
                if val.starts_with("###") || val.starts_with("##") { break; }
                if !val.is_empty() {
                    item.sell_price = val.parse().unwrap_or(0);
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }

        // Only parse stats from the first tier
        if in_first_tier {
            // Slot
            if let Some(slot) = line.strip_prefix("Slot:") {
                item.slot = slot.trim().to_string();
            }

            // Item type line (e.g. "Primary or Secondary \- Relic Item", "Charm Item")
            if line.contains("Charm Item") {
                item.is_charm = true;
                item.item_type = "Charm".to_string();
            } else if line.contains("Primary or Secondary") {
                item.item_type = "Weapon".to_string();
            }

            if line == "Item Stats" {
                found_stats_section = true;
            }

            // Stat lines
            if let Some(val) = line.strip_prefix("Str ") {
                item.str_val = val.trim().parse().unwrap_or(0);
            } else if let Some(val) = line.strip_prefix("End ") {
                item.end_val = val.trim().parse().unwrap_or(0);
            } else if let Some(val) = line.strip_prefix("Dex ") {
                item.dex_val = val.trim().parse().unwrap_or(0);
            } else if let Some(val) = line.strip_prefix("Agi ") {
                item.agi_val = val.trim().parse().unwrap_or(0);
            } else if let Some(val) = line.strip_prefix("Int ") {
                item.int_val = val.trim().parse().unwrap_or(0);
            } else if let Some(val) = line.strip_prefix("Wis ") {
                item.wis_val = val.trim().parse().unwrap_or(0);
            } else if let Some(val) = line.strip_prefix("Cha ") {
                item.cha_val = val.trim().parse().unwrap_or(0);
            } else if let Some(val) = line.strip_prefix("Res ") {
                item.res_val = val.trim().parse().unwrap_or(0);
            }

            // Weapon stats
            if let Some(val) = line.strip_prefix("Damage ") {
                item.damage = val.trim().parse().unwrap_or(0);
            }
            if let Some(val) = line.strip_prefix("Delay ") {
                item.delay = val.trim().trim_end_matches(" sec").parse().unwrap_or(0.0);
            }
            if let Some(val) = line.strip_prefix("Base DPS:") {
                item.base_dps = val.trim().parse().unwrap_or(0);
            }

            // Vitals
            if let Some(val) = line.strip_prefix("Health ") {
                if !line_lower.contains("hitpoints") {
                    item.health = val.trim().parse().unwrap_or(0);
                }
            }
            if let Some(val) = line.strip_prefix("Mana ") {
                item.mana = val.trim().parse().unwrap_or(0);
            }
            if let Some(val) = line.strip_prefix("Armor ") {
                item.armor = val.trim().parse().unwrap_or(0);
            }

            // Resists
            if let Some(val) = line.strip_prefix("Magic +") {
                item.magic_resist = val.trim_end_matches('%').parse().unwrap_or(0);
            }
            if let Some(val) = line.strip_prefix("Poison +") {
                item.poison_resist = val.trim_end_matches('%').parse().unwrap_or(0);
            }
            if let Some(val) = line.strip_prefix("Elemental +") {
                item.elemental_resist = val.trim_end_matches('%').parse().unwrap_or(0);
            }
            if let Some(val) = line.strip_prefix("Void +") {
                item.void_resist = val.trim_end_matches('%').parse().unwrap_or(0);
            }

            // Proc info
            if line.contains("% chance on ATTACK:") || line.contains("% chance on") {
                item.proc_info = line.to_string();
            }

            // Charm modifiers
            if line.starts_with("Hardiness:") || line.starts_with("Finesse:")
                || line.starts_with("Arcanism:") || line.starts_with("Restoration:")
            {
                item.charm_mods.push(line.to_string());
            }

            // Class line (concatenated classes)
            let parsed_classes = parse_classes(line);
            if parsed_classes.len() >= 2 {
                item.classes = parsed_classes;
            }

            // Consumable detection
            if line_lower.contains("consumable") || line_lower.contains("activatable:") {
                item.is_consumable = true;
            }

            // Candidate flavor text: lines that aren't stats/headers/labels
            if found_stats_section
                && !line.is_empty()
                && !line.starts_with('#')
                && !line.starts_with("Str ") && !line.starts_with("End ")
                && !line.starts_with("Dex ") && !line.starts_with("Agi ")
                && !line.starts_with("Int ") && !line.starts_with("Wis ")
                && !line.starts_with("Cha ") && !line.starts_with("Res ")
                && !line.starts_with("Damage ") && !line.starts_with("Delay ")
                && !line.starts_with("Health ") && !line.starts_with("Mana ")
                && !line.starts_with("Armor ") && !line.starts_with("Magic +")
                && !line.starts_with("Poison +") && !line.starts_with("Elemental +")
                && !line.starts_with("Void +") && !line.starts_with("Base DPS:")
                && !line.starts_with("Slot:") && !line.starts_with("Item Stats")
                && !line.starts_with("Vitals") && !line.starts_with("Resists")
                && !line.starts_with("Class modifiers:")
                && !line.starts_with("Hardiness:") && !line.starts_with("Finesse:")
                && !line.starts_with("Arcanism:") && !line.starts_with("Restoration:")
                && !line.contains("Charm Item") && !line.contains("Primary or Secondary")
                && !line.contains("% chance on") && !line.starts_with("Spell ")
                && !line.starts_with("Effect Duration:") && !line.starts_with("Cast Time:")
                && !line.starts_with("Haste +") && !line.starts_with("Attack Roll")
                && !line.starts_with("Hitpoints +") && !line.starts_with("Mana +")
                && !line.starts_with("Movement Speed")
                && !line.contains("Right click or assign")
                && !line.contains("Item Consumed Upon Use")
                && !line.contains("Charms do not increase stats")
                && line != name
                && parse_classes(line).len() < 2
            {
                // This might be flavor text
                if line.len() > 15 && !line.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ' ') {
                    candidate_flavor_lines.push(line.to_string());
                }
            }
        }

        i += 1;
    }

    // Pick the best flavor text (longest candidate from the first tier)
    if let Some(best) = candidate_flavor_lines
        .iter()
        .max_by_key(|s| s.len())
    {
        item.flavor_text = best.clone();
    }

    item
}

/// Load enemy info from enemy .md files.
fn load_enemies(enemies_dir: &Path) -> HashMap<String, EnemyInfo> {
    let mut enemies = HashMap::new();

    let entries = match std::fs::read_dir(enemies_dir) {
        Ok(e) => e,
        Err(_) => return enemies,
    };

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().map_or(true, |e| e != "md") {
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let (title, _, _, _, body) = match parse_frontmatter(&content) {
            Some(f) => f,
            None => continue,
        };

        let mut zone = String::new();
        let mut level: u32 = 0;
        let mut is_boss = false;
        let mut faction = String::new();

        for line in body.lines() {
            let line = line.trim();
            if line == "Boss" {
                is_boss = true;
            }
            // Zone parsing: line after "### Zones:"
            if line.starts_with("### Zones") {
                // Next non-empty line is the zone
                continue;
            }
            // Try to capture zone from content
            if !line.is_empty() && !line.starts_with('#') && zone.is_empty() {
                // Check if previous line was "### Zones:"
                // We'll use a simpler approach: look for specific patterns
            }
        }

        // More robust zone/level/faction extraction
        let lines: Vec<&str> = body.lines().collect();
        for i in 0..lines.len() {
            let line = lines[i].trim();
            if line.starts_with("### Zones") || line == "### Zones:" {
                // Next non-empty line is the zone
                for j in (i + 1)..lines.len() {
                    let next = lines[j].trim();
                    if !next.is_empty() && !next.starts_with('#') {
                        zone = next.to_string();
                        break;
                    }
                }
            }
            if line.starts_with("### Base Level") || line == "### Base Level:" {
                for j in (i + 1)..lines.len() {
                    let next = lines[j].trim();
                    if !next.is_empty() && !next.starts_with('#') {
                        level = next.parse().unwrap_or(0);
                        break;
                    }
                }
            }
            if line.starts_with("### Faction") || line == "### Faction:" {
                for j in (i + 1)..lines.len() {
                    let next = lines[j].trim();
                    if !next.is_empty() && !next.starts_with('#') {
                        faction = next.to_string();
                        break;
                    }
                }
            }
            if line.starts_with("### Type") || line == "### Type:" {
                for j in (i + 1)..lines.len() {
                    let next = lines[j].trim();
                    if !next.is_empty() && !next.starts_with('#') {
                        if next == "Boss" {
                            is_boss = true;
                        }
                        break;
                    }
                }
            }
        }

        enemies.insert(
            title.clone(),
            EnemyInfo {
                name: title,
                zone,
                level,
                is_boss,
                faction,
            },
        );
    }

    info!("Loaded {} enemies for cross-reference", enemies.len());
    enemies
}

/// Get the level tier description.
fn level_tier(level: u32) -> &'static str {
    match level {
        0 => "miscellaneous",
        1..=8 => "starter",
        9..=20 => "mid-level",
        21..=30 => "solid upgrade",
        31..=39 => "endgame",
        _ => "best-in-slot",
    }
}

/// Infer item level from enemy level or buy price.
fn infer_level(item: &ParsedItem, enemies: &HashMap<String, EnemyInfo>) -> u32 {
    // Try enemy level first
    for (enemy_name, _) in &item.drop_sources {
        if let Some(info) = enemies.get(enemy_name) {
            if info.level > 0 {
                return info.level;
            }
        }
    }

    // Fall back to price-based heuristic
    if item.buy_price > 0 {
        return (item.buy_price / 1000).clamp(1, 45) as u32;
    }

    // Rough stat-based heuristic
    let total_stats = item.str_val + item.end_val + item.dex_val + item.agi_val
        + item.int_val + item.wis_val;
    if total_stats > 0 {
        return (total_stats as u32 / 5).clamp(1, 45);
    }

    0
}

/// Build the standout stat description.
fn standout_stats(item: &ParsedItem) -> String {
    let mut highlights = Vec::new();

    // Find highest primary stat
    let stats = [
        ("Strength", item.str_val),
        ("Endurance", item.end_val),
        ("Dexterity", item.dex_val),
        ("Agility", item.agi_val),
        ("Intelligence", item.int_val),
        ("Wisdom", item.wis_val),
        ("Charisma", item.cha_val),
    ];

    let mut best_stats: Vec<(&str, i32)> = stats.iter()
        .filter(|(_, v)| *v > 0)
        .map(|(n, v)| (*n, *v))
        .collect();
    best_stats.sort_by(|a, b| b.1.cmp(&a.1));

    if let Some((name, val)) = best_stats.first() {
        if *val >= 20 {
            highlights.push(format!("massive {} (+{})", name, val));
        } else if *val >= 10 {
            highlights.push(format!("strong {} (+{})", name, val));
        } else {
            highlights.push(format!("{} +{}", name, val));
        }
    }
    if best_stats.len() > 1 {
        let (name, val) = best_stats[1];
        if val >= 10 {
            highlights.push(format!("{} +{}", name, val));
        }
    }

    // Resists
    let resists = [
        ("Magic", item.magic_resist),
        ("Poison", item.poison_resist),
        ("Elemental", item.elemental_resist),
        ("Void", item.void_resist),
    ];
    for (name, val) in &resists {
        if *val > 0 {
            highlights.push(format!("{} resistance +{}%", name, val));
        }
    }

    // Health/Mana/Armor
    if item.health > 0 {
        highlights.push(format!("+{} Health", item.health));
    }
    if item.armor > 100 {
        highlights.push(format!("{} Armor", item.armor));
    }

    // Weapon stats
    if item.damage > 0 {
        highlights.push(format!("{} damage", item.damage));
    }

    if highlights.is_empty() {
        return String::new();
    }

    // Limit to top 3 highlights
    highlights.truncate(3);
    highlights.join(", ") + "."
}

/// Generate the prose summary for an item.
fn generate_prose(
    item: &ParsedItem,
    level: u32,
    enemies: &HashMap<String, EnemyInfo>,
    _same_slot_items: &[(String, u32)], // (name, level) -- same slot alternatives
) -> String {
    let mut parts: Vec<String> = Vec::new();

    let tier = level_tier(level);

    // Opening: item name, tier, slot, classes
    if item.is_charm {
        let class_str = if item.classes.is_empty() {
            "all classes".to_string()
        } else {
            format_class_list(&item.classes)
        };
        parts.push(format!(
            "{} is a charm item usable by {}.",
            item.name, class_str
        ));
    } else if item.is_consumable {
        parts.push(format!("{} is a consumable item.", item.name));
    } else {
        let slot_str = if item.slot.is_empty() && !item.item_type.is_empty() {
            item.item_type.clone()
        } else if !item.slot.is_empty() {
            item.slot.clone()
        } else {
            "gear".to_string()
        };

        let class_str = if item.classes.is_empty() {
            String::new()
        } else {
            format!(" for {}", format_class_list(&item.classes))
        };

        if level > 0 {
            parts.push(format!(
                "{} is a {} {} {}{}.",
                item.name, tier, slot_str.to_lowercase(), "piece", class_str
            ));
        } else {
            parts.push(format!(
                "{} is a {} {}{}.",
                item.name, slot_str.to_lowercase(), "piece", class_str
            ));
        }
    }

    // Drop source with zone cross-reference
    if !item.drop_sources.is_empty() {
        let (ref enemy_name, rate) = item.drop_sources[0];
        let mut drop_str = if let Some(info) = enemies.get(enemy_name) {
            let boss_str = if info.is_boss { " (Boss)" } else { "" };
            let zone_str = if info.zone.is_empty() {
                String::new()
            } else {
                format!(" in {}", info.zone)
            };
            if rate > 0.0 {
                format!(
                    "Dropped by {}{}{} ({:.1}% drop rate).",
                    enemy_name, boss_str, zone_str, rate
                )
            } else {
                format!("Dropped by {}{}{}.", enemy_name, boss_str, zone_str)
            }
        } else {
            if rate > 0.0 {
                format!("Dropped by {} ({:.1}% drop rate).", enemy_name, rate)
            } else {
                format!("Dropped by {}.", enemy_name)
            }
        };

        // Additional drop sources
        if item.drop_sources.len() > 1 {
            let others: Vec<String> = item.drop_sources[1..]
                .iter()
                .take(2)
                .map(|(name, _)| name.clone())
                .collect();
            drop_str.push_str(&format!(" Also drops from {}.", others.join(" and ")));
        }

        parts.push(drop_str);
    } else if item.buy_price > 0 {
        parts.push(format!("Available from vendors for {}g.", item.buy_price));
    }

    // Standout stats
    let stats_desc = standout_stats(item);
    if !stats_desc.is_empty() {
        // Capitalize first char
        let mut chars = stats_desc.chars();
        let capitalized = match chars.next() {
            None => String::new(),
            Some(c) => c.to_uppercase().to_string() + chars.as_str(),
        };
        parts.push(capitalized);
    }

    // Charm modifiers
    if item.is_charm && !item.charm_mods.is_empty() {
        parts.push(format!("Modifiers: {}.", item.charm_mods.join(", ")));
    }

    // Proc info
    if !item.proc_info.is_empty() {
        parts.push(format!("Special: {}.", item.proc_info));
    }

    // Flavor text
    if !item.flavor_text.is_empty() {
        parts.push(format!("\"{}\"", item.flavor_text));
    }

    // Price info (if not already mentioned as vendor-only source)
    if item.buy_price > 0 && !item.drop_sources.is_empty() {
        parts.push(format!("Costs {}g from vendors.", item.buy_price));
    }

    parts.join(" ")
}

/// Format a list of class names: "Paladins, Reavers, and Windblades"
fn format_class_list(classes: &[String]) -> String {
    match classes.len() {
        0 => "all classes".to_string(),
        1 => format!("{}s", classes[0]),
        2 => format!("{}s and {}s", classes[0], classes[1]),
        _ => {
            let last = classes.last().unwrap();
            let rest: Vec<String> = classes[..classes.len() - 1]
                .iter()
                .map(|c| format!("{}s", c))
                .collect();
            format!("{}, and {}s", rest.join(", "), last)
        }
    }
}

/// Rebuild the frontmatter with additional fields.
fn rebuild_frontmatter(
    original_content: &str,
    level: u32,
    classes: &[String],
    slot: &str,
) -> String {
    let trimmed = original_content.trim_start();
    if !trimmed.starts_with("---") {
        return original_content.to_string();
    }

    let after_first = &trimmed[3..];
    if let Some(end_idx) = after_first.find("\n---") {
        let yaml_block = &after_first[..end_idx];
        let mut yaml_lines: Vec<String> = yaml_block.lines().map(|l| l.to_string()).collect();

        // Remove existing level/classes/slot if present
        yaml_lines.retain(|l| {
            !l.trim().starts_with("level:") && !l.trim().starts_with("classes:")
                && !l.trim().starts_with("slot:")
        });

        // Add new fields
        if level > 0 {
            yaml_lines.push(format!("level: {}", level));
        }
        if !classes.is_empty() {
            let classes_json: Vec<String> = classes.iter().map(|c| format!("\"{}\"", c)).collect();
            yaml_lines.push(format!("classes: [{}]", classes_json.join(", ")));
        }
        if !slot.is_empty() {
            yaml_lines.push(format!("slot: \"{}\"", slot));
        }

        format!("---\n{}\n---", yaml_lines.join("\n"))
    } else {
        original_content.to_string()
    }
}

/// Clean all wiki item files in the given directory.
///
/// Returns the number of files processed.
pub fn clean_items(items_dir: &Path, enemies_dir: &Path, _zones_dir: &Path) -> Result<usize> {
    info!("Cleaning item files in {:?}", items_dir);

    if !items_dir.exists() {
        anyhow::bail!("Items directory {:?} does not exist", items_dir);
    }

    // Load enemy cross-reference data
    let enemies = load_enemies(enemies_dir);

    // Collect all item files
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(items_dir)
        .context("Failed to read items directory")?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |e| e == "md"))
        .collect();
    files.sort();

    info!("Found {} item files to process", files.len());

    // First pass: parse all items to build cross-reference data
    let mut all_items: Vec<(std::path::PathBuf, ParsedItem, u32)> = Vec::new();

    for path in &files {
        let file_name = path.file_name().unwrap().to_string_lossy();

        // Skip curated files
        if SKIP_FILES.iter().any(|&s| file_name == s) {
            debug!("Skipping curated file: {}", file_name);
            continue;
        }

        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {:?}", path))?;

        // Skip curated-source files
        let (title, source, _categories, _lore_cat, body) = match parse_frontmatter(&content) {
            Some(f) => f,
            None => {
                debug!("Skipping file without frontmatter: {}", file_name);
                continue;
            }
        };

        if source == "curated" {
            debug!("Skipping curated source: {}", file_name);
            continue;
        }

        let item = parse_item(&title, &body);
        let level = infer_level(&item, &enemies);
        all_items.push((path.clone(), item, level));
    }

    // Build same-slot cross-reference (simplified: we don't generate cross-links
    // for now to keep the output concise)
    let empty_alternatives: Vec<(String, u32)> = Vec::new();

    // Second pass: generate prose and write files
    let mut processed = 0;

    for (path, item, level) in &all_items {
        let content = std::fs::read_to_string(path)?;

        let prose = generate_prose(item, *level, &enemies, &empty_alternatives);

        if prose.is_empty() {
            debug!("Skipping item with no meaningful content: {}", item.name);
            continue;
        }

        // Build new frontmatter with additional fields
        let slot = if !item.slot.is_empty() {
            &item.slot
        } else if !item.item_type.is_empty() {
            &item.item_type
        } else {
            ""
        };
        let new_frontmatter = rebuild_frontmatter(&content, *level, &item.classes, slot);

        // Write the cleaned file
        let new_content = format!("{}\n\n{}\n", new_frontmatter, prose);
        std::fs::write(path, new_content)
            .with_context(|| format!("Failed to write {:?}", path))?;

        processed += 1;
        if processed % 100 == 0 {
            info!("Cleaned {} items...", processed);
        }
    }

    info!("Item cleaning complete: {} files processed", processed);
    Ok(processed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_classes() {
        assert_eq!(
            parse_classes("PaladinReaverWindblade"),
            vec!["Paladin", "Reaver", "Windblade"]
        );
        assert_eq!(
            parse_classes("ArcanistDruidPaladinReaverStormcallerWindblade"),
            vec!["Arcanist", "Druid", "Paladin", "Reaver", "Stormcaller", "Windblade"]
        );
        assert_eq!(parse_classes(""), Vec::<String>::new());
    }

    #[test]
    fn test_level_tier() {
        assert_eq!(level_tier(5), "starter");
        assert_eq!(level_tier(15), "mid-level");
        assert_eq!(level_tier(25), "solid upgrade");
        assert_eq!(level_tier(35), "endgame");
        assert_eq!(level_tier(40), "best-in-slot");
    }

    #[test]
    fn test_format_class_list() {
        assert_eq!(
            format_class_list(&["Paladin".to_string()]),
            "Paladins"
        );
        assert_eq!(
            format_class_list(&["Paladin".to_string(), "Reaver".to_string()]),
            "Paladins and Reavers"
        );
        assert_eq!(
            format_class_list(&[
                "Paladin".to_string(),
                "Reaver".to_string(),
                "Windblade".to_string()
            ]),
            "Paladins, Reavers, and Windblades"
        );
    }

    #[test]
    fn test_parse_item_abyssal_plate() {
        let body = r#"
# Abyssal Plate

## Abyssal Plate

### Dropped by

Sivakayan Voidmaster (4.5%)

### Buy

38000

### Sell

24700

Abyssal Plate

Slot: Chest

Item Stats

Str 25

End 30

Dex 20

Agi 15

Int 0

Wis 0

Cha 0

Res 4

Vitals

Health 400

Mana 200

Armor 175

Resists

Magic +3%

Poison +3%

Elemental +3%

Void +6%

Physical blows are seemingly lost this metal, absorbed into nothingness

PaladinReaverWindblade

Abyssal Plate

Slot: Chest

Item Stats

Str 37
"#;
        let item = parse_item("Abyssal Plate", body);
        assert_eq!(item.name, "Abyssal Plate");
        assert_eq!(item.slot, "Chest");
        assert_eq!(item.str_val, 25);
        assert_eq!(item.end_val, 30);
        assert_eq!(item.void_resist, 6);
        assert_eq!(item.health, 400);
        assert_eq!(item.armor, 175);
        assert_eq!(item.buy_price, 38000);
        assert_eq!(item.classes, vec!["Paladin", "Reaver", "Windblade"]);
        assert!(item.flavor_text.contains("Physical blows"));
        assert_eq!(item.drop_sources.len(), 1);
        assert_eq!(item.drop_sources[0].0, "Sivakayan Voidmaster");
    }

    #[test]
    fn test_parse_charm_item() {
        let body = r#"
# Adventure Charm

## Adventure Charm

### Buy

2500

### Sell

1625

Adventure Charm

Charm Item

Class modifiers:
Hardiness: +1 / 40
Finesse: +1 / 40
Arcanism: +1 / 40
Restoration: +1 / 40

Charms do not increase stats, they modify how effectively your character utilizes stats.

ArcanistDruidPaladinReaverStormcallerWindblade
"#;
        let item = parse_item("Adventure Charm", body);
        assert!(item.is_charm);
        assert_eq!(item.buy_price, 2500);
        assert_eq!(item.classes.len(), 6);
    }
}
