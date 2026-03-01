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

/// Routes LLM requests to local, cloud, or hybrid backends.
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

    /// Generate text using the configured backend(s).
    pub async fn generate(
        &self,
        prompt: &str,
        max_tokens: usize,
        temperature: f32,
    ) -> LlmResult {
        match self.mode {
            LlmMode::Off => LlmResult::Fallback {
                reason: "LLM disabled".to_string(),
            },
            LlmMode::Local => self.generate_local(prompt, max_tokens, temperature).await,
            LlmMode::Cloud => {
                self.generate_cloud(prompt, max_tokens, temperature).await
            }
            LlmMode::Hybrid => {
                self.generate_hybrid(prompt, max_tokens, temperature).await
            }
        }
    }

    /// Run local llama.cpp inference on a blocking thread to avoid segfaults
    /// from cross-compiled mingw native code on tokio worker threads.
    async fn generate_local(&self, prompt: &str, max_tokens: usize, temperature: f32) -> LlmResult {
        let Some(local) = &self.local else {
            return LlmResult::Fallback {
                reason: "Local backend not loaded".to_string(),
            };
        };

        let local = Arc::clone(local);
        let prompt = prompt.to_string();

        match tokio::task::spawn_blocking(move || {
            let start = Instant::now();
            match local.generate(&prompt, max_tokens, temperature) {
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
        }).await {
            Ok(result) => result,
            Err(e) => LlmResult::Fallback {
                reason: format!("Local LLM task panicked: {}", e),
            },
        }
    }

    async fn generate_cloud(
        &self,
        prompt: &str,
        max_tokens: usize,
        temperature: f32,
    ) -> LlmResult {
        let Some(cloud) = &self.cloud else {
            return LlmResult::Fallback {
                reason: "Cloud backend not configured".to_string(),
            };
        };

        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        }];

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

    async fn generate_hybrid(
        &self,
        prompt: &str,
        max_tokens: usize,
        temperature: f32,
    ) -> LlmResult {
        // Try local first (primary), fall back to cloud
        let local_result = self.generate_local(prompt, max_tokens, temperature).await;

        match local_result {
            LlmResult::Success { .. } => local_result,
            LlmResult::Fallback { reason } => {
                debug!("Local failed ({}), trying cloud fallback", reason);
                self.generate_cloud(prompt, max_tokens, temperature).await
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
                if self.local.is_some() {
                    "local_gguf".to_string()
                } else {
                    "none".to_string()
                }
            }
            LlmMode::Cloud => "cloud".to_string(),
            LlmMode::Off => "disabled".to_string(),
        }
    }
}
