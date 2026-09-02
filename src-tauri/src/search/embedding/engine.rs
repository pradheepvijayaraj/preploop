//! [`EmbeddingEngine`] — the single interface the rest of the search stack
//! depends on for dense vector encoding.
//!
//! The current implementation is [`super::llama_cpp::LlamaCppEmbeddingEngine`]
//! (Granite R2, GGUF Q8_0, CLS pooling, CPU).
//! Nothing outside this module tree should import `llama_cpp` directly.

/// A 384-dimensional L2-normalised float32 embedding vector.
///
/// The concrete length is enforced by [`EmbeddingEngine::dimensions`].
/// Callers must not hard-code 384; always query the engine.
pub type Embedding = Vec<f32>;

/// Platform-neutral interface for dense text encoding.
///
/// Implementations must be `Send + Sync` so they can be held behind an
/// `Arc<dyn EmbeddingEngine>` in [`crate::search::service::SearchService`].
///
/// **Design rule**: no Metal / CUDA / NPU surface appears in this trait.
/// Platform acceleration is an internal detail of each implementation.
pub trait EmbeddingEngine: Send + Sync {
    /// Number of dimensions in each returned embedding.
    ///
    /// For Granite R2 Q8_0 this is always 384.
    fn dimensions(&self) -> usize;

    /// Produce a single embedding for a user query.
    ///
    /// Must never be called with an empty string — the caller (`SearchService`)
    /// is responsible for early-returning on blank input before reaching here.
    ///
    /// Returns an L2-normalised vector of length `self.dimensions()`.
    fn embed_query(&self, text: &str) -> Result<Embedding, EmbeddingError>;

    /// Produce embeddings for a batch of document texts.
    ///
    /// Returns one `Embedding` per input text, in the same order.
    /// Batch size is chosen by the caller; the engine may process them
    /// sequentially or in parallel internally.
    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Embedding>, EmbeddingError>;
}

/// Errors produced by [`EmbeddingEngine`] implementations.
#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    /// The underlying model failed to load (path missing, GGUF corrupt, …).
    #[error("model load failed: {0}")]
    ModelLoad(String),

    /// Tokenisation or inference failed for a specific input.
    #[error("inference failed: {0}")]
    Inference(String),

    /// The returned vector has the wrong number of dimensions.
    #[error("dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },

    /// The engine was called with an empty string.
    #[error("embed_query called with empty text")]
    EmptyInput,
}

/// Compute the L2 norm of a float slice.
#[inline]
fn l2_norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

/// In-place L2 normalisation. No-op if the norm is zero (zero vector).
pub fn l2_normalize(v: &mut [f32]) {
    let norm = l2_norm(v);
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l2_normalize_unit_vector() {
        let mut v = vec![3.0_f32, 4.0];
        l2_normalize(&mut v);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-6,
            "expected unit vector, got norm={norm}"
        );
    }

    #[test]
    fn l2_normalize_zero_vector_is_noop() {
        let mut v = vec![0.0_f32, 0.0, 0.0];
        l2_normalize(&mut v);
        assert_eq!(v, [0.0, 0.0, 0.0]);
    }
}
