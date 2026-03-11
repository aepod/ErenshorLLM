//! Dynamic template store for runtime-generated dialog templates.
//!
//! This is separate from `templates.rs` (ResponseStore) which handles
//! pre-authored Phase 2 templates. This module stores templates generated
//! by the LLM at runtime, with personality-aware lookup and cosine dedup.

use chrono::{DateTime, Utc};
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::config::TemplateConfig;
use crate::intelligence::embedder::EmbeddingEngine;
use crate::llm::cloud::ChatMessage;
use crate::llm::router::{LlmResult, LlmRouter};

/// Configuration for the dynamic template store.
#[derive(Debug, Clone)]
pub struct TemplateStoreConfig {
    /// Maximum number of variants per trigger key.
    pub max_variants_per_trigger: usize,
    /// Maximum total stored variants across all triggers.
    pub max_total_templates: usize,
    /// Cosine similarity threshold for deduplication (0.0-1.0).
    pub dedup_threshold: f32,
}

impl Default for TemplateStoreConfig {
    fn default() -> Self {
        Self {
            max_variants_per_trigger: 8,
            max_total_templates: 2000,
            dedup_threshold: 0.92,
        }
    }
}

/// Runtime-generated template store.
///
/// Stores dialog variants keyed by trigger (e.g. "pulling", "lfg", "greeting").
/// Each trigger has multiple variants with personality hints for matched selection.
#[derive(Debug, Serialize, Deserialize)]
pub struct TemplateStore {
    pub version: u32,
    pub templates: HashMap<String, TriggerGroup>,
    #[serde(skip)]
    dirty: bool,
    #[serde(skip)]
    config: TemplateStoreConfig,
}

/// A group of template variants for a single trigger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerGroup {
    pub variants: Vec<StoredVariant>,
    pub last_accessed: DateTime<Utc>,
}

/// A single stored template variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredVariant {
    pub text: String,
    pub personality: PersonalityHint,
    pub source_sim: String,
    pub channel: String,
    pub generated_at: DateTime<Utc>,
    pub last_used: DateTime<Utc>,
    pub use_count: u32,
}

/// Lightweight personality descriptor for template matching.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersonalityHint {
    pub style: String,
    pub class_role: String,
    pub traits: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_sim: Option<String>,
}

impl TemplateStore {
    /// Create a new empty template store.
    pub fn new(config: TemplateStoreConfig) -> Self {
        Self {
            version: 1,
            templates: HashMap::new(),
            dirty: false,
            config,
        }
    }

    /// Load from a JSON file on disk. Returns an empty store if the file
    /// is missing or corrupt.
    pub fn load(path: &Path, config: TemplateStoreConfig) -> Self {
        if !path.exists() {
            info!("Template store file not found at {:?}, starting empty", path);
            return Self::new(config);
        }

        match std::fs::read_to_string(path) {
            Ok(contents) => match serde_json::from_str::<TemplateStore>(&contents) {
                Ok(mut store) => {
                    store.config = config;
                    store.dirty = false;
                    info!(
                        "Template store loaded: {} triggers, {} total variants",
                        store.templates.len(),
                        store.total_variant_count()
                    );
                    store
                }
                Err(e) => {
                    warn!("Corrupt template store at {:?}: {}. Starting empty.", path, e);
                    Self::new(config)
                }
            },
            Err(e) => {
                warn!("Failed to read template store {:?}: {}. Starting empty.", path, e);
                Self::new(config)
            }
        }
    }

    /// Atomic persist: write to .tmp, rename existing to .bak, rename .tmp to target.
    pub fn persist(&self, path: &Path) -> std::io::Result<()> {
        let tmp_path = path.with_extension("json.tmp");
        let bak_path = path.with_extension("json.bak");

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Write to .tmp
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(&tmp_path, json)?;

        // Rename existing to .bak (ignore error if file doesn't exist)
        if path.exists() {
            let _ = std::fs::rename(path, &bak_path);
        }

        // Rename .tmp to target
        std::fs::rename(&tmp_path, path)?;

        debug!("Template store persisted to {:?}", path);
        Ok(())
    }

