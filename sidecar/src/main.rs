//! erenshor-llm: RuVector-powered intelligence sidecar for Erenshor LLM Dialog mod
//!
//! This binary runs as a child process of the Unity game, providing semantic
//! response generation via HTTP on localhost.

mod builder;
mod config;
mod error;
mod intelligence;
mod llm;
mod routes;
mod server;
mod state;

use clap::{Parser, Subcommand};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

#[derive(Parser)]
#[command(name = "erenshor-llm", version = "0.3.0", about = "RuVector intelligence sidecar for Erenshor")]
struct Cli {
    /// Port to listen on
    #[arg(long, default_value = "11435")]
    port: u16,

    /// Path to TOML config file
    #[arg(long)]
    config: Option<PathBuf>,

    /// Data directory (indexes, models, templates)
    #[arg(long, default_value = ".")]
    data_dir: PathBuf,

    /// Number of CPU threads for the ONNX embedding model
    #[arg(long)]
    threads: Option<usize>,

    /// Log format: "pretty", "plain" (no ANSI/timestamps), or "json"
    #[arg(long, default_value = "pretty")]
    log_format: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Build a lore vector database from markdown files.
    /// Output: a .ruvector (redb) database file, or .json for legacy format.
    /// Paths are resolved relative to --data-dir.
    ///
    /// Reads curated lore from the input directory (which should already
    /// contain wiki content imported via `import-wiki`).
    BuildIndex {
        /// Input directory containing markdown files (curated lore + imported wiki)
        #[arg(long, default_value = "lore")]
        input: PathBuf,
        /// Output file path (.ruvector for HNSW database, .json for legacy)
        #[arg(long, default_value = "dist/lore.ruvector")]
        output: PathBuf,
    },
    /// Build a response template vector database from JSON files.
    /// Output: a .ruvector (redb) database file, or .json for legacy format.
    /// Paths are resolved relative to --data-dir.
    BuildResponses {
        /// Input directory containing JSON template files
        #[arg(long, default_value = "templates")]
        input: PathBuf,
        /// Output file path (.ruvector for HNSW database, .json for legacy)
        #[arg(long, default_value = "dist/responses.ruvector")]
        output: PathBuf,
    },
    /// Build a personality vector database from JSON personality files.
    /// Output: a .ruvector (redb) database file.
    /// Paths are resolved relative to --data-dir.
    BuildPersonalities {
        /// Input directory containing personality JSON files
        #[arg(long, default_value = "personalities")]
        input: PathBuf,
        /// Output file path (.ruvector for HNSW database)
        #[arg(long, default_value = "dist/personality.ruvector")]
        output: PathBuf,
    },
    /// Reset all vector databases and rebuild from source.
    /// Deletes .ruvector files and memory, then runs build-index + build-responses + build-personalities.
    InitData {
        /// Lore input directory (markdown files)
        #[arg(long, default_value = "lore")]
        lore_input: PathBuf,
        /// Lore output path
        #[arg(long, default_value = "dist/lore.ruvector")]
        lore_output: PathBuf,
        /// Template input directory (JSON files)
        #[arg(long, default_value = "templates")]
        templates_input: PathBuf,
        /// Template output path
        #[arg(long, default_value = "dist/responses.ruvector")]
        templates_output: PathBuf,
        /// Personality input directory (JSON files)
        #[arg(long, default_value = "personalities")]
        personalities_input: PathBuf,
        /// Personality output path
        #[arg(long, default_value = "dist/personality.ruvector")]
        personalities_output: PathBuf,
    },
    /// Import wiki dump files into the curated lore directory structure.
    /// Reads .md files with YAML frontmatter, cleans wiki syntax,
    /// and organizes them by category. Does NOT embed anything.
    ImportWiki {
        /// Wiki dump directory containing .md files with YAML frontmatter
        #[arg(long)]
        wiki_dir: PathBuf,
        /// Output lore directory to write categorized files
        #[arg(long, default_value = "lore")]
        output_dir: PathBuf,
    },
    /// Clean wiki-imported item files: transform raw stat dumps into
    /// conversational prose suitable for LLM context injection.
    /// Cross-references enemies and zones for drop source grounding.
    CleanItems {
        /// Items directory containing wiki-imported .md files
        #[arg(long, default_value = "lore/items")]
        items_dir: PathBuf,
        /// Enemies directory for cross-referencing drop sources
        #[arg(long, default_value = "lore/enemies")]
        enemies_dir: PathBuf,
        /// Zones directory for zone name validation
        #[arg(long, default_value = "lore/zones")]
        zones_dir: PathBuf,
    },
    /// Export training data for LoRA fine-tuning.
    /// Generates JSONL files in ChatML, Alpaca, and/or ShareGPT formats
    /// from personalities, templates, lore, and grounding data.
    ExportTraining {
        /// Output directory for JSONL files
        #[arg(long, default_value = "dist/training")]
        output_dir: PathBuf,
        /// Output format: chatml, alpaca, sharegpt, or all
        #[arg(long, default_value = "all")]
        format: String,
        /// Strategies (comma-separated): phrases, crossover, lore, multiturn, or all
        #[arg(long, default_value = "all")]
        strategies: String,
        /// Deterministic RNG seed
        #[arg(long, default_value = "42")]
        seed: u64,
        /// Max pairs per strategy (0 = unlimited)
        #[arg(long, default_value = "0")]
        max_per_strategy: usize,
        /// Filter personality types (comma-separated): 1=Nice, 2=Tryhard, 3=Mean, 5=Neutral
        #[arg(long)]
        personality_types: Option<String>,
        /// Filter template categories (comma-separated)
        #[arg(long)]
        categories: Option<String>,
        /// Filter zones (comma-separated)
        #[arg(long)]
        zones: Option<String>,
    },
    /// Validate template JSON files for quality, correctness, and entity grounding.
    /// Checks format, forbidden phrases, duplicate IDs, category balance,
    /// and zone_affinity against grounding.json.
    ValidateTemplates {
        /// Validate a specific file instead of all templates
        #[arg(long)]
        input: Option<PathBuf>,
        /// Check entity grounding in template text (slower)
        #[arg(long)]
        check_grounding: bool,
    },
    /// Export SillyTavern character cards (TavernAI Card V2 JSON).
    /// Generates one JSON file per SimPlayer personality for import
    /// into SillyTavern or any TavernAI-compatible frontend.
    ExportTavern {
        /// Output directory for character card JSON files
        #[arg(long, default_value = "dist/tavern")]
        output_dir: PathBuf,
        /// Filter personality types (comma-separated): 1=Nice, 2=Tryhard, 3=Mean, 5=Neutral
        #[arg(long)]
        personality_types: Option<String>,
        /// Include character_book (lorebook) from knowledge_areas
        #[arg(long)]
        include_lorebook: bool,
    },
    /// Fine-tune a local model using exported training data.
    /// Generates training scripts and optionally runs them.
    FineTune {
        /// Training data directory
        #[arg(long, default_value = "dist/training")]
        output_dir: PathBuf,
        /// Backend: config-only, unsloth, or axolotl
        #[arg(long, default_value = "config-only")]
        backend: String,
        /// HuggingFace base model ID
        #[arg(long, default_value = "unsloth/gemma-3-1b-it")]
        base_model: String,
        /// LoRA rank
        #[arg(long, default_value = "16")]
        lora_rank: u32,
        /// Training epochs
        #[arg(long, default_value = "3")]
        epochs: u32,
        /// Learning rate
        #[arg(long, default_value = "2e-4")]
        learning_rate: f64,
        /// RNG seed
        #[arg(long, default_value = "42")]
        seed: u64,
    },
}

