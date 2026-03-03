use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

use crate::config::LlmMode;
use crate::llm::cloud::{ChatMessage, CloudBackend};
use crate::llm::local::LocalBackend;
use crate::llm::postprocess;

/// Result of an LLM generation attempt.
pub enum LlmResult {
    /// Successful generation.
    Success {
        text: String,
        source: String,
        latency_ms: u64,
    },
    /// Fell back to template (LLM failed or unavailable).
    Fallback { reason: String },
}

/// Routes LLM requests to local (shimmy), cloud (OpenRouter), or hybrid backends.
/// Both backends use OpenAI-compatible HTTP APIs.
pub struct LlmRouter {
    local: Option<Arc<LocalBackend>>,
    cloud: Option<Arc<CloudBackend>>,
    mode: LlmMode,
}

impl LlmRouter {
    pub fn new(
        local: Option<Arc<LocalBackend>>,
        cloud: Option<Arc<CloudBackend>>,
        mode: LlmMode,
    ) -> Self {
        info!(
            "LLM router initialized: mode={:?}, local={}, cloud={}",
            mode,
            local.is_some(),
            cloud.is_some()
        );
        Self { local, cloud, mode }
    }

    /// Generate text using the configured backend(s) with a flat prompt.
    ///
    /// Legacy interface -- wraps the prompt as a single user message.
    /// Prefer `generate_chat()` for fine-tuned models with structured messages.
    pub async fn generate(
        &self,
        prompt: &str,
        max_tokens: usize,
        temperature: f32,
    ) -> LlmResult {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        }];
        self.generate_chat(messages, max_tokens, temperature).await
    }

    /// Generate text using structured chat messages (system + user).
    ///
    /// This is the preferred interface for fine-tuned models trained with
    /// separate system/user/assistant turns.
    pub async fn generate_chat(
        &self,
        messages: Vec<ChatMessage>,
        max_tokens: usize,
        temperature: f32,
    ) -> LlmResult {
        match self.mode {
            LlmMode::Off => LlmResult::Fallback {
                reason: "LLM disabled".to_string(),
            },
            LlmMode::Local => self.generate_local_chat(messages, max_tokens, temperature).await,
            LlmMode::Cloud => {
                self.generate_cloud_chat(messages, max_tokens, temperature).await
            }
            LlmMode::Hybrid => {
                self.generate_hybrid_chat(messages, max_tokens, temperature).await
            }
        }
    }

    /// Generate via the local inference server with structured messages.
    async fn generate_local_chat(
        &self,
        messages: Vec<ChatMessage>,
        max_tokens: usize,
        temperature: f32,
    ) -> LlmResult {
        let Some(local) = &self.local else {
            return LlmResult::Fallback {
                reason: "Local backend not configured".to_string(),
            };
        };

        let start = Instant::now();
        match local.generate_chat(messages, max_tokens, temperature).await {
            Ok(raw) => {
                let text = postprocess::clean(&raw);
                if text.is_empty() {
                    return LlmResult::Fallback {
                        reason: "LLM generated empty response".to_string(),
                    };
                }
                let latency_ms = start.elapsed().as_millis() as u64;
                debug!("Local LLM generated in {}ms", latency_ms);
                LlmResult::Success {
                    text,
                    source: "llm_local".to_string(),
                    latency_ms,
                }
            }
            Err(e) => {
                warn!("Local LLM error: {}", e);
                LlmResult::Fallback {
                    reason: format!("Local LLM error: {}", e),
                }
            }
        }
    }

    async fn generate_cloud_chat(
        &self,
        messages: Vec<ChatMessage>,
        max_tokens: usize,
        temperature: f32,
    ) -> LlmResult {
        let Some(cloud) = &self.cloud else {
            return LlmResult::Fallback {
                reason: "Cloud backend not configured".to_string(),
            };
        };

        let start = Instant::now();
        match cloud.generate(messages, max_tokens, temperature).await {
            Ok(raw) => {
                let text = postprocess::clean(&raw);
                if text.is_empty() {
                    return LlmResult::Fallback {
                        reason: "Cloud LLM generated empty response".to_string(),
                    };
                }
                let latency_ms = start.elapsed().as_millis() as u64;
                debug!("Cloud LLM generated in {}ms", latency_ms);
                LlmResult::Success {
                    text,
                    source: "llm_cloud".to_string(),
                    latency_ms,
                }
            }
            Err(e) => {
                warn!("Cloud LLM error: {}", e);
                LlmResult::Fallback {
                    reason: format!("Cloud LLM error: {}", e),
                }
            }
        }
    }

    async fn generate_hybrid_chat(
        &self,
        messages: Vec<ChatMessage>,
        max_tokens: usize,
        temperature: f32,
    ) -> LlmResult {
        // Try local first, fall back to cloud
        let local_result = self.generate_local_chat(
            messages.clone(), max_tokens, temperature,
        ).await;

        match local_result {
            LlmResult::Success { .. } => local_result,
            LlmResult::Fallback { reason } => {
                debug!("Local failed ({}), trying cloud fallback", reason);
                self.generate_cloud_chat(messages, max_tokens, temperature).await
            }
        }
    }

    /// Get the current mode.
    pub fn mode(&self) -> &LlmMode {
        &self.mode
    }

    /// Check if any backend is available.
    pub fn is_available(&self) -> bool {
        match self.mode {
            LlmMode::Off => false,
            LlmMode::Local => self.local.is_some(),
            LlmMode::Cloud => self.cloud.is_some(),
            LlmMode::Hybrid => self.local.is_some() || self.cloud.is_some(),
        }
    }

    /// Status string for health reporting.
    pub fn status(&self) -> &str {
        if self.is_available() {
            "ready"
        } else {
            "unavailable"
        }
    }

    /// Model name for health reporting.
    pub fn model_name(&self) -> String {
        match self.mode {
            LlmMode::Local | LlmMode::Hybrid => {
                if let Some(ref local) = self.local {
                    format!("shimmy:{}", local.model_name())
                } else {
                    "none".to_string()
                }
            }
            LlmMode::Cloud => "cloud".to_string(),
            LlmMode::Off => "disabled".to_string(),
        }
    }
}
