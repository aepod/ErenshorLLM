//! Offline template validation for quality assurance.
//!
//! Validates generated templates against:
//! 1. Format validation (required fields, valid types, value ranges)
//! 2. Entity grounding (zone_affinity must match grounding.json zones)
//! 3. Quality filtering (length, forbidden AI/meta phrases)
//! 4. Duplicate ID detection
//! 5. Category balance (no single category >25% of total)
//!
//! Usage: `erenshor-llm validate-templates --data-dir data/`

use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tracing::{info, warn};

use crate::intelligence::templates::{RawTemplate, RawTemplateFile};

// ─── Constants ───────────────────────────────────────────────────────────────

const MIN_LENGTH: usize = 15;
const MAX_LENGTH: usize = 300;
const MAX_CATEGORY_RATIO: f64 = 0.25;

const VALID_CHANNELS: &[&str] = &["say", "whisper", "party", "guild", "shout", "hail"];

/// AI/meta tells that should never appear in NPC dialog.
const FORBIDDEN_PHRASES: &[&str] = &[
    "as an ai",
    "language model",
    "i'm sorry but",
    "i cannot",
    "i apologize",
    "as a large language",
    "i don't have the ability",
    "my training data",
    "openai",
    "anthropic",
    "chatgpt",
    "claude",
    "artificial intelligence",
    "machine learning",
];

// ─── Grounding data ──────────────────────────────────────────────────────────

/// Entity lists from grounding.json for validation.
#[derive(Debug, Default)]
struct GroundingData {
    zones: HashSet<String>,
    items: HashSet<String>,
    enemies: HashSet<String>,
    npcs: HashSet<String>,
    quests: HashSet<String>,
    factions: HashSet<String>,
    classes: HashSet<String>,
}

impl GroundingData {
    fn load(data_dir: &Path) -> Self {
        let path = data_dir.join("grounding.json");
        if !path.exists() {
            warn!("grounding.json not found at {:?}, skipping entity grounding", path);
            return Self::default();
        }

        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read grounding.json: {}", e);
                return Self::default();
            }
        };

        let raw: HashMap<String, serde_json::Value> = match serde_json::from_str(&contents) {
            Ok(v) => v,
            Err(e) => {
                warn!("Failed to parse grounding.json: {}", e);
                return Self::default();
            }
        };

        fn extract_set(raw: &HashMap<String, serde_json::Value>, key: &str) -> HashSet<String> {
            raw.get(key)
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.to_lowercase())
                        .collect()
                })
                .unwrap_or_default()
        }

        Self {
            zones: extract_set(&raw, "zones"),
            items: extract_set(&raw, "items"),
            enemies: extract_set(&raw, "enemies"),
            npcs: extract_set(&raw, "npcs"),
            quests: extract_set(&raw, "quests"),
            factions: extract_set(&raw, "factions"),
            classes: extract_set(&raw, "classes"),
        }
    }

    fn has_zones(&self) -> bool {
        !self.zones.is_empty()
    }
}

// ─── Validation result ───────────────────────────────────────────────────────