fn init_tracing(format: &str) {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    match format {
        "json" => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().json().with_writer(std::io::stderr))
                .init();
        }
        "plain" => {
            // No ANSI codes, no timestamps -- clean output for BepInEx log forwarding.
            // Format: "LEVEL module: message"
            tracing_subscriber::registry()
                .with(filter)
                .with(
                    fmt::layer()
                        .with_writer(std::io::stderr)
                        .with_ansi(false)
                        .without_time(),
                )
                .init();
        }
        _ => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().with_writer(std::io::stderr))
                .init();
        }
    }
}

/// Try to load the embedding engine. Returns None if model files are missing.
fn load_embedder(config: &config::AppConfig) -> Option<std::sync::Arc<intelligence::embedder::EmbeddingEngine>> {
    let model_path = config.resolve_path(&config.embedding.model_path);
    let tokenizer_path = config.resolve_path(&config.embedding.tokenizer_path);

    if !model_path.exists() {
        warn!(
            "ONNX model not found at {:?}. Embedding engine will not be available.",
            model_path
        );
        return None;
    }

    if !tokenizer_path.exists() {
        warn!(
            "Tokenizer not found at {:?}. Embedding engine will not be available.",
            tokenizer_path
        );
        return None;
    }

    match intelligence::embedder::EmbeddingEngine::new(
        &model_path,
        &tokenizer_path,
        config.embedding.threads,
    ) {
        Ok(engine) => {
            info!("Embedding engine loaded successfully ({}d)", engine.dimensions());
            Some(engine)
        }
        Err(e) => {
            error!("Failed to load embedding engine: {}", e);
            None
        }
    }
}

