//! SONA (Self-Optimizing Neural Architecture) integration.
//!
//! Wraps ruvector-sona's SonaEngine to provide adaptive learning in the
//! response pipeline. Records query-response trajectories and applies
//! MicroLoRA transformations to enhance future query embeddings.
//!
//! Two injection points in `/v1/respond`:
//! - Point A: `enhance_query()` applies MicroLoRA to the query embedding
//! - Point B: `record_interaction()` records the trajectory for learning

use ruvector_sona::engine::SonaEngine;
use ruvector_sona::types::SonaConfig as RuvectorSonaConfig;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tracing::{debug, info};

/// Configuration for the SonaManager, mapped from our app config.
#[derive(Debug, Clone)]
pub struct SonaManagerConfig {
    pub enabled: bool,
    pub hidden_dim: usize,
    pub micro_lora_rank: usize,
    pub base_lora_rank: usize,
    pub trajectory_capacity: usize,
    pub background_interval_ms: u64,
    pub pattern_clusters: usize,
    pub quality_threshold: f32,
}

impl Default for SonaManagerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            hidden_dim: 384,
            micro_lora_rank: 1,
            base_lora_rank: 4,
            trajectory_capacity: 5000,
            background_interval_ms: 300_000,
            pattern_clusters: 50,
            quality_threshold: 0.3,
        }
    }
}

/// Statistics exposed via /health.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SonaStats {
    pub enabled: bool,
    pub trajectory_count: usize,
    pub pattern_count: usize,
    pub learning_cycles: u64,
    pub queries_enhanced: u64,
    pub llm_trajectories_learned: u64,
}

/// Manager wrapping ruvector-sona's SonaEngine.
///
/// Thread safety: SonaEngine uses internal locking (parking_lot::RwLock).
/// We wrap it in a Mutex for the enhance/record operations since both
/// potentially mutate internal state. Lock duration is very short (<100us).
pub struct SonaManager {
    engine: Mutex<SonaEngine>,
    enabled: bool,
    queries_enhanced: AtomicU64,
    learning_cycles: AtomicU64,
    llm_trajectories_count: AtomicU64,
    background_interval_ms: u64,
}

impl SonaManager {
    /// Create a new SonaManager with the given configuration.
    pub fn new(config: SonaManagerConfig) -> anyhow::Result<Self> {
        let ruvector_config = RuvectorSonaConfig {
            hidden_dim: config.hidden_dim,
            embedding_dim: config.hidden_dim,
            micro_lora_rank: config.micro_lora_rank.clamp(1, 2),
            base_lora_rank: config.base_lora_rank,
            trajectory_capacity: config.trajectory_capacity,
            background_interval_ms: config.background_interval_ms,
            pattern_clusters: config.pattern_clusters,
            quality_threshold: config.quality_threshold,
            ..RuvectorSonaConfig::default()
        };

        let engine = SonaEngine::with_config(ruvector_config);

        info!(
            "SONA initialized: hidden_dim={}, micro_rank={}, base_rank={}, capacity={}",
            config.hidden_dim, config.micro_lora_rank, config.base_lora_rank,
            config.trajectory_capacity
        );

        Ok(Self {
            engine: Mutex::new(engine),
            enabled: config.enabled,
            queries_enhanced: AtomicU64::new(0),
            learning_cycles: AtomicU64::new(0),
            llm_trajectories_count: AtomicU64::new(0),
            background_interval_ms: config.background_interval_ms,
        })
    }

    /// Apply MicroLoRA to a query embedding (Point A).
    ///
    /// Returns the enhanced embedding. If SONA is disabled or the lock
    /// cannot be acquired, returns the original embedding unchanged.
    /// Caps maximum modification (L2 norm of delta) at 0.5 for safety.
    pub fn enhance_query(&self, embedding: &[f32]) -> Vec<f32> {
        if !self.enabled {
            return embedding.to_vec();
        }

        let engine = match self.engine.lock() {
            Ok(e) => e,
            Err(_) => return embedding.to_vec(),
        };

        if !engine.is_enabled() {
            return embedding.to_vec();
        }

        let mut output = vec![0.0f32; embedding.len()];
        engine.apply_micro_lora(embedding, &mut output);

        // Check if LoRA actually modified the output (output is additive delta)
        let has_delta = output.iter().any(|&v| v.abs() > 1e-10);
        if !has_delta {
            return embedding.to_vec();
        }

        // Apply delta to original embedding: enhanced = original + delta
        let mut enhanced: Vec<f32> = embedding
            .iter()
            .zip(output.iter())
            .map(|(&orig, &delta)| orig + delta)
            .collect();

        // Safety cap: if the delta L2 norm is too large, scale it down
        let delta_norm: f32 = output.iter().map(|x| x * x).sum::<f32>().sqrt();
        if delta_norm > 0.5 {
            let scale = 0.5 / delta_norm;
            enhanced = embedding
                .iter()
                .zip(output.iter())
                .map(|(&orig, &delta)| orig + delta * scale)
                .collect();
            debug!("SONA delta capped: norm={:.4} -> scaled to 0.5", delta_norm);
        }

        self.queries_enhanced.fetch_add(1, Ordering::Relaxed);
        enhanced
    }