    /// Look up a template variant for the given trigger and personality hint.
    ///
    /// Personality matching: prefers variants whose style or class_role matches
    /// the hint. Falls back to a random variant if no personality match is found.
    /// Updates last_accessed, last_used, and use_count on the selected variant.
    pub fn lookup(&mut self, trigger: &str, hint: &PersonalityHint) -> Option<String> {
        let group = self.templates.get_mut(trigger)?;
        if group.variants.is_empty() {
            return None;
        }

        group.last_accessed = Utc::now();

        // Score each variant by personality match
        let scored: Vec<(usize, u32)> = group
            .variants
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let mut score = 0u32;
                if !hint.style.is_empty()
                    && v.personality.style.eq_ignore_ascii_case(&hint.style)
                {
                    score += 2;
                }
                if !hint.class_role.is_empty()
                    && v.personality.class_role.eq_ignore_ascii_case(&hint.class_role)
                {
                    score += 1;
                }
                for t in &hint.traits {
                    if v.personality
                        .traits
                        .iter()
                        .any(|vt| vt.eq_ignore_ascii_case(t))
                    {
                        score += 1;
                    }
                }
                (i, score)
            })
            .collect();

        // Find the max score
        let max_score = scored.iter().map(|(_, s)| *s).max().unwrap_or(0);

        // Collect candidates at the max score level
        let candidates: Vec<usize> = scored
            .iter()
            .filter(|(_, s)| *s == max_score)
            .map(|(i, _)| *i)
            .collect();

        // Pick a random candidate
        let mut rng = rand::thread_rng();
        let &idx = candidates.choose(&mut rng)?;

        let variant = &mut group.variants[idx];
        variant.last_used = Utc::now();
        variant.use_count += 1;

        self.dirty = true;

        Some(variant.text.clone())
    }

    /// Insert variants with cosine dedup against existing variants for the trigger.
    ///
    /// If the embedder is None, dedup is skipped and all variants are inserted.
    /// Enforces per-trigger limit and total limit with LRU eviction.
    pub async fn insert_with_dedup(
        &mut self,
        trigger: &str,
        variants: Vec<StoredVariant>,
        embedder: &Option<Arc<EmbeddingEngine>>,
        threshold: f32,
    ) {
        let group = self
            .templates
            .entry(trigger.to_string())
            .or_insert_with(|| TriggerGroup {
                variants: Vec::new(),
                last_accessed: Utc::now(),
            });

        for variant in variants {
            // Cosine dedup: skip if too similar to an existing variant
            if let Some(ref emb) = embedder {
                if let Ok(new_vec) = emb.embed(&variant.text) {
                    let is_dup = group.variants.iter().any(|existing| {
                        if let Ok(existing_vec) = emb.embed(&existing.text) {
                            cosine_similarity(&new_vec, &existing_vec) >= threshold
                        } else {
                            false
                        }
                    });

                    if is_dup {
                        debug!(
                            "Skipping duplicate variant for trigger '{}': '{}'",
                            trigger,
                            &variant.text[..variant.text.len().min(50)]
                        );
                        continue;
                    }
                }
            }

            group.variants.push(variant);
        }

        group.last_accessed = Utc::now();

        // Enforce per-trigger limit: keep most recently used
        if group.variants.len() > self.config.max_variants_per_trigger {
            group
                .variants
                .sort_by(|a, b| b.last_used.cmp(&a.last_used));
            group.variants.truncate(self.config.max_variants_per_trigger);
        }

        // Enforce total limit with LRU eviction across all triggers
        self.evict_lru();

        self.dirty = true;
    }

    /// Evict least-recently-used variants until under the total limit.
    fn evict_lru(&mut self) {
        while self.total_variant_count() > self.config.max_total_templates {
            // Find the trigger with the oldest last_accessed
            let oldest_trigger = self
                .templates
                .iter()
                .min_by_key(|(_, g)| g.last_accessed)
                .map(|(k, _)| k.clone());

            if let Some(trigger) = oldest_trigger {
                if let Some(group) = self.templates.get_mut(&trigger) {
                    // Remove the oldest variant in this group
                    if !group.variants.is_empty() {
                        let oldest_idx = group
                            .variants
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, v)| v.last_used)
                            .map(|(i, _)| i)
                            .unwrap();
                        group.variants.remove(oldest_idx);
                    }

                    // Remove the trigger entirely if empty
                    if group.variants.is_empty() {
                        self.templates.remove(&trigger);
                    }
                }
            } else {
                break;
            }
        }
    }

    /// Count total variants across all triggers.
    pub fn total_variant_count(&self) -> usize {
        self.templates.values().map(|g| g.variants.len()).sum()
    }

    /// Whether the store has been modified since last persist.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Mark the store as dirty (modified).
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Clear the dirty flag (after persisting).
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    /// Returns (total_variant_count, trigger_count).
    pub fn stats(&self) -> (usize, usize) {
        (self.total_variant_count(), self.templates.len())
    }

    /// Bulk import variants with dedup.
    pub async fn import(
        &mut self,
        data: HashMap<String, Vec<StoredVariant>>,
        embedder: &Option<Arc<EmbeddingEngine>>,
    ) {
        let threshold = self.config.dedup_threshold;
        for (trigger, variants) in data {
            self.insert_with_dedup(&trigger, variants, embedder, threshold)
                .await;
        }
    }
}

