//! Configuration loading from TOML files with sensible defaults.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Top-level configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub embedding: EmbeddingConfig,
    #[serde(default)]
    pub indexes: IndexConfig,
    #[serde(default)]
    pub vectordb: VectorDbConfig,
    #[serde(default)]
    pub sona: SonaConfig,
    #[serde(default)]
    pub respond: RespondConfig,
    #[serde(default)]
    pub llm: LlmConfig,

    /// The resolved data directory (not serialized from TOML)
    #[serde(skip)]
    pub data_dir: PathBuf,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            embedding: EmbeddingConfig::default(),
            indexes: IndexConfig::default(),
            vectordb: VectorDbConfig::default(),
            sona: SonaConfig::default(),
            respond: RespondConfig::default(),
            llm: LlmConfig::default(),
            data_dir: PathBuf::from("."),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_host")]
    pub host: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            host: default_host(),
        }
    }
}

fn default_port() -> u16 {
    11435
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    #[serde(default = "default_model_path")]
    pub model_path: String,
    #[serde(default = "default_tokenizer_path")]
    pub tokenizer_path: String,
    #[serde(default = "default_threads")]
    pub threads: usize,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            model_path: default_model_path(),
            tokenizer_path: default_tokenizer_path(),
            threads: default_threads(),
        }
    }
}

fn default_model_path() -> String {
    "models/all-minilm-l6-v2.onnx".to_string()
}

fn default_tokenizer_path() -> String {
    "models/tokenizer.json".to_string()
}

fn default_threads() -> usize {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexConfig {
    #[serde(default = "default_lore_path")]
    pub lore_path: String,
    #[serde(default = "default_responses_path")]
    pub responses_path: String,
    #[serde(default = "default_memory_path")]
    pub memory_path: String,
    #[serde(default = "default_personality_path")]
    pub personality_path: String,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            lore_path: default_lore_path(),
            responses_path: default_responses_path(),
            memory_path: default_memory_path(),
            personality_path: default_personality_path(),
        }
    }
}

fn default_lore_path() -> String {
    "dist/lore.json".to_string()
}

fn default_responses_path() -> String {
    "dist/responses.json".to_string()
}

fn default_memory_path() -> String {
    "dist/memory.json".to_string()
}

