//! ONNX-based sentence embedding engine.
//!
//! Uses all-MiniLM-L6-v2 (384-dim) with mean pooling + L2 normalization.
//! Thread-safe: the ort::Session is wrapped in a Mutex for safe concurrent access.

use anyhow::{Context, Result};
use ndarray::{Array2, ArrayView3, Ix3};
use ort::session::Session;
use ort::value::Tensor;
use parking_lot::Mutex;
use std::path::Path;
use std::sync::Arc;
use tokenizers::Tokenizer;
use tracing::{debug, info};

/// The embedding engine wrapping ONNX inference + tokenization.
///
/// The ort v2 `Session::run()` requires `&mut self`, so we wrap it in a Mutex.
/// For our use case (sequential embedding calls during index build, and
/// low-concurrency HTTP requests), mutex contention is negligible.
pub struct EmbeddingEngine {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
    dimensions: usize,
}

impl EmbeddingEngine {
    /// Load the ONNX model and tokenizer from disk.
    ///
    /// `model_path`: path to the `.onnx` model file
    /// `tokenizer_path`: path to the HuggingFace `tokenizer.json`
    /// `threads`: number of intra-op threads for ONNX Runtime
    pub fn new(model_path: &Path, tokenizer_path: &Path, threads: usize) -> Result<Arc<Self>> {
        info!(
            "Loading embedding model from {:?} with {} threads",
            model_path, threads
        );

        let session = Session::builder()?
            .with_intra_threads(threads)?
            .commit_from_file(model_path)
            .context("Failed to load ONNX model")?;

        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {}", e))?;

        info!(
            "Embedding model loaded: {} inputs, {} outputs",
            session.inputs().len(),
            session.outputs().len()
        );

        Ok(Arc::new(Self {
            session: Mutex::new(session),
            tokenizer,
            dimensions: 384,
        }))
    }

    /// Get the output dimensionality (384 for all-MiniLM-L6-v2).
    pub fn dimensions(&self) -> usize {
        self.dimensions
    }

    /// Embed a single text string into a 384-dim f32 vector.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("Tokenization failed: {}", e))?;

        let input_ids = encoding.get_ids();
        let attention_mask = encoding.get_attention_mask();
        let token_type_ids = encoding.get_type_ids();
        let seq_len = input_ids.len();

        debug!("Tokenized '{}' -> {} tokens", text, seq_len);

        // Build input tensors as [1, seq_len]
        let ids_array = Array2::from_shape_vec(
            (1, seq_len),
            input_ids.iter().map(|&x| x as i64).collect(),
        )?;
        let mask_array = Array2::from_shape_vec(
            (1, seq_len),
            attention_mask.iter().map(|&x| x as i64).collect(),
        )?;
        let type_array = Array2::from_shape_vec(
            (1, seq_len),
            token_type_ids.iter().map(|&x| x as i64).collect(),
        )?;

        // Convert ndarray to ort Tensor values
        let ids_tensor = Tensor::from_array(ids_array)?;
        let mask_tensor = Tensor::from_array(mask_array)?;
        let type_tensor = Tensor::from_array(type_array)?;

        // Run inference (requires &mut session)
        let mut session = self.session.lock();
        let outputs = session.run(ort::inputs![
            "input_ids" => ids_tensor,
            "attention_mask" => mask_tensor,
            "token_type_ids" => type_tensor,
        ])?;

        // Extract token embeddings: shape [1, seq_len, 384]
        let token_dyn = outputs[0]
            .try_extract_array::<f32>()
            .context("Failed to extract output tensor")?;

        let token_view: ArrayView3<'_, f32> = token_dyn
            .into_dimensionality::<Ix3>()
            .context("Expected 3D output tensor [batch, seq_len, dims]")?;

        // Mean pooling weighted by attention mask
        let embedding = Self::mean_pool(&token_view, attention_mask, self.dimensions);

        // L2 normalize
        let normalized = Self::l2_normalize(&embedding);

        Ok(normalized)
    }

    /// Embed a batch of text strings.
    ///
    /// For simplicity, processes each independently (no batched ONNX inference).
    /// This is fine for our use case (index building, small batches).
    pub fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        texts.iter().map(|text| self.embed(text)).collect()
    }

    /// Mean pooling over token embeddings, weighted by attention mask.
    ///
    /// For each dimension d:
    ///   pooled[d] = sum(token_embeddings[t][d] * mask[t]) / sum(mask[t])
    fn mean_pool(
        token_embeddings: &ArrayView3<'_, f32>,
        attention_mask: &[u32],
        dims: usize,
    ) -> Vec<f32> {
        let seq_len = attention_mask.len();
        let mut pooled = vec![0.0f32; dims];
        let mut mask_sum = 0.0f32;

        for t in 0..seq_len {
            let mask_val = attention_mask[t] as f32;
            mask_sum += mask_val;
            for d in 0..dims {
                // token_embeddings shape is [1, seq_len, dims]
                pooled[d] += token_embeddings[[0, t, d]] * mask_val;
            }
        }

        if mask_sum > 0.0 {
            for d in 0..dims {
                pooled[d] /= mask_sum;
            }
        }

        pooled
    }

    /// L2 normalize a vector (returns the normalized vector).
    fn l2_normalize(vec: &[f32]) -> Vec<f32> {
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            vec.iter().map(|x| x / norm).collect()
        } else {
            vec.to_vec()
        }
    }

    /// Compute cosine similarity between two normalized vectors.
    ///
    /// Since our vectors are L2-normalized, cosine similarity = dot product.
    pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
    }

    /// Async-safe embedding: runs ONNX inference on a blocking thread to
    /// avoid segfaults when the dynamically-loaded onnxruntime.dll has
    /// thread-affinity issues with cross-compiled mingw + tokio worker threads.
    pub async fn embed_async(self: &Arc<Self>, text: String) -> Result<Vec<f32>> {
        let engine = Arc::clone(self);
        tokio::task::spawn_blocking(move || engine.embed(&text))
            .await
            .map_err(|e| anyhow::anyhow!("Embedding task panicked: {}", e))?
    }

    /// Get the number of tokens for a text (useful for usage reporting).
    pub fn token_count(&self, text: &str) -> usize {
        self.tokenizer
            .encode(text, true)
            .map(|enc| enc.get_ids().len())
            .unwrap_or(0)
    }
}