/// Load the lore index. Tries .ruvector first, falls back to .json.
fn load_lore(config: &config::AppConfig) -> intelligence::lore::LoreStore {
    let ruvector_path = config.resolve_path(
        &config::IndexConfig::ruvector_path(&config.indexes.lore_path),
    );
    let json_path = config.resolve_path(
        &config::IndexConfig::json_path(&config.indexes.lore_path),
    );
    let adapter_config = config.vectordb.to_adapter_config();

    let store = intelligence::lore::LoreStore::open(&ruvector_path, &json_path, &adapter_config);
    if store.is_loaded() {
        info!("Lore index loaded: {} entries", store.entry_count());
    } else {
        warn!("Lore index is empty or not found. Starting with empty lore.");
    }
    store
}

/// Load the memory store. Creates a new .ruvector if neither exists.
fn load_memory(config: &config::AppConfig) -> intelligence::memory::MemoryStore {
    let ruvector_path = config.resolve_path(
        &config::IndexConfig::ruvector_path(&config.indexes.memory_path),
    );
    let json_path = config.resolve_path(
        &config::IndexConfig::json_path(&config.indexes.memory_path),
    );
    let adapter_config = config.vectordb.to_adapter_config();

    let store = intelligence::memory::MemoryStore::open(&ruvector_path, &json_path, &adapter_config);
    if store.is_loaded() {
        info!("Memory loaded: {} entries", store.entry_count());
    } else {
        info!("Memory empty, starting fresh.");
    }
    store
}

/// Load the response template store. Tries .ruvector first, falls back to .json.
fn load_responses(config: &config::AppConfig) -> intelligence::templates::ResponseStore {
    let ruvector_path = config.resolve_path(
        &config::IndexConfig::ruvector_path(&config.indexes.responses_path),
    );
    let json_path = config.resolve_path(
        &config::IndexConfig::json_path(&config.indexes.responses_path),
    );
    let adapter_config = config.vectordb.to_adapter_config();

    let store = intelligence::templates::ResponseStore::open(&ruvector_path, &json_path, &adapter_config);
    if store.is_loaded() {
        info!("Response templates loaded: {} entries", store.entry_count());
    } else {
        warn!("Response templates not found. Starting without templates.");
    }
    store
}

/// Load personality store from data/personalities/ directory.
fn load_personalities(config: &config::AppConfig) -> Arc<llm::personality::PersonalityStore> {
    let dir = config.resolve_path("personalities");
    Arc::new(llm::personality::PersonalityStore::load(&dir))
}

/// Hash the personality directory contents for change detection.
///
/// Concatenates file names, sizes, and modification times, then hashes
/// with DefaultHasher. This is fast and sufficient for detecting edits.
fn hash_personality_dir(dir: &std::path::Path) -> String {
    let mut hasher = DefaultHasher::new();
    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => return String::new(),
    };
    entries.sort_by_key(|e| e.file_name());

    for entry in &entries {
        let name = entry.file_name();
        name.to_string_lossy().hash(&mut hasher);
        if let Ok(meta) = entry.metadata() {
            hasher.write_u64(meta.len());
            if let Ok(mtime) = meta.modified() {
                if let Ok(dur) = mtime.duration_since(std::time::SystemTime::UNIX_EPOCH) {
                    hasher.write_u64(dur.as_secs());
                }
            }
        }
    }

    format!("{:x}", hasher.finish())
}