    /// Record a query-response trajectory (Point B).
    ///
    /// Called after the response has been selected. Uses the re-ranked
    /// confidence score as the quality metric.
    pub fn record_interaction(
        &self,
        query_embedding: &[f32],
        template_id: &str,
        confidence: f32,
        sim_name: &str,
    ) {
        if !self.enabled {
            return;
        }

        let engine = match self.engine.lock() {
            Ok(e) => e,
            Err(_) => return,
        };

        if !engine.is_enabled() {
            return;
        }

        // Build trajectory: query -> template selection step
        let mut builder = engine.begin_trajectory(query_embedding.to_vec());

        // Add a step representing the template selection.
        // activations = query embedding (used for gradient estimation)
        // attention_weights = empty (not applicable for our use case)
        // reward = confidence score
        builder.add_step(query_embedding.to_vec(), vec![], confidence);

        // End trajectory with overall quality = confidence
        engine.end_trajectory(builder, confidence);

        debug!(
            "SONA recorded: sim={}, template={}, confidence={:.3}",
            sim_name, template_id, confidence
        );
    }

    /// Record an LLM response trajectory for learning.
    ///
    /// 1. Records query -> LLM response trajectory in SONA
    /// 2. Finds nearest template to LLM response
    /// 3. If cosine > 0.7, reinforces query -> template path
    pub fn record_llm_trajectory(
        &self,
        query_embedding: &[f32],
        llm_response_embedding: &[f32],
        response_store: &crate::intelligence::templates::ResponseStore,
    ) {
        if !self.enabled {
            return;
        }

        let engine = match self.engine.lock() {
            Ok(e) => e,
            Err(_) => return,
        };

        if !engine.is_enabled() {
            return;
        }

        // Record query -> LLM response trajectory
        let mut builder = engine.begin_trajectory(query_embedding.to_vec());
        builder.add_step(llm_response_embedding.to_vec(), vec![], 1.0);
        engine.end_trajectory(builder, 1.0);
        self.llm_trajectories_count.fetch_add(1, Ordering::Relaxed);

        // Find nearest template to LLM response and reinforce if close
        let candidates = response_store.search(llm_response_embedding, 1, 0.3);
        if let Some(best) = candidates.first() {
            if best.semantic_score > 0.7 {
                // Reinforce query -> template path with the similarity as quality
                let mut builder2 = engine.begin_trajectory(query_embedding.to_vec());
                builder2.add_step(query_embedding.to_vec(), vec![], best.semantic_score);
                engine.end_trajectory(builder2, best.semantic_score);
                debug!(
                    "SONA: LLM response reinforced template {} (cosine: {:.3})",
                    best.template.id, best.semantic_score
                );
            }
        }
    }

    /// Background tick: process accumulated trajectories.
    ///
    /// Called periodically by the background tokio task.
    pub fn tick(&self) {
        if !self.enabled {
            return;
        }

        let engine = match self.engine.lock() {
            Ok(e) => e,
            Err(_) => return,
        };

        if let Some(result) = engine.tick() {
            self.learning_cycles.fetch_add(1, Ordering::Relaxed);
            debug!("SONA tick: {}", result);
        }
    }

    /// Get the background tick interval in milliseconds.
    pub fn background_interval_ms(&self) -> u64 {
        self.background_interval_ms
    }

    /// Get statistics for /health endpoint.
    pub fn stats(&self) -> SonaStats {
        let (trajectory_count, pattern_count) = match self.engine.lock() {
            Ok(engine) => {
                let stats = engine.stats();
                (stats.trajectories_buffered, stats.patterns_stored)
            }
            Err(_) => (0, 0),
        };

        SonaStats {
            enabled: self.enabled,
            trajectory_count,
            pattern_count,
            learning_cycles: self.learning_cycles.load(Ordering::Relaxed),
            queries_enhanced: self.queries_enhanced.load(Ordering::Relaxed),
            llm_trajectories_learned: self.llm_trajectories_count.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_manager() -> SonaManager {
        SonaManager::new(SonaManagerConfig {
            enabled: true,
            hidden_dim: 64,
            micro_lora_rank: 1,
            base_lora_rank: 4,
            trajectory_capacity: 100,
            background_interval_ms: 60_000,
            pattern_clusters: 10,
            quality_threshold: 0.3,
        })
        .unwrap()
    }

    #[test]
    fn test_sona_manager_creation() {
        let mgr = create_test_manager();
        let stats = mgr.stats();
        assert!(stats.enabled);
        assert_eq!(stats.trajectory_count, 0);
        assert_eq!(stats.queries_enhanced, 0);
    }

    #[test]
    fn test_enhance_returns_same_dimensions() {
        let mgr = create_test_manager();
        let input = vec![0.5f32; 64];
        let output = mgr.enhance_query(&input);
        assert_eq!(output.len(), 64);
    }

    #[test]
    fn test_record_interaction_increments_trajectory() {
        let mgr = create_test_manager();
        mgr.record_interaction(&vec![0.1; 64], "tmpl_001", 0.85, "Bumknee");
        let stats = mgr.stats();
        assert_eq!(stats.trajectory_count, 1);
    }

    #[test]
    fn test_tick_does_not_panic() {
        let mgr = create_test_manager();
        mgr.tick(); // Should not panic even with no trajectories
    }

    #[test]
    fn test_disabled_sona_passthrough() {
        let mgr = SonaManager::new(SonaManagerConfig {
            enabled: false,
            hidden_dim: 64,
            ..SonaManagerConfig::default()
        })
        .unwrap();

        let input = vec![1.0f32; 64];
        let output = mgr.enhance_query(&input);
        assert_eq!(input, output); // Should be unchanged
    }

    #[test]
    fn test_multiple_interactions() {
        let mgr = create_test_manager();
        for i in 0..10 {
            mgr.record_interaction(
                &vec![0.1 * i as f32; 64],
                &format!("tmpl_{:03}", i),
                0.5 + 0.05 * i as f32,
                "TestSim",
            );
        }
        let stats = mgr.stats();
        assert_eq!(stats.trajectory_count, 10);
    }
}
