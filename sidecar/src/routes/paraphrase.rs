//! POST /v1/paraphrase endpoint.
//!
//! Enriches canned game dialog through LLM paraphrasing. Unlike /v1/respond
//! (which searches templates and optionally paraphrases), this endpoint takes
//! existing game text and always paraphrases it with personality, lore, and
//! event context.
//!
//! Use cases:
//!   - Group member death reactions
//!   - Loot roll/request messages
//!   - Group invite/join announcements
//!   - Combat callouts (pulling, need heal, low health)
//!   - Zone entry remarks
//!   - Hail/greeting triggers
//!   - Level-up congratulations
//!   - Trade/auction messages
//!
//! The C# mod hooks into SimPlayer dialog triggers and sends the canned text
//! here for enrichment. On failure, the original text is returned unchanged.

use crate::error::{AppError, AppResult};
use crate::llm::grounding::GroundingContext;
use crate::llm::postprocess;
use crate::llm::prompt::{LoreContext, PromptBuilder};
use crate::llm::router::LlmResult;
use crate::state::AppState;
use axum::{extract::State, routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Request body for POST /v1/paraphrase
#[derive(Debug, Deserialize)]
pub struct ParaphraseRequest {
    /// The canned game text to paraphrase.
    pub text: String,
    /// Event trigger type (e.g. "group_death", "loot_request", "group_invite",
    /// "combat_callout", "zone_entry", "hail", "level_up", "trade", "generic").
    #[serde(default = "default_trigger")]
    pub trigger: String,
    /// Name of the SimPlayer speaking.
    pub sim_name: String,
    /// Current zone name.
    #[serde(default)]
    pub zone: String,
    /// Chat channel.
    #[serde(default = "default_channel")]
    pub channel: String,
    /// Personality trait flags (optional -- derived from personality store if empty).
    #[serde(default)]
    pub personality: HashMap<String, bool>,
    /// Relationship level (0.0-10.0).
    #[serde(default = "default_relationship")]
    pub relationship: f32,
    /// Player character name (may be empty for sim-to-sim).
    #[serde(default)]
    pub player_name: String,
    /// Event-specific context key-value pairs.
    /// Examples:
    ///   group_death:    {"dead_member": "Phanty", "cause": "Abyssal Lurker"}
    ///   loot_request:   {"item_name": "Eon Blade of Time"}
    ///   group_invite:   {"target": "Hero", "activity": "dungeon"}
    ///   combat_callout: {"callout": "pulling", "enemy": "Sivakayan Voidmaster"}
    ///   zone_entry:     {"zone_name": "The Bone Pits"}
    ///   level_up:       {"player": "Hero", "new_level": "25"}
    #[serde(default)]
    pub context: HashMap<String, String>,
}

fn default_trigger() -> String {
    "generic".to_string()
}

fn default_channel() -> String {
    "say".to_string()
}

fn default_relationship() -> f32 {
    5.0
}

/// Response body for POST /v1/paraphrase
#[derive(Debug, Serialize)]
pub struct ParaphraseResponse {
    /// The paraphrased text (or original on failure).
    pub text: String,
    /// The original canned text.
    pub original: String,
    /// Whether the text was actually paraphrased (false = returned original).
    pub paraphrased: bool,
    /// Source: "paraphrase", "paraphrase+lore", or "original".
    pub source: String,
    /// Timing breakdown.
    pub timing: ParaphraseTiming,
}

/// Timing breakdown for the paraphrase pipeline.
#[derive(Debug, Serialize)]
pub struct ParaphraseTiming {
    pub embed_ms: u64,
    pub lore_search_ms: u64,
    pub llm_ms: u64,
    pub total_ms: u64,
}

/// Minimum lore similarity score for context enrichment.
const LORE_MIN_SCORE: f32 = 0.3;

async fn handle_paraphrase(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ParaphraseRequest>,
) -> AppResult<Json<ParaphraseResponse>> {
    let pipeline_start = Instant::now();

    if request.text.is_empty() {
        return Err(AppError::BadRequest(
            "Field 'text' is required and must be non-empty".to_string(),
        ));
    }

    // LLM must be available for paraphrasing
    let router = state.llm_router.as_ref().ok_or_else(|| {
        AppError::Unavailable("LLM not enabled -- paraphrase requires LLM".to_string())
    })?;

    let llm_config = &state.config.llm;
    if !llm_config.enabled {
        // Return original text when LLM is disabled (graceful degradation)
        return Ok(Json(ParaphraseResponse {
            text: request.text.clone(),
            original: request.text,
            paraphrased: false,
            source: "original".to_string(),
            timing: ParaphraseTiming {
                embed_ms: 0,
                lore_search_ms: 0,
                llm_ms: 0,
                total_ms: 0,
            },
        }));
    }

    // Step 1: Embed the canned text for lore search
    // Build a richer search query from the text + event context
    let search_query = build_search_query(&request);

    let embed_start = Instant::now();
    let query_embedding = if let Some(ref embedder) = state.embedder {
        match embedder.embed_async(search_query).await {
            Ok(vec) => Some(vec),
            Err(e) => {
                debug!("Embedding failed for paraphrase, skipping lore: {}", e);
                None
            }
        }
    } else {
        None
    };
    let embed_ms = embed_start.elapsed().as_millis() as u64;

    // Step 2: Search lore for relevant context to weave into the paraphrase
    let lore_start = Instant::now();
    let lore_results = if let Some(ref embedding) = query_embedding {
        if state.lore.is_loaded() {
            state.lore.search(embedding, 2, LORE_MIN_SCORE)
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    let lore_search_ms = lore_start.elapsed().as_millis() as u64;

    // Step 3: Build personality context
    let personality_traits = if request.personality.is_empty() {
        state.personality_store.derive_traits(&request.sim_name)
    } else {
        request.personality.clone()
    };

    let personality = state
        .personality_store
        .get_or_generate(&request.sim_name, &personality_traits);

    // Step 4: Build GEPA grounding context
    let lore_ctx: Vec<LoreContext> = lore_results
        .iter()
        .map(|r| LoreContext {
            text: r.text.clone(),
        })
        .collect();

    let grounding_ctx = state.static_grounding.as_ref().map(|sg| {
        GroundingContext::from_event_context(&request.zone, &request.context, sg)
    });

    // Step 5: Build the enriched paraphrase prompt and send to LLM
    let llm_start = Instant::now();
    let messages = PromptBuilder::build_event_paraphrase_messages(
        &personality,
        &request.text,
        &request.trigger,
        &request.context,
        &request.zone,
        &request.channel,
        request.relationship,
        &lore_ctx,
        grounding_ctx.as_ref(),
    );

    let result = router
        .generate_chat(messages, llm_config.max_tokens, llm_config.temperature)
        .await;
    let llm_ms = llm_start.elapsed().as_millis() as u64;

    let (output_text, paraphrased, source) = match result {
        LlmResult::Success {
            text,
            source: _,
            latency_ms,
        } => {
            info!(
                "Paraphrased event '{}' for '{}' ({}ms)",
                request.trigger, request.sim_name, latency_ms
            );

            let cleaned = postprocess::clean(&text);
            let validated = if let Some(ref gc) = grounding_ctx {
                postprocess::validate_entities_full(
                    &cleaned,
                    gc,
                    state.static_grounding.as_ref(),
                )
            } else {
                cleaned
            };

            // SONA learns from event paraphrases
            if let Some(ref sona) = state.sona {
                if let (Some(ref emb), Some(ref q_emb)) = (&state.embedder, &query_embedding) {
                    if let Ok(llm_vec) = emb.embed_async(validated.clone()).await {
                        sona.record_llm_trajectory(q_emb, &llm_vec, &state.responses);
                    }
                }
            }

            let src = if lore_results.is_empty() {
                "paraphrase"
            } else {
                "paraphrase+lore"
            };
            (validated, true, src.to_string())
        }
        LlmResult::Fallback { reason } => {
            warn!(
                "Paraphrase failed for '{}' (trigger={}): {}",
                request.sim_name, request.trigger, reason
            );
            (request.text.clone(), false, "original".to_string())
        }
    };

    let total_ms = pipeline_start.elapsed().as_millis() as u64;

    debug!(
        "Paraphrase '{}' [{}] -> '{}' (sim={}, {}ms, lore={})",
        request.text,
        request.trigger,
        output_text,
        request.sim_name,
        total_ms,
        lore_results.len()
    );

    Ok(Json(ParaphraseResponse {
        text: output_text,
        original: request.text,
        paraphrased,
        source,
        timing: ParaphraseTiming {
            embed_ms,
            lore_search_ms,
            llm_ms,
            total_ms,
        },
    }))
}

/// Build a search query by combining the canned text with event context
/// for better lore retrieval. E.g., for a death event in The Bone Pits,
/// the search query includes the zone and enemy names for more relevant lore.
fn build_search_query(request: &ParaphraseRequest) -> String {
    let mut parts: Vec<&str> = vec![&request.text];

    // Add zone context for better lore search
    if !request.zone.is_empty() {
        parts.push(&request.zone);
    }

    // Add event-specific context values
    for (key, value) in &request.context {
        match key.as_str() {
            "dead_member" | "cause" | "enemy" | "item_name" | "zone_name" | "quest_name" => {
                parts.push(value);
            }
            _ => {}
        }
    }

    parts.join(" ")
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/v1/paraphrase", post(handle_paraphrase))
}
