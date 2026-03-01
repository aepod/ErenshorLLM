//! Context-aware re-ranking of template candidates.
//!
//! Applies the weighted scoring formula:
//!   final_score = semantic(w_s) + channel(w_c) + zone(w_z)
//!               + personality(w_p) + relationship(w_r)
//!
//! Weights default to config values but can be overridden per-request
//! via optional fields on the respond request body.
//!
//! Hard-filters templates outside the relationship range before scoring.

use crate::intelligence::templates::TemplateCandidate;
use std::collections::HashMap;
use tracing::debug;

/// The context for re-ranking (derived from the respond request).
pub struct RankContext {
    /// Chat channel: "say", "whisper", "party", "guild", "shout", "hail"
    pub channel: String,
    /// Current zone name
    pub zone: String,
    /// Personality trait flags (e.g. {"social": true, "friendly": true})
    pub personality: HashMap<String, bool>,
    /// Relationship level (0.0 - 10.0)
    pub relationship: f32,
}

/// Re-ranking weights, either from config or per-request overrides.
#[derive(Debug, Clone, Copy)]
pub struct RankWeights {
    pub semantic: f32,
    pub channel: f32,
    pub zone: f32,
    pub personality: f32,
    pub relationship: f32,
}

impl Default for RankWeights {
    fn default() -> Self {
        Self {
            semantic: 0.20,
            channel: 0.15,
            zone: 0.20,
            personality: 0.30,
            relationship: 0.15,
        }
    }
}

impl RankWeights {
    /// Build weights from config defaults, with optional per-request overrides.
    pub fn from_config_with_overrides(
        config: &crate::config::RespondConfig,
        w_semantic: Option<f32>,
        w_channel: Option<f32>,
        w_zone: Option<f32>,
        w_personality: Option<f32>,
        w_relationship: Option<f32>,
    ) -> Self {
        Self {
            semantic: w_semantic.unwrap_or(config.semantic_weight),
            channel: w_channel.unwrap_or(config.channel_weight),
            zone: w_zone.unwrap_or(config.zone_weight),
            personality: w_personality.unwrap_or(config.personality_weight),
            relationship: w_relationship.unwrap_or(config.relationship_weight),
        }
    }
}

/// Re-rank a list of template candidates given the dialog context.
///
/// Steps:
/// 1. Hard-filter templates outside the relationship range
/// 2. Score each remaining candidate with the weighted formula
/// 3. Sort by final_score descending
/// 4. Tie-break randomly if top-2 scores are within 0.001
pub fn rerank(
    candidates: Vec<TemplateCandidate>,
    ctx: &RankContext,
    weights: &RankWeights,
) -> Vec<(TemplateCandidate, f32)> {
    // Step 1: Hard filter by relationship range
    let filtered: Vec<TemplateCandidate> = candidates
        .into_iter()
        .filter(|c| {
            ctx.relationship >= c.template.relationship_min
                && ctx.relationship <= c.template.relationship_max
        })
        .collect();

    if filtered.is_empty() {
        return Vec::new();
    }

    // Step 2: Score each candidate
    let mut scored: Vec<(TemplateCandidate, f32)> = filtered
        .into_iter()
        .map(|c| {
            let channel_score = compute_channel_score(&c.template.channel, &ctx.channel);
            let zone_score = compute_zone_score(&c.template.zone_affinity, &ctx.zone);
            let personality_score =
                compute_personality_score(&c.template.personality_affinity, &ctx.personality);
            let relationship_score =
                compute_relationship_score(ctx.relationship, c.template.relationship_min, c.template.relationship_max);

            let final_score = c.semantic_score * weights.semantic
                + channel_score * weights.channel
                + zone_score * weights.zone
                + personality_score * weights.personality
                + relationship_score * weights.relationship;

            // Apply template priority multiplier
            let final_score = final_score * c.template.priority;

            debug!(
                "Rerank '{}': sem={:.3} ch={:.3} zone={:.3} pers={:.3} rel={:.3} -> {:.4} (w: {:.2}/{:.2}/{:.2}/{:.2}/{:.2})",
                c.template.id,
                c.semantic_score,
                channel_score,
                zone_score,
                personality_score,
                relationship_score,
                final_score,
                weights.semantic, weights.channel, weights.zone,
                weights.personality, weights.relationship,
            );

            (c, final_score)
        })
        .collect();

    // Step 3: Add small random jitter (±5%) to break ties between
    // similarly-scored templates. This produces natural variation when
    // multiple sims respond to the same message -- different sims won't
    // always pick the exact same template from a cluster of close scores.
    for (_, score) in scored.iter_mut() {
        let jitter = (rand::random::<f32>() - 0.5) * 0.20 * *score; // ±10%
        *score += jitter;
    }

    // Step 4: Sort by jittered final_score descending
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    scored
}

