use anyhow::{bail, Context, Result};
use reqwest::Client;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::config::LocalLlmConfig;
use crate::llm::cloud::ChatMessage;

/// Request body for OpenAI-compatible chat completions API.
#[derive(serde::Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: usize,
    temperature: f32,
    stream: bool,
    /// Penalize repeated tokens to prevent looping (1.0 = no penalty).
    repetition_penalty: f32,
    /// Penalize tokens based on frequency in the response so far.
    frequency_penalty: f32,
}

/// Response from OpenAI-compatible chat completions API.
#[derive(serde::Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
}

#[derive(serde::Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(serde::Deserialize)]
struct ChoiceMessage {
    content: String,
}

/// Local LLM backend via shimmy (external OpenAI-compatible inference server).
///
/// Shimmy runs as a separate process, loads GGUF models, and handles GPU
/// acceleration (Vulkan/CUDA) natively. We communicate over HTTP on localhost.
pub struct LocalBackend {
    client: Client,
    config: LocalLlmConfig,
}

impl LocalBackend {
    /// Create a new local backend HTTP client.
    /// Does NOT load any model -- shimmy handles model management.
    pub fn new(config: &LocalLlmConfig) -> Result<Self> {
        let timeout = Duration::from_millis(config.timeout_ms);
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .context("Failed to build HTTP client for local backend")?;

        info!(
            "Local LLM backend configured: endpoint={}, model={}",
            config.endpoint, config.model
        );

        Ok(Self {
            client,
            config: config.clone(),
        })
    }

    /// Generate text by calling the local inference server's chat completions API.
    ///
    /// Legacy flat-prompt interface: wraps the prompt as a single user message.
    /// Prefer `generate_chat()` for structured system/user messages.
    pub async fn generate(
        &self,
        prompt: &str,
        max_tokens: usize,
        temperature: f32,
    ) -> Result<String> {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        }];
        self.generate_chat(messages, max_tokens, temperature).await
    }

    /// Generate text from structured chat messages (system + user).
    ///
    /// This is the preferred interface for fine-tuned models that were trained
    /// with separate system/user/assistant messages.
    pub async fn generate_chat(
        &self,
        messages: Vec<ChatMessage>,
        max_tokens: usize,
        temperature: f32,
    ) -> Result<String> {
        let request_body = ChatCompletionRequest {
            model: self.config.model.clone(),
            messages,
            max_tokens,
            temperature,
            stream: false,
            repetition_penalty: 1.5,
            frequency_penalty: 0.7,
        };

        let endpoint = format!("{}/v1/chat/completions", self.config.endpoint);

        debug!(
            "Local LLM request to {} (model: {}, max_tokens: {})",
            endpoint, self.config.model, max_tokens
        );

        // Log the actual messages being sent for debugging prompt issues
        for (i, msg) in request_body.messages.iter().enumerate() {
            debug!(
                "Local LLM message[{}] role={}: {}",
                i, msg.role,
                msg.content.chars().take(500).collect::<String>()
            );
        }

        let response = self
            .client
            .post(&endpoint)
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send request to local inference server")?;

        let status = response.status();

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!(
                "Local inference server error: {} (status {})",
                body.chars().take(200).collect::<String>(),
                status.as_u16()
            );
        }

        let body = response.text().await
            .context("Failed to read local inference server response body")?;

        debug!("Local LLM raw response ({}B): {}", body.len(),
            body.chars().take(500).collect::<String>());

        let completion: ChatCompletionResponse = serde_json::from_str(&body)
            .context(format!("Failed to parse local inference server response: {}",
                body.chars().take(300).collect::<String>()))?;

        completion
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| anyhow::anyhow!("Local inference server returned empty choices"))
    }

    /// Check if the backend endpoint is reachable.
    pub async fn health_check(&self) -> bool {
        let url = format!("{}/v1/models", self.config.endpoint);
        match self.client.get(&url).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(e) => {
                warn!("Local inference server health check failed: {}", e);
                false
            }
        }
    }

    /// Check if the backend is configured (always true if constructed).
    pub fn is_ready(&self) -> bool {
        true
    }

    /// Get the configured model name for reporting.
    pub fn model_name(&self) -> &str {
        &self.config.model
    }
}
