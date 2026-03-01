//! Application state shared across route handlers.

use crate::config::AppConfig;
use crate::intelligence::embedder::EmbeddingEngine;
use crate::intelligence::lore::LoreStore;
use crate::intelligence::memory::MemoryStore;
use crate::intelligence::personality_store::VectorPersonalityStore;
use crate::intelligence::sona_integration::SonaManager;
use crate::intelligence::templates::ResponseStore;
use crate::llm::grounding::StaticGrounding;
use crate::llm::personality::PersonalityStore;
use crate::llm::router::LlmRouter;
use std::sync::Arc;
use tokio::sync::{Notify, RwLock};

/// Shared application state passed to all route handlers via axum State.
///
/// Stores are accessed directly (no RwLock) because VectorDB handles its own
/// internal locking via redb. MemoryStore.ingest() takes &self, not &mut self.
///
/// Exception: `vector_personalities` is behind a RwLock because it is built
/// in a background task after the server starts (deferred startup).
pub struct AppState {
    pub config: AppConfig,
    pub shutdown: Arc<Notify>,
    pub start_time: std::time::Instant,
    /// ONNX embedding engine (None if model not found/loaded).
    pub embedder: Option<Arc<EmbeddingEngine>>,
    /// Lore knowledge index.
    pub lore: LoreStore,
    /// Response template store.
    pub responses: ResponseStore,
    /// Runtime conversation memory.
    pub memory: MemoryStore,
    /// SONA adaptive learning engine (None if disabled or init failed).
    pub sona: Option<SonaManager>,
    /// Personality store for LLM prompt construction (HashMap-based).
    pub personality_store: Arc<PersonalityStore>,
    /// Vector-backed personality store for semantic search.
    /// Wrapped in RwLock: starts empty, populated by background build task.
    pub vector_personalities: RwLock<VectorPersonalityStore>,
    /// LLM router (local/cloud/hybrid). None when LLM is disabled.
    pub llm_router: Option<Arc<LlmRouter>>,
    /// Static grounding data for GEPA prompt anchoring.
    pub static_grounding: Option<StaticGrounding>,
}

impl AppState {
    /// Create a new AppState with all intelligence components.
    pub fn new(
        config: AppConfig,
        embedder: Option<Arc<EmbeddingEngine>>,
        lore: LoreStore,
        responses: ResponseStore,
        memory: MemoryStore,
        sona: Option<SonaManager>,
        personality_store: Arc<PersonalityStore>,
        vector_personalities: VectorPersonalityStore,
        llm_router: Option<Arc<LlmRouter>>,
        static_grounding: Option<StaticGrounding>,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            shutdown: Arc::new(Notify::new()),
            start_time: std::time::Instant::now(),
            embedder,
            lore,
            responses,
            memory,
            sona,
            personality_store,
            vector_personalities: RwLock::new(vector_personalities),
            llm_router,
            static_grounding,
        })
    }

    /// Get uptime in seconds.
    pub fn uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }
}