/// Channel score: 1.0 if template's channel list contains the context channel,
/// 0.5 if the template's channel list is empty (wildcard), 0.0 otherwise.
fn compute_channel_score(template_channels: &[String], context_channel: &str) -> f32 {
    if template_channels.is_empty() {
        return 0.5; // Wildcard -- template works for any channel
    }
    if template_channels
        .iter()
        .any(|c| c.eq_ignore_ascii_case(context_channel))
    {
        1.0
    } else {
        0.0
    }
}

/// Zone score: 1.0 if template's zone_affinity contains the current zone,
/// 0.5 if the template has no zone affinity (works anywhere), 0.0 otherwise.
fn compute_zone_score(template_zones: &[String], context_zone: &str) -> f32 {
    if template_zones.is_empty() {
        return 0.5; // No zone restriction
    }
    if context_zone.is_empty() {
        return 0.3; // No zone context available
    }
    if template_zones
        .iter()
        .any(|z| z.eq_ignore_ascii_case(context_zone))
    {
        1.0
    } else {
        0.0
    }
}

/// Personality score: (count of matching traits) / (total traits in template).
/// If the template has no personality affinity, return 0.5.
fn compute_personality_score(
    template_traits: &[String],
    context_personality: &HashMap<String, bool>,
) -> f32 {
    if template_traits.is_empty() {
        return 0.5; // No personality restriction
    }

    let matching = template_traits
        .iter()
        .filter(|trait_name| {
            context_personality
                .get(trait_name.as_str())
                .copied()
                .unwrap_or(false)
        })
        .count();

    matching as f32 / template_traits.len() as f32
}

/// Relationship score: how well the context relationship fits within the
/// template's [min, max] range. Returns 1.0 if centered, decays toward edges.
fn compute_relationship_score(relationship: f32, min: f32, max: f32) -> f32 {
    if max <= min {
        return 0.5;
    }
    // Normalize: 0.0 at min edge, 1.0 at center, 0.0 at max edge
    // Use a simpler model: linear from 0.5 to 1.0 toward center
    let range = max - min;
    let center = (min + max) / 2.0;
    let distance_from_center = (relationship - center).abs();
    let half_range = range / 2.0;

    if half_range == 0.0 {
        return 1.0;
    }

    // Score: 1.0 at center, 0.5 at edges
    let normalized_distance = distance_from_center / half_range;
    1.0 - (normalized_distance * 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_score() {
        assert_eq!(compute_channel_score(&[], "say"), 0.5);
        assert_eq!(
            compute_channel_score(&["say".to_string(), "whisper".to_string()], "say"),
            1.0
        );
        assert_eq!(
            compute_channel_score(&["party".to_string()], "say"),
            0.0
        );
    }

    #[test]
    fn test_zone_score() {
        assert_eq!(compute_zone_score(&[], "Stormhaven"), 0.5);
        assert_eq!(
            compute_zone_score(&["Stormhaven".to_string()], "Stormhaven"),
            1.0
        );
        assert_eq!(
            compute_zone_score(&["Stormhaven".to_string()], "Port Azure"),
            0.0
        );
        assert_eq!(compute_zone_score(&["Stormhaven".to_string()], ""), 0.3);
    }

    #[test]
    fn test_personality_score() {
        let mut personality = HashMap::new();
        personality.insert("social".to_string(), true);
        personality.insert("friendly".to_string(), true);
        personality.insert("aggressive".to_string(), false);

        assert_eq!(compute_personality_score(&[], &personality), 0.5);
        assert_eq!(
            compute_personality_score(
                &["social".to_string(), "friendly".to_string()],
                &personality
            ),
            1.0
        );
        assert_eq!(
            compute_personality_score(
                &["social".to_string(), "aggressive".to_string()],
                &personality
            ),
            0.5
        );
        assert_eq!(
            compute_personality_score(&["scholarly".to_string()], &personality),
            0.0
        );
    }

    #[test]
    fn test_relationship_score() {
        // Center of [0, 10] range at relationship 5.0
        let score = compute_relationship_score(5.0, 0.0, 10.0);
        assert!((score - 1.0).abs() < 0.01);

        // Edge of [0, 10] range
        let score = compute_relationship_score(0.0, 0.0, 10.0);
        assert!((score - 0.5).abs() < 0.01);

        // Center of [6, 10] range at relationship 8.0
        let score = compute_relationship_score(8.0, 6.0, 10.0);
        assert!((score - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_default_weights() {
        let w = RankWeights::default();
        let sum = w.semantic + w.channel + w.zone + w.personality + w.relationship;
        assert!((sum - 1.0).abs() < 0.01);
    }
}