/// Compute cosine similarity between two vectors.
/// Returns 0.0 for empty or mismatched-length vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for (ai, bi) in a.iter().zip(b.iter()) {
        dot += ai * bi;
        norm_a += ai * ai;
        norm_b += bi * bi;
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom < 1e-10 {
        0.0
    } else {
        dot / denom
    }
}

// ---------------------------------------------------------------------------
// TemplateGenerator -- background LLM generation with mpsc queue
// ---------------------------------------------------------------------------

/// A request to generate template variants for a trigger.
#[derive(Debug, Clone)]
pub struct GenerationRequest {
    pub trigger: String,
    pub original: String,
    pub channel: String,
    pub personality: PersonalityHint,
    pub count: usize,
}

/// Background LLM template generator.
///
/// Accepts generation requests via an async mpsc channel, builds
/// personality-aware prompts, calls the LLM, parses variants, and
/// stores them in the `TemplateStore` with cosine dedup.
pub struct TemplateGenerator {
    tx: tokio::sync::mpsc::Sender<GenerationRequest>,
    queue_depth: Arc<AtomicUsize>,
    max_depth: usize,
}

impl TemplateGenerator {
    /// Start the generator with a background processing loop.
    ///
    /// The generator spawns two background tasks:
    /// 1. A processing loop that receives requests and generates templates
    /// 2. A debounced persistence task that saves dirty stores every 5 minutes
    pub fn start(
        llm_router: Arc<LlmRouter>,
        embedder: Option<Arc<EmbeddingEngine>>,
        store: Arc<RwLock<TemplateStore>>,
        config: TemplateConfig,
        persist_path: PathBuf,
        shutdown: Arc<tokio::sync::Notify>,
    ) -> Arc<Self> {
        let (tx, rx) = tokio::sync::mpsc::channel(config.generation_queue_depth);
        let queue_depth = Arc::new(AtomicUsize::new(0));

        // Spawn the generation processing loop
        let depth = queue_depth.clone();
        let store_for_gen = store.clone();
        let embedder_for_gen = embedder.clone();
        let dedup_threshold = config.dedup_threshold;
        let shutdown_for_gen = shutdown.clone();
        tokio::spawn(async move {
            Self::run_loop(
                rx,
                llm_router,
                embedder_for_gen,
                store_for_gen,
                dedup_threshold,
                depth,
                shutdown_for_gen,
            )
            .await;
        });

        // Spawn the debounced persistence task (every 5 minutes)
        let store_for_persist = store.clone();
        let shutdown_for_persist = shutdown.clone();
        tokio::spawn(async move {
            Self::persist_loop(store_for_persist, persist_path, shutdown_for_persist).await;
        });

        Arc::new(Self {
            tx,
            queue_depth,
            max_depth: config.generation_queue_depth,
        })
    }