/// Build and load vector-backed personality store at startup.
///
/// Skips rebuild if personality files haven't changed (hash-based detection).
/// The `init-data` command always forces a full rebuild (deletes the ruvector).
/// Requires the embedding engine to be available.
fn load_vector_personalities(
    config: &config::AppConfig,
    embedder: &Option<std::sync::Arc<intelligence::embedder::EmbeddingEngine>>,
) -> intelligence::personality_store::VectorPersonalityStore {
    let ruvector_path = config.resolve_path(&config.indexes.personality_path);
    let personalities_dir = config.resolve_path("personalities");
    let adapter_config = config.vectordb.to_adapter_config();
    let hash_path = config.resolve_path("dist/personality.hash");

    // Check if rebuild is needed by comparing directory hash
    let current_hash = hash_personality_dir(&personalities_dir);

    let needs_rebuild = if ruvector_path.exists() && hash_path.exists() {
        let stored_hash = std::fs::read_to_string(&hash_path).unwrap_or_default();
        let changed = stored_hash.trim() != current_hash;
        if !changed {
            debug!("Personality files unchanged (hash: {}), skipping rebuild", &current_hash[..8.min(current_hash.len())]);
        }
        changed
    } else {
        true
    };

    if needs_rebuild {
        if let Some(ref emb) = embedder {
            debug!("Building personality vectors from {:?}...", personalities_dir);
            // Remove stale .ruvector if present
            if ruvector_path.exists() {
                if let Err(e) = std::fs::remove_file(&ruvector_path) {
                    warn!("Failed to remove stale personality DB: {}", e);
                }
            }
            match builder::personality_builder::build_personality_index(&personalities_dir, &ruvector_path, emb) {
                Ok(()) => {
                    debug!("Personality vectors rebuilt successfully");
                    // Store hash for next startup
                    if let Some(parent) = hash_path.parent() {
                        std::fs::create_dir_all(parent).ok();
                    }
                    if let Err(e) = std::fs::write(&hash_path, &current_hash) {
                        warn!("Failed to write personality hash: {}", e);
                    }
                }
                Err(e) => {
                    warn!("Failed to build personality vectors: {}. Personality vector search will be unavailable.", e);
                }
            }
        } else {
            warn!("Embedding engine unavailable; cannot build personality vectors at startup.");
        }
    }

    let store = intelligence::personality_store::VectorPersonalityStore::open(&ruvector_path, &adapter_config);
    if store.is_loaded() {
        info!("Vector personality store loaded: {} entries", store.entry_count());
    } else {
        info!("Vector personality store empty. Personality vector search will be unavailable.");
    }
    store
}

