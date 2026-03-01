//! POST /v1/respond endpoint.
//!
//! The primary intelligence endpoint. Accepts dialog context, performs semantic
//! template retrieval with context-aware re-ranking, enriches the response with
//! lore and placeholders, and returns the best response.
//!
//! Optional override fields allow the C# mod to control tuning parameters
//! per-request (template_candidates, lore/memory counts, re-ranking weights).
//! When absent, the sidecar's config defaults are used.

use crate::error::{AppError, AppResult};
use crate::intelligence::enricher;
use crate::intelligence::ranker::{self, RankWeights};
use crate::llm::grounding::GroundingContext;
use crate::llm::postprocess;
use crate::llm::prompt::{LoreContext, MemoryContext, PromptBuilder};
use crate::llm::router::LlmResult;
use crate::state::AppState;
use axum::{extract::State, routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info};

/// Request body for POST /v1/respond
#[derive(Debug, Deserialize)]
pub struct RespondRequest {
    /// The raw player chat message
    pub player_message: String,
    /// One of: "say", "whisper", "party", "guild", "shout", "hail"
    pub channel: String,
    /// Name of the target SimPlayer
    pub sim_name: String,
    /// Personality trait flags
    #[serde(default)]
    pub personality: HashMap<String, bool>,
    /// Current zone name
    #[serde(default)]
    pub zone: String,
    /// SimPlayer's opinion of the player (0.0-10.0)
    #[serde(default = "default_relationship")]
    pub relationship: f32,
    /// Player character name
    #[serde(default)]
    pub player_name: String,
    /// Player level
    #[serde(default)]
    pub player_level: u32,
    /// Player class name
    #[serde(default)]
    pub player_class: String,
    /// Player's guild name
    #[serde(default)]
    pub player_guild: String,
    /// SimPlayer's guild name
    #[serde(default)]
    pub sim_guild: String,
    /// Whether the SimPlayer is a Rival (Friends' Club member)
    #[serde(default)]
    pub sim_is_rival: bool,
    /// Names of current group members
    #[serde(default)]
    pub group_members: Vec<String>,

    // ── Optional overrides (from BepInEx config, override sidecar defaults) ──

    /// Number of template candidates to retrieve before re-ranking.
    #[serde(default)]
    pub template_candidates: Option<usize>,
    /// Number of lore passages to retrieve for context enrichment.
    #[serde(default)]
    pub lore_context_count: Option<usize>,
    /// Number of memory entries to retrieve.
    #[serde(default)]
    pub memory_context_count: Option<usize>,
    /// Re-ranking weight override: semantic similarity.
    #[serde(default)]
    pub w_semantic: Option<f32>,
    /// Re-ranking weight override: channel match.
    #[serde(default)]
    pub w_channel: Option<f32>,
    /// Re-ranking weight override: zone affinity.
    #[serde(default)]
    pub w_zone: Option<f32>,
    /// Re-ranking weight override: personality matching.
    #[serde(default)]
    pub w_personality: Option<f32>,
    /// Re-ranking weight override: relationship level.
    #[serde(default)]
    pub w_relationship: Option<f32>,
}

fn default_relationship() -> f32 {
    5.0
}

/// Response body for POST /v1/respond
#[derive(Debug, Serialize)]
pub struct RespondResponse {
    pub response: String,
    pub template_id: String,
    pub confidence: f32,
    pub source: String,
    pub lore_context: Vec<String>,
    pub memory_context: Vec<String>,
    pub timing: RespondTiming,
    /// Whether SONA modified the query embedding.
    pub sona_enhanced: bool,
    /// If LLM was attempted but failed, the reason for falling back to template.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_fallback_reason: Option<String>,
}

/// Timing breakdown for the respond pipeline.
#[derive(Debug, Serialize)]
pub struct RespondTiming {
    pub embed_ms: u64,
    pub sona_transform_ms: u64,
    pub template_search_ms: u64,
    pub rerank_ms: u64,
    pub lore_search_ms: u64,
    pub memory_search_ms: u64,
    pub llm_ms: u64,
    pub total_ms: u64,
}

/// Minimum semantic similarity for template search.
const TEMPLATE_MIN_SCORE: f32 = 0.15;
/// Minimum lore similarity score.
const LORE_MIN_SCORE: f32 = 0.3;

