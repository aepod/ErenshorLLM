use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, warn};

use crate::config::CloudLlmConfig;

/// Chat message for OpenAI-compatible API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: usize,
    temperature: f32,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    content: String,
}

/// Cloud LLM backend using OpenRouter (OpenAI-compatible API).
pub struct CloudBackend {
    client: Client,
    config: CloudLlmConfig,
}

impl CloudBackend {
    /// Create a new cloud backend with the given config.
    pub fn new(config: &CloudLlmConfig) -> Result<Self> {
        if config.api_key.is_empty() {
            bail!("Cloud LLM API key is empty");
        }

        let timeout = Duration::from_millis(config.timeout_ms);
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            client,
            config: config.clone(),
        })
    }

    /// Generate text using the cloud LLM API.
    pub async fn generate(
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
        };

        let endpoint = format!("{}/chat/completions", self.config.api_endpoint);

        debug!(
            "Cloud LLM request to {} (model: {}, max_tokens: {})",
            endpoint, self.config.model, max_tokens
        );

        // Retry with exponential backoff on 429/5xx
        let mut last_err = None;
        let backoffs = [500u64, 1000, 2000];

        for (attempt, &backoff_ms) in std::iter::once(&0u64).chain(backoffs.iter()).enumerate() {
            if attempt > 0 {
                debug!("Retry attempt {} after {}ms backoff", attempt, backoff_ms);
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            }

            match self.send_request(&endpoint, &request_body).await {
                Ok(text) => return Ok(text),
                Err(e) => {
                    if is_retryable_error(&e) && attempt < backoffs.len() {
                        warn!("Retryable cloud LLM error (attempt {}): {}", attempt + 1, e);
                        last_err = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("Cloud LLM failed after retries")))
    }

    async fn send_request(
        &self,
        endpoint: &str,
        body: &ChatCompletionRequest,
    ) -> Result<String> {
        let response = self
            .client
            .post(endpoint)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await
            .context("Failed to send cloud LLM request")?;

        let status = response.status();

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!(
                "Cloud LLM API error: {} (status {})",
                body.chars().take(200).collect::<String>(),
                status.as_u16()
            );
        }

        let completion: ChatCompletionResponse = response
            .json()
            .await
            .context("Failed to parse cloud LLM response")?;

        completion
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| anyhow::anyhow!("Cloud LLM returned empty choices"))
    }

    /// Check if the backend is configured and ready.
    pub fn is_ready(&self) -> bool {
        !self.config.api_key.is_empty()
    }
}

fn is_retryable_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string();
    msg.contains("status 429")
        || msg.contains("status 500")
        || msg.contains("status 502")
        || msg.contains("status 503")
        || msg.contains("status 504")
        || msg.contains("timeout")
        || msg.contains("timed out")
}