/// Accumulated validation results.
#[derive(Debug, Default)]
pub struct ValidationReport {
    pub total_templates: usize,
    pub valid_templates: usize,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl ValidationReport {
    fn error(&mut self, msg: String) {
        self.errors.push(msg);
    }

    fn warn(&mut self, msg: String) {
        self.warnings.push(msg);
    }

    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    /// Print a human-readable summary to stdout.
    pub fn print_summary(&self) {
        println!();
        println!("{}", "=".repeat(60));
        println!("Validation Summary");
        println!("{}", "=".repeat(60));
        println!("Total templates: {}", self.total_templates);
        println!("Valid templates: {}", self.valid_templates);
        println!("Errors: {}", self.errors.len());
        println!("Warnings: {}", self.warnings.len());

        if !self.errors.is_empty() {
            println!("\nErrors ({}):", self.errors.len());
            for (i, err) in self.errors.iter().enumerate() {
                if i >= 50 {
                    println!("  ... and {} more", self.errors.len() - 50);
                    break;
                }
                println!("  ERROR: {}", err);
            }
        }

        if !self.warnings.is_empty() {
            println!("\nWarnings ({}):", self.warnings.len());
            for (i, w) in self.warnings.iter().enumerate() {
                if i >= 30 {
                    println!("  ... and {} more", self.warnings.len() - 30);
                    break;
                }
                println!("  WARN: {}", w);
            }
        }
    }
}

// ─── Single-template validation ──────────────────────────────────────────────

/// Validate a single template against all rules.
/// Returns true if the template passes (no errors).
fn validate_template(
    tmpl: &RawTemplate,
    grounding: &GroundingData,
    seen_ids: &mut HashSet<String>,
    report: &mut ValidationReport,
) -> bool {
    let mut valid = true;
    let tid = &tmpl.id;

    // Required fields: id, text, context_tags (checked at parse time,
    // but verify non-empty)
    if tid.is_empty() {
        report.error("[<missing>] Empty template ID".to_string());
        valid = false;
    }
    if tmpl.text.is_empty() {
        report.error(format!("[{}] Empty text field", tid));
        valid = false;
    }
    if tmpl.context_tags.is_empty() {
        report.error(format!("[{}] Empty context_tags array", tid));
        valid = false;
    }

    // Length check
    let text_len = tmpl.text.len();
    if text_len < MIN_LENGTH {
        report.error(format!(
            "[{}] Too short ({} chars, min {}): '{}'",
            tid, text_len, MIN_LENGTH, tmpl.text
        ));
        valid = false;
    } else if text_len > MAX_LENGTH {
        report.warn(format!(
            "[{}] Too long ({} chars, max {}): '{}...'",
            tid,
            text_len,
            MAX_LENGTH,
            &tmpl.text[..50.min(tmpl.text.len())]
        ));
    }

    // Forbidden phrases
    let text_lower = tmpl.text.to_lowercase();
    for phrase in FORBIDDEN_PHRASES {
        if text_lower.contains(phrase) {
            report.error(format!(
                "[{}] Contains forbidden phrase '{}': '{}...'",
                tid,
                phrase,
                &tmpl.text[..60.min(tmpl.text.len())]
            ));
            valid = false;
        }
    }

    // Duplicate ID
    if seen_ids.contains(tid) {
        report.error(format!("[{}] Duplicate template ID", tid));
        valid = false;
    }
    seen_ids.insert(tid.clone());

    // Channel validation
    for ch in &tmpl.channel {
        if !VALID_CHANNELS.contains(&ch.to_lowercase().as_str()) {
            report.warn(format!(
                "[{}] Invalid channel '{}' (valid: {:?})",
                tid, ch, VALID_CHANNELS
            ));
        }
    }

    // Relationship range
    if tmpl.relationship_min > tmpl.relationship_max {
        report.error(format!(
            "[{}] relationship_min ({}) > relationship_max ({})",
            tid, tmpl.relationship_min, tmpl.relationship_max
        ));
        valid = false;
    }

    // Priority
    if tmpl.priority <= 0.0 {
        report.warn(format!("[{}] Invalid priority: {}", tid, tmpl.priority));
    }

    // sim_name validation
    if let Some(ref name) = tmpl.sim_name {
        if name.is_empty() {
            report.error(format!("[{}] sim_name is empty string (use null instead)", tid));
            valid = false;
        }
    }

    // Zone affinity grounding
    if grounding.has_zones() {
        for zone in &tmpl.zone_affinity {
            if !grounding.zones.contains(&zone.to_lowercase()) {
                report.warn(format!(
                    "[{}] Unknown zone in zone_affinity: '{}'",
                    tid, zone
                ));
            }
        }
    }

    valid
}

// ─── File-level validation ───────────────────────────────────────────────────

/// Validate a single template JSON file.
fn validate_file(
    filepath: &Path,
    grounding: &GroundingData,
    seen_ids: &mut HashSet<String>,
    report: &mut ValidationReport,
) -> (usize, usize) {
    let contents = match std::fs::read_to_string(filepath) {
        Ok(c) => c,
        Err(e) => {
            report.error(format!(
                "[{}] Failed to read: {}",
                filepath.display(),
                e
            ));
            return (0, 0);
        }
    };

    let data: RawTemplateFile = match serde_json::from_str(&contents) {
        Ok(d) => d,
        Err(e) => {
            report.error(format!(
                "[{}] Invalid JSON: {}",
                filepath.display(),
                e
            ));
            return (0, 0);
        }
    };

    if data.templates.is_empty() {
        report.warn(format!(
            "[{}] Empty templates array",
            filepath.display()
        ));
        return (0, 0);
    }

    let mut valid_count = 0;
    for tmpl in &data.templates {
        if validate_template(tmpl, grounding, seen_ids, report) {
            valid_count += 1;
        }
    }

    (data.templates.len(), valid_count)
}

// ─── Category balance ────────────────────────────────────────────────────────

/// Check that no single category exceeds 25% of total templates.
fn check_category_balance(
    templates_dir: &Path,
    report: &mut ValidationReport,
) {
    let mut category_counts: HashMap<String, usize> = HashMap::new();
    let mut total = 0usize;

    let files = collect_json_files_recursive(templates_dir);
    for path in &files {
        let contents = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let data: RawTemplateFile = match serde_json::from_str(&contents) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let count = data.templates.len();
        *category_counts.entry(data.category).or_default() += count;
        total += count;
    }

    if total == 0 {
        return;
    }

    println!("\nCategory distribution ({} total):", total);

    let mut sorted: Vec<_> = category_counts.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));

    for (cat, count) in &sorted {
        let ratio = **count as f64 / total as f64;
        let marker = if ratio > MAX_CATEGORY_RATIO { " !!!" } else { "" };
        println!("  {}: {} ({:.1}%){}", cat, count, ratio * 100.0, marker);
        if ratio > MAX_CATEGORY_RATIO {
            report.warn(format!(
                "Category '{}' has {}/{} templates ({:.1}%), exceeds {:.0}% threshold",
                cat, count, total, ratio * 100.0, MAX_CATEGORY_RATIO * 100.0
            ));
        }
    }
}