async fn handle_respond(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RespondRequest>,
) -> AppResult<Json<RespondResponse>> {
    let pipeline_start = Instant::now();

    if request.player_message.is_empty() {
        return Err(AppError::BadRequest(
            "Field 'player_message' is required and must be non-empty".to_string(),
        ));
    }

    let embedder = state.embedder.as_ref().ok_or_else(|| {
        AppError::Unavailable("Embedding model not loaded".to_string())
    })?;

    // Resolve tuning parameters: request override > config default
    let respond_config = &state.config.respond;
    let template_k = request.template_candidates.unwrap_or(respond_config.template_candidates);
    let lore_k = request.lore_context_count.unwrap_or(respond_config.lore_candidates);
    let memory_k = request.memory_context_count.unwrap_or(respond_config.memory_candidates);

    // Resolve re-ranking weights
    let weights = RankWeights::from_config_with_overrides(
        respond_config,
        request.w_semantic,
        request.w_channel,
        request.w_zone,
        request.w_personality,
        request.w_relationship,
    );

    // Step 1: Embed the player message (spawn_blocking to avoid ONNX
    // thread-affinity segfaults with cross-compiled mingw + tokio workers)
    let embed_start = Instant::now();
    let query_embedding = embedder
        .embed_async(request.player_message.clone())
        .await
        .map_err(|e| AppError::Internal(format!("Embedding failed: {}", e)))?;
    let embed_ms = embed_start.elapsed().as_millis() as u64;

    // Step 1b: SONA enhance (Point A) -- apply MicroLoRA to query embedding
    let sona_start = Instant::now();
    let (enhanced_embedding, sona_enhanced) = if let Some(ref sona) = state.sona {
        let enhanced = sona.enhance_query(&query_embedding);
        let was_enhanced = enhanced != query_embedding;
        (enhanced, was_enhanced)
    } else {
        (query_embedding.clone(), false)
    };
    let sona_transform_ms = sona_start.elapsed().as_millis() as u64;

    // Step 2: Search response templates (using enhanced embedding)
    let template_start = Instant::now();
    let candidates = if state.responses.is_loaded() {
        state.responses.search(&enhanced_embedding, template_k, TEMPLATE_MIN_SCORE)
    } else {
        Vec::new()
    };
    let template_search_ms = template_start.elapsed().as_millis() as u64;

    // Step 3: Re-rank candidates using dialog context
    let rerank_start = Instant::now();

    // Auto-derive personality traits from the personality store when not
    // provided in the request. This ensures different sims get different
    // template rankings even when the C# mod doesn't send trait flags.
    let personality_traits = if request.personality.is_empty() {
        let derived = state.personality_store.derive_traits(&request.sim_name);
        debug!(
            "Derived personality traits for '{}': {:?}",
            request.sim_name, derived
        );
        derived
    } else {
        request.personality.clone()
    };

    let rank_ctx = ranker::RankContext {
        channel: request.channel.clone(),
        zone: request.zone.clone(),
        personality: personality_traits,
        relationship: request.relationship,
    };
    let ranked = ranker::rerank(candidates, &rank_ctx, &weights);
    let rerank_ms = rerank_start.elapsed().as_millis() as u64;

    // Step 4: Lore search for context enrichment
    let lore_start = Instant::now();
    let lore_results = if state.lore.is_loaded() {
        state.lore.search(&query_embedding, lore_k, LORE_MIN_SCORE)
    } else {
        Vec::new()
    };
    let lore_search_ms = lore_start.elapsed().as_millis() as u64;

    // Step 5: Memory search
    let memory_start = Instant::now();
    let memory_results = if state.memory.is_loaded() {
        state.memory.search(&query_embedding, memory_k, LORE_MIN_SCORE)
    } else {
        Vec::new()
    };
    let memory_search_ms = memory_start.elapsed().as_millis() as u64;
    let memory_context: Vec<String> = memory_results
        .iter()
        .map(|r| r.text.clone())
        .collect();

    // Step 6: Select the best template and enrich
    let (template_text, template_id, confidence, template_source) = if let Some((best_candidate, score)) =
        ranked.first()
    {
        let enrich_ctx = enricher::EnrichContext {
            player_name: if request.player_name.is_empty() {
                "adventurer".to_string()
            } else {
                request.player_name.clone()
            },
            sim_name: request.sim_name.clone(),
            zone: request.zone.clone(),
            mob_name: None,
            item_name: None,
        };

        let enriched = enricher::enrich(&best_candidate.template.text, &enrich_ctx, &lore_results);

        let source = if lore_results.is_empty() {
            "template".to_string()
        } else {
            "template+lore".to_string()
        };

        (
            enriched,
            best_candidate.template.id.clone(),
            *score,
            source,
        )
    } else {
        // Fallback: no templates matched or all were filtered out
        let fallback_text = if let Some(tmpl) = state.responses.fallback_template() {
            let enrich_ctx = enricher::EnrichContext {
                player_name: if request.player_name.is_empty() {
                    "adventurer".to_string()
                } else {
                    request.player_name.clone()
                },
                sim_name: request.sim_name.clone(),
                zone: request.zone.clone(),
                mob_name: None,
                item_name: None,
            };
            let id = tmpl.id.clone();
            let text = enricher::enrich(&tmpl.text, &enrich_ctx, &[]);
            (text, id, 0.0f32, "fallback".to_string())
        } else {
            (
                "Hmm, interesting.".to_string(),
                "hardcoded_fallback".to_string(),
                0.0f32,
                "fallback".to_string(),
            )
        };
        fallback_text
    };

    // Extract lore context snippets
    let lore_context = enricher::extract_lore_context(&lore_results, 2);

    // Step 7: LLM enhancement (confidence-gated)
    let llm_start = Instant::now();
    let llm_config = &state.config.llm;
    let (response_text, source, llm_fallback_reason) =
        if llm_config.enabled && confidence < llm_config.enhance_threshold {
            if let Some(ref router) = state.llm_router {
                // Build LLM prompt with personality + lore + memory context
                let personality = state.personality_store.get(&request.sim_name);
                let lore_ctx: Vec<LoreContext> = lore_results
                    .iter()
                    .map(|r| LoreContext {
                        text: r.text.clone(),
                    })
                    .collect();
                let memory_ctx: Vec<MemoryContext> = memory_results
                    .iter()
                    .map(|r| MemoryContext {
                        text: r.text.clone(),
                    })
                    .collect();

                // Build GEPA grounding context to prevent hallucinated entity names
                let grounding_ctx = state.static_grounding.as_ref().map(|sg| {
                    GroundingContext::from_search_results(&lore_ctx, &request, sg)
                });

                // Token budget for prompt assembly (shimmy/cloud handle actual context window)
                let context_budget = 2048;
                let prompt = PromptBuilder::build(
                    personality,
                    &lore_ctx,
                    &memory_ctx,
                    &request,
                    context_budget,
                    grounding_ctx.as_ref(),
                );

                let result = router
                    .generate(&prompt, llm_config.max_tokens, llm_config.temperature)
                    .await;

                match result {
                    LlmResult::Success {
                        text,
                        source,
                        latency_ms,
                    } => {
                        info!(
                            "LLM enhanced response for '{}' ({}ms, source={})",
                            request.sim_name, latency_ms, source
                        );

                        // Postprocess: clean markdown/formatting and validate entity names
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

                        // SONA LLM trajectory learning: embed the response
                        // and record a trajectory so SONA learns from LLM output
                        if let Some(ref sona) = state.sona {
                            if let Some(ref emb) = state.embedder {
                                if let Ok(llm_vec) = emb.embed_async(validated.clone()).await {
                                    sona.record_llm_trajectory(
                                        &query_embedding,
                                        &llm_vec,
                                        &state.responses,
                                    );
                                }
                            }
                        }

                        (validated, source, None)
                    }
                    LlmResult::Fallback { reason } => {
                        debug!("LLM fallback: {}", reason);
                        (template_text, template_source, Some(reason))
                    }
                }
            } else {
                (
                    template_text,
                    template_source,
                    Some("LLM router not initialized".to_string()),
                )
            }
        } else {
            // Fast path: high confidence template or LLM disabled
            (template_text, template_source, None)
        };
    let llm_ms = llm_start.elapsed().as_millis() as u64;

    // SONA record (Point B): record trajectory for learning
    if let Some(ref sona) = state.sona {
        sona.record_interaction(
            &query_embedding,  // Original embedding, not enhanced
            &template_id,
            confidence,
            &request.sim_name,
        );
    }

    let total_ms = pipeline_start.elapsed().as_millis() as u64;

    debug!(
        "Respond '{}' -> '{}' (template={}, confidence={:.3}, sona={}, source={}, {}ms, k={}/{}/{})",
        request.player_message, response_text, template_id, confidence,
        sona_enhanced, source, total_ms, template_k, lore_k, memory_k
    );

    Ok(Json(RespondResponse {
        response: response_text,
        template_id,
        confidence,
        source,
        lore_context,
        memory_context,
        timing: RespondTiming {
            embed_ms,
            sona_transform_ms,
            template_search_ms,
            rerank_ms,
            lore_search_ms,
            memory_search_ms,
            llm_ms,
            total_ms,
        },
        sona_enhanced,
        llm_fallback_reason,
    }))
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/v1/respond", post(handle_respond))
}
