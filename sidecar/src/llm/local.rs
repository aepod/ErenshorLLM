use anyhow::{bail, Context, Result};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel, Special};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;
use std::num::NonZeroU32;
use std::path::Path;
use tracing::{debug, info, warn};

use crate::config::LocalLlmConfig;

/// Local LLM backend using llama.cpp (via llama-cpp-2 bindings) for GGUF
/// quantized model inference. Supports CPU, CUDA, and Vulkan backends.
///
/// Context is created per-generate call because `LlamaContext` borrows the model.
/// For a game mod where LLM calls are seconds apart, this overhead is negligible.
pub struct LocalBackend {
    backend: LlamaBackend,
    model: LlamaModel,
    config: LocalLlmConfig,
}

// Safety: LlamaBackend and LlamaModel are thread-safe via internal llama.cpp mutexes.
// generate() creates its own context per call, so no shared mutable state.
unsafe impl Send for LocalBackend {}
unsafe impl Sync for LocalBackend {}

impl LocalBackend {
    /// Load a GGUF quantized model via llama.cpp.
    /// Tokenizer is embedded in the GGUF file -- no separate tokenizer needed.
    /// GPU backend (CUDA/Vulkan) is selected at compile time via feature flags.
    pub fn load(model_path: &Path, config: &LocalLlmConfig) -> Result<Self> {
        info!("Loading GGUF model from: {}", model_path.display());

        if !model_path.exists() {
            bail!("GGUF model file not found: {}", model_path.display());
        }

        // Initialize llama.cpp backend
        let backend = LlamaBackend::init()
            .with_context(|| "Failed to initialize llama.cpp backend")?;

        // Configure model params with GPU offloading.
        // llama.cpp auto-detects available GPU backend (CUDA > Vulkan > CPU)
        // when compiled with the appropriate feature flags.
        // gpu_layers=999 means "offload all layers"; llama.cpp silently clamps
        // to the actual layer count if GPU is available, or ignores if CPU-only.
        let model_params = LlamaModelParams::default()
            .with_n_gpu_layers(config.gpu_layers);

        info!("GPU layers requested: {} (0=CPU, 999=all)", config.gpu_layers);

        // Load the GGUF model
        info!("Loading GGUF model (this may take a moment)...");
        let model = LlamaModel::load_from_file(&backend, model_path, &model_params)
            .map_err(|e| anyhow::anyhow!("Failed to load GGUF model: {}", e))?;

        info!(
            "GGUF model loaded: {} vocab, {} context max",
            model.n_vocab(),
            model.n_ctx_train(),
        );

        Ok(Self {
            backend,
            model,
            config: config.clone(),
        })
    }

    /// Generate text from a prompt using the loaded model.
    /// Creates a fresh inference context per call to avoid lifetime issues.
    pub fn generate(&self, prompt: &str, max_tokens: usize, temperature: f32) -> Result<String> {
        // Tokenize the prompt using the model's built-in tokenizer
        let tokens = self
            .model
            .str_to_token(prompt, AddBos::Always)
            .map_err(|e| anyhow::anyhow!("Tokenization failed: {}", e))?;

        if tokens.is_empty() {
            bail!("Empty tokenization result");
        }

        // Enforce context size limit
        let context_size = self.config.context_size.min(self.model.n_ctx_train() as usize);
        let input_len = tokens.len();
        if input_len >= context_size {
            warn!(
                "Input ({} tokens) exceeds context size ({}), truncating",
                input_len, context_size
            );
        }
        let effective_tokens = if input_len > context_size.saturating_sub(max_tokens) {
            let start = input_len.saturating_sub(context_size.saturating_sub(max_tokens));
            &tokens[start..]
        } else {
            &tokens
        };

        info!(
            "LLM generate: {} input tokens, max {} output, temp {}, ctx_size {}",
            effective_tokens.len(),
            max_tokens,
            temperature,
            context_size
        );

        // Create a fresh context for this generation
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(context_size as u32));

        let mut ctx = self
            .model
            .new_context(&self.backend, ctx_params)
            .map_err(|e| anyhow::anyhow!("Failed to create inference context: {}", e))?;

        // Create batch and add prompt tokens
        let mut batch = LlamaBatch::new(context_size, 1);

        let last_idx = (effective_tokens.len() - 1) as i32;
        for (i, &token) in effective_tokens.iter().enumerate() {
            let is_last = i as i32 == last_idx;
            batch
                .add(token, i as i32, &[0], is_last)
                .map_err(|e| anyhow::anyhow!("Failed to add token to batch: {}", e))?;
        }

        // Decode prompt tokens (prefill)
        ctx.decode(&mut batch)
            .map_err(|e| anyhow::anyhow!("Prompt decoding failed: {}", e))?;

        // Set up sampler chain: top-k -> temperature -> distribution sampling
        let seed = rand::random::<u32>();
        let mut sampler = LlamaSampler::chain_simple(vec![
            LlamaSampler::top_k(40),
            LlamaSampler::temp(temperature),
            LlamaSampler::dist(seed),
        ]);

        // Get the EOS token for stop detection
        let eos_token = self.model.token_eos();

        let mut generated_tokens: Vec<LlamaToken> = Vec::with_capacity(max_tokens);
        let mut n_cur = effective_tokens.len() as i32;

        // Autoregressive generation loop
        for i in 0..max_tokens {
            let new_token = sampler.sample(&ctx, -1);

            if new_token == eos_token {
                debug!("EOS at token {}", i);
                break;
            }

            generated_tokens.push(new_token);

            batch.clear();
            batch
                .add(new_token, n_cur, &[0], true)
                .map_err(|e| anyhow::anyhow!("Failed to add generated token: {}", e))?;
            n_cur += 1;

            ctx.decode(&mut batch)
                .map_err(|e| anyhow::anyhow!("Token decoding failed: {}", e))?;
        }

        debug!("Generated {} tokens", generated_tokens.len());

        // Decode generated tokens back to text
        let text = self.decode_tokens(&generated_tokens)?;

        Ok(text)
    }

    /// Decode a sequence of tokens back to a string using the model's vocabulary.
    fn decode_tokens(&self, tokens: &[LlamaToken]) -> Result<String> {
        let mut text = String::new();
        for &token in tokens {
            let piece = self
                .model
                .token_to_str(token, Special::Tokenize)
                .map_err(|e| anyhow::anyhow!("Token decoding failed: {}", e))?;
            text.push_str(&piece);
        }
        Ok(text)
    }

    /// Check if the model is loaded and ready.
    pub fn is_ready(&self) -> bool {
        true // If constructed, the model is loaded
    }

    /// Get the model file name for reporting.
    pub fn model_name(config: &LocalLlmConfig) -> String {
        Path::new(&config.model_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string()
    }
}