// ─── Utilities ───────────────────────────────────────────────────────────────

/// Recursively collect all .json files from a directory.
fn collect_json_files_recursive(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                files.extend(collect_json_files_recursive(&path));
            } else if path.extension().map_or(false, |e| e == "json") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

// ─── Public entry point ──────────────────────────────────────────────────────

/// Validate all template files in the data directory.
///
/// If `input_file` is Some, validates only that file.
/// Otherwise validates all templates under `data_dir/templates/`.
///
/// Returns a ValidationReport with errors, warnings, and stats.
pub fn validate_templates(
    data_dir: &Path,
    input_file: Option<&Path>,
    check_grounding: bool,
) -> Result<ValidationReport> {
    let mut report = ValidationReport::default();
    let mut seen_ids = HashSet::new();

    // Load grounding data
    let grounding = if check_grounding {
        GroundingData::load(data_dir)
    } else {
        // Still load zones for zone_affinity validation
        GroundingData::load(data_dir)
    };

    if let Some(filepath) = input_file {
        // Validate single file
        let (total, valid) = validate_file(filepath, &grounding, &mut seen_ids, &mut report);
        report.total_templates += total;
        report.valid_templates += valid;
    } else {
        // Validate all template files
        let templates_dir = data_dir.join("templates");
        if !templates_dir.exists() {
            anyhow::bail!("Templates directory not found: {:?}", templates_dir);
        }

        let files = collect_json_files_recursive(&templates_dir);
        for path in &files {
            let (total, valid) = validate_file(path, &grounding, &mut seen_ids, &mut report);
            report.total_templates += total;
            report.valid_templates += valid;

            if total > 0 {
                let rel = path.strip_prefix(data_dir).unwrap_or(path);
                println!("  {}: {}/{} valid", rel.display(), valid, total);
            }
        }

        // Category balance check (only for full validation)
        check_category_balance(&templates_dir, &mut report);
    }

    info!(
        "Validation complete: {}/{} valid, {} errors, {} warnings",
        report.valid_templates,
        report.total_templates,
        report.errors.len(),
        report.warnings.len()
    );

    Ok(report)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intelligence::templates::RawTemplate;

    fn make_template(id: &str, text: &str) -> RawTemplate {
        RawTemplate {
            id: id.to_string(),
            text: text.to_string(),
            context_tags: vec!["test".to_string()],
            zone_affinity: vec![],
            personality_affinity: vec![],
            relationship_min: 0.0,
            relationship_max: 10.0,
            channel: vec![],
            priority: 1.0,
            sim_name: None,
        }
    }

    #[test]
    fn test_valid_template() {
        let tmpl = make_template("test_001", "The Blades of Vitheo guard the western coast.");
        let grounding = GroundingData::default();
        let mut seen = HashSet::new();
        let mut report = ValidationReport::default();

        assert!(validate_template(&tmpl, &grounding, &mut seen, &mut report));
        assert!(report.errors.is_empty());
    }

    #[test]
    fn test_too_short() {
        let tmpl = make_template("short_001", "Hi.");
        let grounding = GroundingData::default();
        let mut seen = HashSet::new();
        let mut report = ValidationReport::default();

        assert!(!validate_template(&tmpl, &grounding, &mut seen, &mut report));
        assert!(report.errors.iter().any(|e| e.contains("Too short")));
    }

    #[test]
    fn test_forbidden_phrase() {
        let tmpl = make_template("ai_001", "As an AI, I cannot help you with that quest.");
        let grounding = GroundingData::default();
        let mut seen = HashSet::new();
        let mut report = ValidationReport::default();

        assert!(!validate_template(&tmpl, &grounding, &mut seen, &mut report));
        assert!(report.errors.iter().any(|e| e.contains("forbidden phrase")));
    }

    #[test]
    fn test_duplicate_id() {
        let tmpl = make_template("dup_001", "First template with this ID, fairly long.");
        let grounding = GroundingData::default();
        let mut seen = HashSet::new();
        let mut report = ValidationReport::default();

        assert!(validate_template(&tmpl, &grounding, &mut seen, &mut report));

        let tmpl2 = make_template("dup_001", "Second template with same ID, also long enough.");
        assert!(!validate_template(&tmpl2, &grounding, &mut seen, &mut report));
        assert!(report.errors.iter().any(|e| e.contains("Duplicate")));
    }

    #[test]
    fn test_bad_relationship_range() {
        let mut tmpl = make_template("rel_001", "Relationship range is inverted on this one.");
        tmpl.relationship_min = 8.0;
        tmpl.relationship_max = 3.0;
        let grounding = GroundingData::default();
        let mut seen = HashSet::new();
        let mut report = ValidationReport::default();

        assert!(!validate_template(&tmpl, &grounding, &mut seen, &mut report));
        assert!(report.errors.iter().any(|e| e.contains("relationship_min")));
    }

    #[test]
    fn test_unknown_zone() {
        let mut tmpl = make_template("zone_001", "Zone affinity references a fake zone name.");
        tmpl.zone_affinity = vec!["Fake Zone".to_string()];
        let mut grounding = GroundingData::default();
        grounding.zones.insert("port azure".to_string());
        let mut seen = HashSet::new();
        let mut report = ValidationReport::default();

        validate_template(&tmpl, &grounding, &mut seen, &mut report);
        assert!(report.warnings.iter().any(|w| w.contains("Unknown zone")));
    }
}