fn default_personality_path() -> String {
    "dist/personality.ruvector".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorDbConfig {
    /// HNSW M parameter (connections per layer).
    #[serde(default = "default_hnsw_m")]
    pub hnsw_m: usize,
    /// HNSW efConstruction parameter.
    #[serde(default = "default_hnsw_ef_construction")]
    pub hnsw_ef_construction: usize,
    /// HNSW efSearch parameter.
    #[serde(default = "default_hnsw_ef_search")]
    pub hnsw_ef_search: usize,
    /// Maximum number of elements per index.
    #[serde(default = "default_max_elements")]
    pub max_elements: usize,
}

impl Default for VectorDbConfig {
    fn default() -> Self {
        Self {
            hnsw_m: default_hnsw_m(),
            hnsw_ef_construction: default_hnsw_ef_construction(),
            hnsw_ef_search: default_hnsw_ef_search(),
            max_elements: default_max_elements(),
        }
    }
}

fn default_hnsw_m() -> usize {
    16
}

fn default_hnsw_ef_construction() -> usize {
    200
}

fn default_hnsw_ef_search() -> usize {
    50
}

fn default_max_elements() -> usize {
    10_000
}

impl VectorDbConfig {
    /// Convert to AdapterConfig for VectorStoreAdapter.
    pub fn to_adapter_config(&self) -> crate::intelligence::vector_store::AdapterConfig {
        crate::intelligence::vector_store::AdapterConfig {
            dimensions: 384,
            hnsw_m: self.hnsw_m,
            hnsw_ef_construction: self.hnsw_ef_construction,
            hnsw_ef_search: self.hnsw_ef_search,
            max_elements: self.max_elements,
            quantization: false,
        }
    }
}

impl IndexConfig {
    /// Get the .ruvector path for a given index path.
    /// Handles both `.json` and `.ruvector` input:
    ///   "dist/lore.json"     -> "dist/lore.ruvector"
    ///   "dist/lore.ruvector" -> "dist/lore.ruvector"
    pub fn ruvector_path(path: &str) -> String {
        if path.ends_with(".ruvector") {
            path.to_string()
        } else if let Some(stem) = path.strip_suffix(".json") {
            format!("{}.ruvector", stem)
        } else {
            format!("{}.ruvector", path)
        }
    }

    /// Get the .json fallback path for a given index path.
    /// Handles both `.ruvector` and `.json` input:
    ///   "dist/lore.ruvector" -> "dist/lore.json"
    ///   "dist/lore.json"     -> "dist/lore.json"
    pub fn json_path(path: &str) -> String {
        if path.ends_with(".json") {
            path.to_string()
        } else if let Some(stem) = path.strip_suffix(".ruvector") {
            format!("{}.json", stem)
        } else {
            format!("{}.json", path)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SonaConfig {
    /// Enable SONA adaptive learning.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Hidden/embedding dimension (must match embedding model).
    #[serde(default = "default_hidden_dim")]
    pub hidden_dim: usize,
    /// MicroLoRA rank (1-2). Rank-2 is ~5% faster due to SIMD.
    #[serde(default = "default_micro_lora_rank")]
    pub micro_lora_rank: usize,
    /// BaseLoRA rank (4-16).
    #[serde(default = "default_base_lora_rank")]
    pub base_lora_rank: usize,
    /// Maximum number of trajectories to buffer.
    #[serde(default = "default_trajectory_capacity")]
    pub trajectory_capacity: usize,
    /// Background learning interval in milliseconds.
    #[serde(default = "default_background_interval_ms")]
    pub background_interval_ms: u64,
    /// Number of pattern clusters for trajectory analysis.
    #[serde(default = "default_pattern_clusters")]
    pub pattern_clusters: usize,
    /// Minimum quality threshold for learning from a trajectory.
    #[serde(default = "default_quality_threshold")]
    pub quality_threshold: f32,
}

impl Default for SonaConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            hidden_dim: default_hidden_dim(),
            micro_lora_rank: default_micro_lora_rank(),
            base_lora_rank: default_base_lora_rank(),
            trajectory_capacity: default_trajectory_capacity(),
            background_interval_ms: default_background_interval_ms(),
            pattern_clusters: default_pattern_clusters(),
            quality_threshold: default_quality_threshold(),
        }
    }
}

impl SonaConfig {
    /// Convert to SonaManagerConfig for the integration layer.
    pub fn to_manager_config(&self) -> crate::intelligence::sona_integration::SonaManagerConfig {
        crate::intelligence::sona_integration::SonaManagerConfig {
            enabled: self.enabled,
            hidden_dim: self.hidden_dim,
            micro_lora_rank: self.micro_lora_rank,
            base_lora_rank: self.base_lora_rank,
            trajectory_capacity: self.trajectory_capacity,
            background_interval_ms: self.background_interval_ms,
            pattern_clusters: self.pattern_clusters,
            quality_threshold: self.quality_threshold,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_hidden_dim() -> usize {
    384
}

fn default_micro_lora_rank() -> usize {
    1
}

fn default_base_lora_rank() -> usize {
    4
}

fn default_trajectory_capacity() -> usize {
    5000
}

fn default_background_interval_ms() -> u64 {
    300_000
}

fn default_pattern_clusters() -> usize {
    50
}

fn default_quality_threshold() -> f32 {
    0.3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespondConfig {
    #[serde(default = "default_template_candidates")]
    pub template_candidates: usize,
    #[serde(default = "default_lore_candidates")]
    pub lore_candidates: usize,
    #[serde(default = "default_memory_candidates")]
    pub memory_candidates: usize,
    #[serde(default = "default_min_confidence")]
    pub min_confidence: f32,
    #[serde(default = "default_semantic_weight")]
    pub semantic_weight: f32,
    #[serde(default = "default_zone_weight")]
    pub zone_weight: f32,
    #[serde(default = "default_personality_weight")]
    pub personality_weight: f32,
    #[serde(default = "default_relationship_weight")]
    pub relationship_weight: f32,
    #[serde(default = "default_channel_weight")]
    pub channel_weight: f32,
    #[serde(default = "default_sim_name_weight")]
    pub sim_name_weight: f32,
}

impl Default for RespondConfig {
    fn default() -> Self {
        Self {
            template_candidates: default_template_candidates(),
            lore_candidates: default_lore_candidates(),
            memory_candidates: default_memory_candidates(),
            min_confidence: default_min_confidence(),
            semantic_weight: default_semantic_weight(),
            zone_weight: default_zone_weight(),
            personality_weight: default_personality_weight(),
            relationship_weight: default_relationship_weight(),
            channel_weight: default_channel_weight(),
            sim_name_weight: default_sim_name_weight(),
        }
    }
}

fn default_template_candidates() -> usize {
    10
}

fn default_lore_candidates() -> usize {
    3
}

fn default_memory_candidates() -> usize {
    3
}

fn default_min_confidence() -> f32 {
    0.3
}

fn default_semantic_weight() -> f32 {
    0.15
}

fn default_zone_weight() -> f32 {
    0.15
}

fn default_personality_weight() -> f32 {
    0.25
}

fn default_relationship_weight() -> f32 {
    0.15
}

fn default_channel_weight() -> f32 {
    0.15
}

fn default_sim_name_weight() -> f32 {
    0.15
}

// --- Phase 3: LLM Configuration ---

/// LLM mode: Off, Local (llama.cpp GGUF), Cloud (OpenRouter), or Hybrid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LlmMode {
    Off,
    Local,
    Cloud,
    Hybrid,
}

impl Default for LlmMode {
    fn default() -> Self {
        Self::Off
    }
}

/// Top-level LLM configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Enable LLM text generation.
    #[serde(default)]
    pub enabled: bool,
    /// LLM mode (off/local/cloud/hybrid).
    #[serde(default)]
    pub mode: LlmMode,
    /// Template confidence below which LLM enhancement is triggered.
    #[serde(default = "default_enhance_threshold")]
    pub enhance_threshold: f32,
    /// Maximum tokens to generate.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    /// Sampling temperature.
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// Maximum concurrent LLM inference requests.
    #[serde(default = "default_queue_depth")]
    pub queue_depth: usize,
    /// Base probability (0.0-1.0) of paraphrasing a template through LLM for variety.
    #[serde(default = "default_paraphrase_chance")]
    pub paraphrase_chance: f32,
    /// Paraphrase probability when the selected template was recently used.
    #[serde(default = "default_paraphrase_recency_chance")]
    pub paraphrase_recency_chance: f32,
    /// Paraphrase probability for sim-to-sim dialog (ambient chatter).
    /// Higher than player-to-sim because variety matters more for overheard dialog.
    #[serde(default = "default_paraphrase_sim_to_sim_chance")]
    pub paraphrase_sim_to_sim_chance: f32,
    /// Local LLM backend config.
    #[serde(default)]
    pub local: LocalLlmConfig,
    /// Cloud LLM backend config.
    #[serde(default)]
    pub cloud: CloudLlmConfig,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: LlmMode::Off,
            enhance_threshold: default_enhance_threshold(),
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            queue_depth: default_queue_depth(),
            paraphrase_chance: default_paraphrase_chance(),
            paraphrase_recency_chance: default_paraphrase_recency_chance(),
            paraphrase_sim_to_sim_chance: default_paraphrase_sim_to_sim_chance(),
            local: LocalLlmConfig::default(),
            cloud: CloudLlmConfig::default(),
        }
    }
}

fn default_enhance_threshold() -> f32 {
    0.85
}

fn default_max_tokens() -> usize {
    150
}

fn default_temperature() -> f32 {
    0.7
}

fn default_queue_depth() -> usize {
    5
}

fn default_paraphrase_chance() -> f32 {
    0.15
}

fn default_paraphrase_recency_chance() -> f32 {
    0.80
}

fn default_paraphrase_sim_to_sim_chance() -> f32 {
    0.90
}

/// Local LLM backend configuration (shimmy external inference server).
/// Shimmy runs as a separate process, loads GGUF models, and handles GPU
/// acceleration (Vulkan/CUDA) natively. We communicate over HTTP on localhost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalLlmConfig {
    /// HTTP endpoint for the local inference server (shimmy).
    #[serde(default = "default_local_endpoint")]
    pub endpoint: String,
    /// Model name to request from the local inference server.
    #[serde(default = "default_local_model")]
    pub model: String,
    /// Request timeout in milliseconds.
    #[serde(default = "default_local_timeout_ms")]
    pub timeout_ms: u64,
    /// Whether the sidecar should auto-start shimmy.
    #[serde(default = "default_true")]
    pub auto_start: bool,
    /// GPU backend for shimmy: "cuda", "vulkan", "auto", "cpu".
    #[serde(default = "default_gpu_backend")]
    pub gpu_backend: String,
    /// Directory containing GGUF model files (relative to data-dir).
    #[serde(default = "default_model_dir")]
    pub model_dir: String,
}