/// Initialize the LLM router based on configuration.
/// Both local (shimmy) and cloud (OpenRouter) backends are HTTP clients.
fn load_llm_router(config: &config::AppConfig) -> Option<Arc<llm::router::LlmRouter>> {
    if !config.llm.enabled {
        info!("LLM text generation disabled");
        return None;
    }

    let mode = config.llm.mode;

    // Create local backend HTTP client (shimmy) if needed
    let local = if matches!(mode, config::LlmMode::Local | config::LlmMode::Hybrid) {
        match llm::local::LocalBackend::new(&config.llm.local) {
            Ok(backend) => {
                info!(
                    "Local LLM backend configured (shimmy at {})",
                    config.llm.local.endpoint
                );
                Some(Arc::new(backend))
            }
            Err(e) => {
                warn!("Failed to configure local LLM backend: {}. Continuing without.", e);
                None
            }
        }
    } else {
        None
    };

    // Create cloud backend HTTP client (OpenRouter) if needed
    let cloud = if matches!(mode, config::LlmMode::Cloud | config::LlmMode::Hybrid) {
        match llm::cloud::CloudBackend::new(&config.llm.cloud) {
            Ok(backend) => {
                info!("Cloud LLM backend configured (provider: {})", config.llm.cloud.provider);
                Some(Arc::new(backend))
            }
            Err(e) => {
                warn!("Failed to configure cloud LLM backend: {}. Continuing without.", e);
                None
            }
        }
    } else {
        None
    };

    Some(Arc::new(llm::router::LlmRouter::new(local, cloud, mode)))
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    init_tracing(&cli.log_format);

    // Handle subcommands (builder modes)
    if let Some(command) = &cli.command {
        // Builders need the embedding engine
        let config = match config::load_config(cli.config.as_deref(), &cli.data_dir, cli.port, cli.threads) {
            Ok(c) => c,
            Err(e) => {
                error!("Failed to load config: {}", e);
                std::process::exit(1);
            }
        };

        match command {
            Commands::BuildIndex { input, output } => {
                let input = config.resolve_path(&input.to_string_lossy());
                let output = config.resolve_path(&output.to_string_lossy());

                let embedder = match load_embedder(&config) {
                    Some(e) => e,
                    None => {
                        error!("Embedding engine is required for index building. Ensure model files exist.");
                        std::process::exit(1);
                    }
                };

                info!("Building lore index from {:?} -> {:?}", input, output);

                if let Err(e) = builder::index_builder::build_lore_index(&input, &output, &embedder) {
                    error!("Index build failed: {}", e);
                    std::process::exit(1);
                }

                info!("Lore index built successfully.");
                return;
            }
            Commands::BuildResponses { input, output } => {
                // Resolve paths relative to data_dir if not absolute
                let input = config.resolve_path(&input.to_string_lossy());
                let output = config.resolve_path(&output.to_string_lossy());
                info!("Building response templates from {:?} -> {:?}", input, output);

                let embedder = match load_embedder(&config) {
                    Some(e) => e,
                    None => {
                        error!("Embedding engine is required for template building. Ensure model files exist.");
                        std::process::exit(1);
                    }
                };

                if let Err(e) = builder::template_builder::build_response_index(&input, &output, &embedder) {
                    error!("Template build failed: {}", e);
                    std::process::exit(1);
                }

                info!("Response template index built successfully.");
                return;
            }
            Commands::BuildPersonalities { input, output } => {
                let input = config.resolve_path(&input.to_string_lossy());
                let output = config.resolve_path(&output.to_string_lossy());
                info!("Building personality index from {:?} -> {:?}", input, output);

                let embedder = match load_embedder(&config) {
                    Some(e) => e,
                    None => {
                        error!("Embedding engine is required for personality building. Ensure model files exist.");
                        std::process::exit(1);
                    }
                };

                if let Err(e) = builder::personality_builder::build_personality_index(&input, &output, &embedder) {
                    error!("Personality build failed: {}", e);
                    std::process::exit(1);
                }

                info!("Personality index built successfully.");
                return;
            }
            Commands::InitData { lore_input, lore_output, templates_input, templates_output, personalities_input, personalities_output } => {
                let lore_input = config.resolve_path(&lore_input.to_string_lossy());
                let lore_output = config.resolve_path(&lore_output.to_string_lossy());
                let templates_input = config.resolve_path(&templates_input.to_string_lossy());
                let templates_output = config.resolve_path(&templates_output.to_string_lossy());
                let personalities_input = config.resolve_path(&personalities_input.to_string_lossy());
                let personalities_output = config.resolve_path(&personalities_output.to_string_lossy());

                info!("=== init-data: resetting vector databases ===");

                // 1. Delete .ruvector files
                let ruvector_files = [
                    config.resolve_path("dist/lore.ruvector"),
                    config.resolve_path("dist/responses.ruvector"),
                    config.resolve_path("dist/memory.ruvector"),
                    config.resolve_path("dist/personality.ruvector"),
                ];
                for path in &ruvector_files {
                    if path.exists() {
                        info!("Removing {:?}", path);
                        if let Err(e) = std::fs::remove_file(path) {
                            warn!("Failed to remove {:?}: {}", path, e);
                        }
                    }
                }

                // 2. Delete memory.json (reset memories)
                let memory_json = config.resolve_path("dist/memory.json");
                if memory_json.exists() {
                    info!("Removing {:?}", memory_json);
                    if let Err(e) = std::fs::remove_file(&memory_json) {
                        warn!("Failed to remove {:?}: {}", memory_json, e);
                    }
                }

                // 3. Load embedding engine (required for all builds)
                let embedder = match load_embedder(&config) {
                    Some(e) => e,
                    None => {
                        error!("Embedding engine is required for init-data.");
                        std::process::exit(1);
                    }
                };

                // 4. Build lore index (curated lore only, wiki content should be pre-imported)
                info!("=== Building lore index: {:?} -> {:?} ===", lore_input, lore_output);
                if let Err(e) = builder::index_builder::build_lore_index(&lore_input, &lore_output, &embedder) {
                    error!("Lore index build failed: {}", e);
                    std::process::exit(1);
                }
                info!("Lore index built.");

                // 5. Build response templates
                info!("=== Building response templates: {:?} -> {:?} ===", templates_input, templates_output);
                if let Err(e) = builder::template_builder::build_response_index(&templates_input, &templates_output, &embedder) {
                    error!("Template build failed: {}", e);
                    std::process::exit(1);
                }
                info!("Response templates built.");

                // 6. Build personality index
                info!("=== Building personality index: {:?} -> {:?} ===", personalities_input, personalities_output);
                if let Err(e) = builder::personality_builder::build_personality_index(&personalities_input, &personalities_output, &embedder) {
                    error!("Personality build failed: {}", e);
                    std::process::exit(1);
                }
                info!("Personality index built.");

                info!("=== init-data complete ===");
                return;
            }
            Commands::ImportWiki { wiki_dir, output_dir } => {
                let wiki_dir = config.resolve_path(&wiki_dir.to_string_lossy());
                let output_dir = config.resolve_path(&output_dir.to_string_lossy());
                info!("Importing wiki dump from {:?} -> {:?}", wiki_dir, output_dir);

                if let Err(e) = builder::wiki_importer::import_wiki(&wiki_dir, &output_dir) {
                    error!("Wiki import failed: {}", e);
                    std::process::exit(1);
                }

                info!("Wiki import complete.");
                return;
            }
            Commands::CleanItems { items_dir, enemies_dir, zones_dir } => {
                let items_dir = config.resolve_path(&items_dir.to_string_lossy());
                let enemies_dir = config.resolve_path(&enemies_dir.to_string_lossy());
                let zones_dir = config.resolve_path(&zones_dir.to_string_lossy());
                info!("Cleaning item files: {:?}", items_dir);

                match builder::item_cleaner::clean_items(&items_dir, &enemies_dir, &zones_dir) {
                    Ok(count) => {
                        info!("Item cleaning complete: {} files processed.", count);
                    }
                    Err(e) => {
                        error!("Item cleaning failed: {}", e);
                        std::process::exit(1);
                    }
                }
                return;
            }
            Commands::ValidateTemplates { input, check_grounding } => {
                let data_dir = config.data_dir.clone();
                let input_path = input.as_ref().map(|p| {
                    if p.is_absolute() {
                        p.clone()
                    } else {
                        config.resolve_path(&p.to_string_lossy())
                    }
                });

                info!("Validating templates in {:?}", data_dir);

                match builder::template_validator::validate_templates(
                    &data_dir,
                    input_path.as_deref(),
                    *check_grounding,
                ) {
                    Ok(report) => {
                        report.print_summary();
                        if !report.is_valid() {
                            std::process::exit(1);
                        }
                    }
                    Err(e) => {
                        error!("Validation failed: {}", e);
                        std::process::exit(1);
                    }
                }
                return;
            }
            Commands::ExportTavern {
                output_dir,
                personality_types,
                include_lorebook,
            } => {
                use builder::tavern_exporter::TavernExportConfig;

                let output_dir = config.resolve_path(&output_dir.to_string_lossy());

                let type_filter = personality_types.as_ref().map(|s| {
                    s.split(',')
                        .filter_map(|t| t.trim().parse::<u8>().ok())
                        .collect::<Vec<_>>()
                });

                let tavern_config = TavernExportConfig {
                    data_dir: config.data_dir.clone(),
                    output_dir,
                    personality_types: type_filter,
                    include_lorebook: *include_lorebook,
                };

                info!("Exporting SillyTavern character cards...");
                if let Err(e) = builder::tavern_exporter::export_tavern_cards(&tavern_config) {
                    error!("Tavern export failed: {}", e);
                    std::process::exit(1);
                }

                info!("SillyTavern character card export complete.");
                return;
            }
            Commands::ExportTraining {
                output_dir,
                format,
                strategies,
                seed,
                max_per_strategy,
                personality_types,
                categories,
                zones,
            } => {
                use builder::training_exporter::{ExportConfig, OutputFormat, Strategy};

                let output_dir = config.resolve_path(&output_dir.to_string_lossy());

                let formats = match format.to_lowercase().as_str() {
                    "all" => vec![OutputFormat::ChatML, OutputFormat::Alpaca, OutputFormat::ShareGPT],
                    "chatml" => vec![OutputFormat::ChatML],
                    "alpaca" => vec![OutputFormat::Alpaca],
                    "sharegpt" => vec![OutputFormat::ShareGPT],
                    other => {
                        error!("Unknown format: '{}'. Use: chatml, alpaca, sharegpt, or all", other);
                        std::process::exit(1);
                    }
                };

                let strats = match strategies.to_lowercase().as_str() {
                    "all" => Strategy::all().to_vec(),
                    other => {
                        let mut result = Vec::new();
                        for s in other.split(',') {
                            match Strategy::from_str(s.trim()) {
                                Some(strat) => result.push(strat),
                                None => {
                                    error!("Unknown strategy: '{}'. Use: phrases, crossover, lore, multiturn, or all", s.trim());
                                    std::process::exit(1);
                                }
                            }
                        }
                        result
                    }
                };

                let type_filter = personality_types.as_ref().map(|s| {
                    s.split(',')
                        .filter_map(|t| t.trim().parse::<u8>().ok())
                        .collect::<Vec<_>>()
                });

                let cat_filter = categories.as_ref().map(|s| {
                    s.split(',')
                        .map(|c| c.trim().to_string())
                        .collect::<Vec<_>>()
                });

                let zone_filter = zones.as_ref().map(|s| {
                    s.split(',')
                        .map(|z| z.trim().to_string())
                        .collect::<Vec<_>>()
                });

                let export_config = ExportConfig {
                    data_dir: config.data_dir.clone(),
                    output_dir,
                    formats,
                    strategies: strats,
                    seed: *seed,
                    max_per_strategy: *max_per_strategy,
                    personality_types: type_filter,
                    categories: cat_filter,
                    zones: zone_filter,
                };

                info!("Exporting training data...");
                if let Err(e) = builder::training_exporter::export_training(&export_config) {
                    error!("Training export failed: {}", e);
                    std::process::exit(1);
                }

                info!("Training data export complete.");
                return;
            }
            Commands::FineTune {
                output_dir,
                backend,
                base_model,
                lora_rank,
                epochs,
                learning_rate,
                seed,
            } => {
                use builder::training_exporter::{FineTuneBackend, FineTuneConfig};

                let output_dir = config.resolve_path(&output_dir.to_string_lossy());

                let backend = match backend.to_lowercase().as_str() {
                    "config-only" => FineTuneBackend::ConfigOnly,
                    "unsloth" => FineTuneBackend::Unsloth,
                    "axolotl" => FineTuneBackend::Axolotl,
                    other => {
                        error!("Unknown backend: '{}'. Use: config-only, unsloth, or axolotl", other);
                        std::process::exit(1);
                    }
                };

                let ft_config = FineTuneConfig {
                    data_dir: config.data_dir.clone(),
                    output_dir,
                    backend,
                    base_model: base_model.clone(),
                    lora_rank: *lora_rank,
                    epochs: *epochs,
                    learning_rate: *learning_rate,
                    seed: *seed,
                };

                info!("Running fine-tune pipeline...");
                if let Err(e) = builder::training_exporter::fine_tune(&ft_config) {
                    error!("Fine-tune failed: {}", e);
                    std::process::exit(1);
                }

                info!("Fine-tune pipeline complete.");
                return;
            }
        }
    }

    // Server mode: load config and start HTTP server
    let config = match config::load_config(cli.config.as_deref(), &cli.data_dir, cli.port, cli.threads) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to load config: {}", e);
            std::process::exit(1);
        }
    };

    info!(
        "erenshor-llm v{} starting on {}:{}",
        env!("CARGO_PKG_VERSION"),
        config.server.host,
        config.server.port
    );

    // Load intelligence components (graceful degradation if files missing)
    let embedder = load_embedder(&config);

    // Warm-up embedding to pre-initialize ONNX thread pools
    if let Some(ref emb) = embedder {
        match emb.embed("warm-up test") {
            Ok(_) => {}
            Err(e) => error!("Embedding warm-up FAILED: {}", e),
        }
    }

    let lore = load_lore(&config);
    let responses = load_responses(&config);
    let memory = load_memory(&config);

    // Start with empty personality vectors -- built in background after server starts
    let vector_personalities = intelligence::personality_store::VectorPersonalityStore::empty();

    // Initialize SONA adaptive learning
    let sona = if config.sona.enabled {
        match intelligence::sona_integration::SonaManager::new(config.sona.to_manager_config()) {
            Ok(s) => {
                info!("SONA adaptive learning enabled");
                Some(s)
            }
            Err(e) => {
                warn!("Failed to initialize SONA: {}. Continuing without.", e);
                None
            }
        }
    } else {
        info!("SONA adaptive learning disabled");
        None
    };

    // Load Phase 3 components
    let personality_store = load_personalities(&config);

    // Start shimmy if local LLM mode is enabled
    let _shimmy = if config.llm.enabled
        && matches!(config.llm.mode, config::LlmMode::Local | config::LlmMode::Hybrid)
        && config.llm.local.auto_start
    {
        // Extract bind address from endpoint URL (e.g. "http://127.0.0.1:8012" -> "127.0.0.1:8012")
        let bind_addr = config.llm.local.endpoint
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .to_string();

        let shimmy = llm::shimmy::ShimmyProcess::start(
            &config.data_dir,
            &bind_addr,
            &config.llm.local.gpu_backend,
            &config.llm.local.model_dir,
        );

        if shimmy.is_some() {
            // Wait for shimmy to become ready before accepting requests
            let ready = llm::shimmy::ShimmyProcess::wait_ready(
                &config.llm.local.endpoint,
                std::time::Duration::from_secs(60),
            ).await;

            if !ready {
                warn!("Shimmy not ready; local LLM may fail on first requests");
            }
        }

        shimmy
    } else {
        None
    };

    let llm_router = load_llm_router(&config);

    // Load GEPA grounding data for hallucination prevention
    let static_grounding = llm::grounding::StaticGrounding::load(
        &config.resolve_path("grounding.json"),
    );

    let app_state = state::AppState::new(
        config, embedder, lore, responses, memory, sona,
        personality_store, vector_personalities, llm_router,
        static_grounding,
    );

    // Start SONA background tick task
    if app_state.sona.is_some() {
        let interval_ms = app_state.sona.as_ref().unwrap().background_interval_ms();
        let state_ref: Arc<state::AppState> = Arc::clone(&app_state);
        let shutdown: Arc<tokio::sync::Notify> = Arc::clone(&app_state.shutdown);
        tokio::spawn(async move {
            let mut tick_interval = tokio::time::interval(
                std::time::Duration::from_millis(interval_ms),
            );
            loop {
                tokio::select! {
                    _ = tick_interval.tick() => {
                        if let Some(ref sona_mgr) = state_ref.sona {
                            sona_mgr.tick();
                        }
                    }
                    _ = shutdown.notified() => {
                        info!("SONA background tick shutting down");
                        break;
                    }
                }
            }
        });
    }

    // Spawn background task to build personality vectors without blocking startup.
    // The server is already listening; personality search will return empty results
    // until the build completes, at which point the RwLock is swapped.
    {
        let state_for_build = Arc::clone(&app_state);
        tokio::spawn(async move {
            // Clone what the blocking closure needs so state_for_build stays available
            let config_clone = state_for_build.config.clone();
            let embedder_clone = state_for_build.embedder.clone();

            let result = tokio::task::spawn_blocking(move || {
                load_vector_personalities(&config_clone, &embedder_clone)
            })
            .await;

            match result {
                Ok(store) => {
                    let mut guard = state_for_build.vector_personalities.write().await;
                    *guard = store;
                    info!("Background personality vector build complete");
                }
                Err(e) => {
                    warn!("Background personality build task failed: {}", e);
                }
            }
        });
    }

    if let Err(e) = server::serve(app_state).await {
        error!("Server error: {}", e);
        std::process::exit(1);
    }
}