    /// Enqueue a generation request.
    ///
    /// Returns the queue position on success, or an error string if
    /// the queue is full or the channel is closed.
    pub async fn enqueue(&self, req: GenerationRequest) -> Result<usize, &'static str> {
        let depth = self.queue_depth.load(Ordering::Relaxed);
        if depth >= self.max_depth {
            return Err("queue_full");
        }
        self.tx.send(req).await.map_err(|_| "channel_closed")?;
        let new_depth = self.queue_depth.fetch_add(1, Ordering::Relaxed) + 1;
        Ok(new_depth)
    }

    /// Current number of pending requests.
    pub fn pending(&self) -> usize {
        self.queue_depth.load(Ordering::Relaxed)
    }

    /// Background processing loop.
    async fn run_loop(
        mut rx: tokio::sync::mpsc::Receiver<GenerationRequest>,
        llm_router: Arc<LlmRouter>,
        embedder: Option<Arc<EmbeddingEngine>>,
        store: Arc<RwLock<TemplateStore>>,
        dedup_threshold: f32,
        queue_depth: Arc<AtomicUsize>,
        shutdown: Arc<tokio::sync::Notify>,
    ) {
        loop {
            tokio::select! {
                Some(req) = rx.recv() => {
                    queue_depth.fetch_sub(1, Ordering::Relaxed);

                    let messages = Self::build_prompt(&req);
                    match llm_router.generate_chat(messages, 300, 0.9).await {
                        LlmResult::Success { text, .. } => {
                            let variants = Self::parse_variants(&text, &req);
                            if !variants.is_empty() {
                                let count = variants.len();
                                let mut guard = store.write().await;
                                guard
                                    .insert_with_dedup(
                                        &req.trigger,
                                        variants,
                                        &embedder,
                                        dedup_threshold,
                                    )
                                    .await;
                                info!(
                                    "Generated {} variants for trigger '{}' (channel: {})",
                                    count, req.trigger, req.channel
                                );
                            }
                        }
                        LlmResult::Fallback { reason } => {
                            debug!(
                                "Template generation skipped for '{}': {}",
                                req.trigger, reason
                            );
                        }
                    }
                }
                _ = shutdown.notified() => {
                    info!("Template generator shutting down, draining queue...");
                    // Drain remaining items
                    rx.close();
                    while let Some(req) = rx.recv().await {
                        queue_depth.fetch_sub(1, Ordering::Relaxed);
                        debug!("Drained queued request for trigger '{}'", req.trigger);
                    }
                    info!("Template generator queue drained");
                    break;
                }
            }
        }
    }

    /// Build a personality-aware LLM prompt for template generation.
    fn build_prompt(req: &GenerationRequest) -> Vec<ChatMessage> {
        let name = req
            .personality
            .source_sim
            .as_deref()
            .unwrap_or("a SimPlayer");
        let class_role = if req.personality.class_role.is_empty() {
            "adventurer"
        } else {
            &req.personality.class_role
        };
        let style = if req.personality.style.is_empty() {
            "neutral"
        } else {
            &req.personality.style
        };
        let traits_str = if req.personality.traits.is_empty() {
            "no particular quirks".to_string()
        } else {
            req.personality.traits.join(", ")
        };

        let mut system = format!(
            "You are generating speech variants for {name}, a {class_role} who speaks in a {style} manner. \
             They are {traits}.\n\n\
             Original combat callout: \"{original}\"\n\n\
             Generate {count} alternative ways this character would express the same intent.\n\
             Each variant should:\n\
             - Match the character's speech patterns and personality\n\
             - Be concise (combat callouts are brief)\n\
             - Preserve the functional meaning (pulling, aggro warning, OOM, etc.)\n\
             - Use {{{{speaker}}}} as a placeholder where the speaker refers to themselves",
            name = name,
            class_role = class_role,
            style = style,
            traits = traits_str,
            original = req.original,
            count = req.count,
        );

        // Channel-aware zone filter
        let ch = req.channel.to_lowercase();
        if ch == "party" || ch == "guild" {
            system.push_str(
                "\n- Avoid self-referential location statements like \"here in [place]\" -- \
                 the group already knows where they are",
            );
        }

        system.push_str(
            "\n\nOutput each variant on a separate line, numbered 1-N. \
             Do not include explanations or commentary.",
        );

        vec![
            ChatMessage {
                role: "system".to_string(),
                content: system,
            },
            ChatMessage {
                role: "user".to_string(),
                content: format!("Generate {} variants:", req.count),
            },
        ]
    }

    /// Parse LLM output into individual `StoredVariant`s.
    ///
    /// Strips numbering prefixes (1. / 1) / 1:), filters by length,
    /// and preserves {speaker} placeholders.
    fn parse_variants(response: &str, req: &GenerationRequest) -> Vec<StoredVariant> {
        let now = Utc::now();
        response
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    return None;
                }

                // Strip numbering: "1. Text", "1) Text", "1: Text"
                let text = trimmed
                    .trim_start_matches(|c: char| c.is_ascii_digit())
                    .trim_start_matches(|c: char| c == '.' || c == ')' || c == ':')
                    .trim()
                    .to_string();

                if text.len() < 3 || text.len() > 200 {
                    return None;
                }

                Some(StoredVariant {
                    text,
                    personality: req.personality.clone(),
                    source_sim: req
                        .personality
                        .source_sim
                        .clone()
                        .unwrap_or_default(),
                    channel: req.channel.clone(),
                    generated_at: now,
                    last_used: now,
                    use_count: 0,
                })
            })
            .collect()
    }

    /// Debounced persistence loop: persists every 5 minutes if dirty.
    async fn persist_loop(
        store: Arc<RwLock<TemplateStore>>,
        persist_path: PathBuf,
        shutdown: Arc<tokio::sync::Notify>,
    ) {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let store_guard = store.read().await;
                    if store_guard.is_dirty() {
                        match store_guard.persist(&persist_path) {
                            Ok(()) => {
                                drop(store_guard);
                                let mut write_guard = store.write().await;
                                write_guard.clear_dirty();
                                debug!("Template store auto-persisted");
                            }
                            Err(e) => {
                                warn!("Template auto-persist failed: {}", e);
                            }
                        }
                    }
                }
                _ = shutdown.notified() => {
                    // Final persist on shutdown
                    let store_guard = store.read().await;
                    if store_guard.is_dirty() {
                        match store_guard.persist(&persist_path) {
                            Ok(()) => info!("Template store persisted on shutdown"),
                            Err(e) => warn!("Template shutdown persist failed: {}", e),
                        }
                    }
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let v = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_empty() {
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn test_cosine_similarity_mismatched() {
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn test_template_store_new() {
        let store = TemplateStore::new(TemplateStoreConfig::default());
        assert_eq!(store.templates.len(), 0);
        assert!(!store.is_dirty());
    }

    #[test]
    fn test_template_store_lookup_empty() {
        let mut store = TemplateStore::new(TemplateStoreConfig::default());
        let hint = PersonalityHint::default();
        assert!(store.lookup("nonexistent", &hint).is_none());
    }

    #[test]
    fn test_template_store_stats() {
        let mut store = TemplateStore::new(TemplateStoreConfig::default());
        assert_eq!(store.stats(), (0, 0));

        store.templates.insert(
            "pulling".to_string(),
            TriggerGroup {
                variants: vec![StoredVariant {
                    text: "Incoming!".to_string(),
                    personality: PersonalityHint::default(),
                    source_sim: "TestSim".to_string(),
                    channel: "say".to_string(),
                    generated_at: Utc::now(),
                    last_used: Utc::now(),
                    use_count: 0,
                }],
                last_accessed: Utc::now(),
            },
        );

        assert_eq!(store.stats(), (1, 1));
    }

    #[test]
    fn test_template_store_lookup_with_personality() {
        let mut store = TemplateStore::new(TemplateStoreConfig::default());
        let now = Utc::now();

        store.templates.insert(
            "pulling".to_string(),
            TriggerGroup {
                variants: vec![
                    StoredVariant {
                        text: "Let's go!".to_string(),
                        personality: PersonalityHint {
                            style: "enthusiastic".to_string(),
                            class_role: "tank".to_string(),
                            traits: vec!["brave".to_string()],
                            source_sim: None,
                        },
                        source_sim: "Boldric".to_string(),
                        channel: "say".to_string(),
                        generated_at: now,
                        last_used: now,
                        use_count: 0,
                    },
                    StoredVariant {
                        text: "Whatever...".to_string(),
                        personality: PersonalityHint {
                            style: "grumpy".to_string(),
                            class_role: "dps".to_string(),
                            traits: vec!["mean".to_string()],
                            source_sim: None,
                        },
                        source_sim: "Grimbold".to_string(),
                        channel: "say".to_string(),
                        generated_at: now,
                        last_used: now,
                        use_count: 0,
                    },
                ],
                last_accessed: now,
            },
        );

        // Request enthusiastic tank -- should get "Let's go!"
        let hint = PersonalityHint {
            style: "enthusiastic".to_string(),
            class_role: "tank".to_string(),
            traits: vec![],
            source_sim: None,
        };

        let result = store.lookup("pulling", &hint);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "Let's go!");
    }

    #[test]
    fn test_parse_variants_numbered() {
        let req = GenerationRequest {
            trigger: "pulling".to_string(),
            original: "Incoming!".to_string(),
            channel: "say".to_string(),
            personality: PersonalityHint {
                style: "enthusiastic".to_string(),
                class_role: "tank".to_string(),
                traits: vec!["brave".to_string()],
                source_sim: Some("Boldric".to_string()),
            },
            count: 3,
        };

        let response = "1. Here they come!\n2. Pulling now, stay sharp!\n3. {speaker} is pulling!";
        let variants = TemplateGenerator::parse_variants(response, &req);
        assert_eq!(variants.len(), 3);
        assert_eq!(variants[0].text, "Here they come!");
        assert_eq!(variants[1].text, "Pulling now, stay sharp!");
        assert_eq!(variants[2].text, "{speaker} is pulling!");
        assert_eq!(variants[0].source_sim, "Boldric");
        assert_eq!(variants[0].channel, "say");
    }

    #[test]
    fn test_parse_variants_paren_numbering() {
        let req = GenerationRequest {
            trigger: "oom".to_string(),
            original: "OOM!".to_string(),
            channel: "party".to_string(),
            personality: PersonalityHint::default(),
            count: 2,
        };

        let response = "1) I'm out of mana!\n2) No mana left, hold on!";
        let variants = TemplateGenerator::parse_variants(response, &req);
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0].text, "I'm out of mana!");
        assert_eq!(variants[1].text, "No mana left, hold on!");
    }

    #[test]
    fn test_parse_variants_filters_short_long() {
        let req = GenerationRequest {
            trigger: "test".to_string(),
            original: "test".to_string(),
            channel: "say".to_string(),
            personality: PersonalityHint::default(),
            count: 3,
        };

        let too_short = "1. ab\n"; // 2 chars after strip
        let ok = "2. This is fine\n";
        let too_long = format!("3. {}\n", "x".repeat(201));
        let response = format!("{}{}{}", too_short, ok, too_long);
        let variants = TemplateGenerator::parse_variants(&response, &req);
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0].text, "This is fine");
    }

    #[test]
    fn test_parse_variants_empty_lines_skipped() {
        let req = GenerationRequest {
            trigger: "test".to_string(),
            original: "test".to_string(),
            channel: "say".to_string(),
            personality: PersonalityHint::default(),
            count: 2,
        };

        let response = "1. First line\n\n\n2. Second line\n\n";
        let variants = TemplateGenerator::parse_variants(response, &req);
        assert_eq!(variants.len(), 2);
    }

    #[test]
    fn test_build_prompt_party_channel() {
        let req = GenerationRequest {
            trigger: "pulling".to_string(),
            original: "Incoming!".to_string(),
            channel: "party".to_string(),
            personality: PersonalityHint {
                style: "enthusiastic".to_string(),
                class_role: "tank".to_string(),
                traits: vec!["brave".to_string()],
                source_sim: Some("Boldric".to_string()),
            },
            count: 4,
        };

        let messages = TemplateGenerator::build_prompt(&req);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[1].role, "user");
        // Should contain zone filter for party channel
        assert!(messages[0].content.contains("Avoid self-referential location"));
        assert!(messages[0].content.contains("Boldric"));
        assert!(messages[0].content.contains("tank"));
        assert!(messages[0].content.contains("enthusiastic"));
        assert!(messages[0].content.contains("Incoming!"));
    }

    #[test]
    fn test_build_prompt_say_channel_no_zone_filter() {
        let req = GenerationRequest {
            trigger: "greeting".to_string(),
            original: "Hello there!".to_string(),
            channel: "say".to_string(),
            personality: PersonalityHint {
                style: "friendly".to_string(),
                class_role: "healer".to_string(),
                traits: vec![],
                source_sim: None,
            },
            count: 3,
        };

        let messages = TemplateGenerator::build_prompt(&req);
        // Should NOT contain zone filter for say channel
        assert!(!messages[0].content.contains("Avoid self-referential location"));
    }

    #[test]
    fn test_build_prompt_guild_channel_has_zone_filter() {
        let req = GenerationRequest {
            trigger: "lfg".to_string(),
            original: "LFG!".to_string(),
            channel: "guild".to_string(),
            personality: PersonalityHint::default(),
            count: 4,
        };

        let messages = TemplateGenerator::build_prompt(&req);
        assert!(messages[0].content.contains("Avoid self-referential location"));
    }

    #[test]
    fn test_persist_and_load() {
        let dir = std::env::temp_dir().join("erenshor_template_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test_templates.json");

        let mut store = TemplateStore::new(TemplateStoreConfig::default());
        store.templates.insert(
            "hello".to_string(),
            TriggerGroup {
                variants: vec![StoredVariant {
                    text: "Hi there!".to_string(),
                    personality: PersonalityHint::default(),
                    source_sim: "Test".to_string(),
                    channel: "say".to_string(),
                    generated_at: Utc::now(),
                    last_used: Utc::now(),
                    use_count: 1,
                }],
                last_accessed: Utc::now(),
            },
        );

        store.persist(&path).unwrap();

        let loaded = TemplateStore::load(&path, TemplateStoreConfig::default());
        assert_eq!(loaded.templates.len(), 1);
        assert!(loaded.templates.contains_key("hello"));

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }
}