impl Default for LocalLlmConfig {
    fn default() -> Self {
        Self {
            endpoint: default_local_endpoint(),
            model: default_local_model(),
            timeout_ms: default_local_timeout_ms(),
            auto_start: true,
            gpu_backend: default_gpu_backend(),
            model_dir: default_model_dir(),
        }
    }
}

fn default_local_endpoint() -> String {
    "http://127.0.0.1:8012".to_string()
}

fn default_local_model() -> String {
    "gemma3npc-1b-q4_k_m".to_string()
}

fn default_local_timeout_ms() -> u64 {
    30_000
}

fn default_gpu_backend() -> String {
    "cuda".to_string()
}

fn default_model_dir() -> String {
    "models".to_string()
}

/// Cloud LLM backend configuration (OpenRouter).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudLlmConfig {
    /// API provider name.
    #[serde(default = "default_provider")]
    pub provider: String,
    /// API key (empty = disabled).
    #[serde(default)]
    pub api_key: String,
    /// API endpoint URL.
    #[serde(default = "default_api_endpoint")]
    pub api_endpoint: String,
    /// Model identifier for the cloud provider.
    #[serde(default = "default_cloud_model")]
    pub model: String,
    /// Request timeout in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for CloudLlmConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            api_key: String::new(),
            api_endpoint: default_api_endpoint(),
            model: default_cloud_model(),
            timeout_ms: default_timeout_ms(),
        }
    }
}

fn default_provider() -> String {
    "openrouter".to_string()
}

fn default_api_endpoint() -> String {
    "https://openrouter.ai/api/v1".to_string()
}

fn default_cloud_model() -> String {
    "anthropic/claude-3.5-haiku".to_string()
}

fn default_timeout_ms() -> u64 {
    10_000
}

/// Load configuration from a TOML file, with CLI overrides.
///
/// Priority:
/// 1. CLI flags override config file values
/// 2. Config file (if provided via `--config` or found at `{data_dir}/erenshor-llm.toml`)
/// 3. Defaults
pub fn load_config(
    config_path: Option<&Path>,
    data_dir: &Path,
    cli_port: u16,
    cli_threads: Option<usize>,
) -> anyhow::Result<AppConfig> {
    let config_file = config_path
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir.join("erenshor-llm.toml"));

    let mut config = if config_file.exists() {
        let contents = std::fs::read_to_string(&config_file)?;
        tracing::info!("Loading config from {:?}", config_file);
        toml::from_str::<AppConfig>(&contents)?
    } else {
        tracing::info!("No config file found, using defaults");
        AppConfig::default()
    };

    // CLI port overrides config file
    if cli_port != default_port() {
        config.server.port = cli_port;
    }

    // CLI threads overrides config file
    if let Some(threads) = cli_threads {
        config.embedding.threads = threads;
    }

    // Store the resolved data directory as an absolute path.
    // This is critical: if data_dir is relative (e.g. "data"), we must
    // canonicalize it NOW while the CWD is known. Otherwise all
    // resolve_path() calls depend on CWD remaining unchanged.
    config.data_dir = if data_dir.is_absolute() {
        data_dir.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(cwd) => {
                let abs = cwd.join(data_dir);
                tracing::info!("Data directory (absolute): {:?}", abs);
                abs
            }
            Err(e) => {
                tracing::warn!("Could not get CWD to resolve data_dir: {}. Using as-is: {:?}", e, data_dir);
                data_dir.to_path_buf()
            }
        }
    };

    Ok(config)
}

impl AppConfig {
    /// Resolve a relative path against the data directory.
    ///
    /// Normalizes path separators to the platform native separator so that
    /// forward-slash defaults (e.g. "dist/lore.ruvector") work correctly
    /// when data_dir uses backslashes on Windows. Without this, the cross-
    /// compiled MinGW binary produces mixed-separator paths like
    /// `data\dist/lore.ruvector` which fail to find existing files.
    pub fn resolve_path(&self, relative: &str) -> PathBuf {
        // Build the relative portion from individual components so the
        // platform-native separator is used throughout.
        let mut p = PathBuf::new();
        for component in relative.split('/') {
            if !component.is_empty() {
                p.push(component);
            }
        }
        if p.is_absolute() {
            p
        } else {
            self.data_dir.join(p)
        }
    }
}
