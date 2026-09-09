//! [`LlamaCppEmbeddingEngine`] — Granite R2 (GGUF Q8_0, 384d, CLS pooling)
//! running on llama.cpp via the `llama-cpp-2` crate.
//!
//! # Lifecycle
//!
//! The model is **lazy-loaded**: loading does not happen at `new()` but on the
//! first call to [`EmbeddingEngine::embed_query`] or
//! [`EmbeddingEngine::embed_documents`].
//! After the first load the model lives for the lifetime of this struct.
//!
//! ```text
//! PrepLoop starts          → model unloaded (0 extra RSS)
//! First semantic search    → model loaded once (~50 MB RSS)
//! Every subsequent search  → reuses loaded model (3–4 ms p50 on CPU)
//! ```
//!
//! # Thread safety
//!
//! `LlamaCppEmbeddingEngine` is `Send + Sync`. Concurrent callers share a
//! single `Mutex<Option<LoadedModel>>`. Superseded requests stop waiting and
//! skip inference. The published binding cannot interrupt an active decode;
//! that decode finishes, then its cancelled result is discarded.
//!
//! # CPU baseline
//!
//! This implementation uses zero GPU layers (`n_gpu_layers = 0`).
//! Platform-specific acceleration can be added here in the future without
//! touching `SearchService`, FTS, the vector index, or the ranking code.

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::LlamaModel;

use super::engine::{l2_normalize, Embedding, EmbeddingEngine, EmbeddingError};
use crate::search::control::SearchCancellation;

/// The expected embedding dimension for Granite R2 Q8_0.
const GRANITE_R2_DIMS: usize = 384;

/// Maximum context window used for embedding (must be ≥ longest document).
/// Granite R2's training context is 512 tokens; 1024 gives headroom.
const CTX_SIZE: u32 = 1024;

/// Number of CPU threads used for embedding inference.
/// Defaults to the number of physical cores, capped at 8.
fn default_n_threads() -> i32 {
    let phys = num_cpus::get_physical() as i32;
    phys.clamp(1, 8)
}

// ---------------------------------------------------------------------------
// Global llama backend (must be initialised once per process)
// ---------------------------------------------------------------------------

static LLAMA_BACKEND: Mutex<Option<Arc<LlamaBackend>>> = Mutex::new(None);

fn get_or_init_backend() -> Result<Arc<LlamaBackend>, EmbeddingError> {
    let mut guard = LLAMA_BACKEND
        .lock()
        .map_err(|_| EmbeddingError::ModelLoad("backend mutex poisoned".to_string()))?;
    if let Some(backend) = guard.as_ref() {
        return Ok(Arc::clone(backend));
    }
    let mut backend = LlamaBackend::init()
        .map_err(|e| EmbeddingError::ModelLoad(format!("llama backend init failed: {e}")))?;
    if !cfg!(debug_assertions) {
        backend.void_logs();
    }
    let arc_backend = Arc::new(backend);
    *guard = Some(Arc::clone(&arc_backend));
    Ok(arc_backend)
}

// ---------------------------------------------------------------------------
// Internal loaded state
// ---------------------------------------------------------------------------

struct LoadedModel {
    backend: Arc<LlamaBackend>,
    model: LlamaModel,
    n_threads: i32,
    gpu_offload: bool,
}

// ---------------------------------------------------------------------------
// Public engine
// ---------------------------------------------------------------------------

/// Persistent, lazy-loading llama.cpp embedding engine.
///
/// Construct with [`LlamaCppEmbeddingEngine::new`], then store behind an
/// `Arc<dyn EmbeddingEngine>` in `SearchService`.
pub struct LlamaCppEmbeddingEngine {
    gguf_path: PathBuf,
    n_threads: i32,
    n_gpu_layers: u32,
    inner: Mutex<Option<LoadedModel>>,
}

impl LlamaCppEmbeddingEngine {
    /// Create an engine that will load the GGUF at `gguf_path` on first use.
    ///
    /// # Errors
    ///
    /// Returns an error only if the path does not exist. Actual model loading
    /// is deferred to first inference.
    pub fn new(gguf_path: impl AsRef<Path>) -> Result<Self, EmbeddingError> {
        let path = gguf_path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(EmbeddingError::ModelLoad(format!(
                "GGUF not found: {}",
                path.display()
            )));
        }
        Ok(Self {
            gguf_path: path,
            n_threads: default_n_threads(),
            n_gpu_layers: 0,
            inner: Mutex::new(None),
        })
    }

    /// Override the number of CPU threads (useful in tests).
    pub fn with_n_threads(mut self, n: i32) -> Self {
        self.n_threads = n.max(1);
        self
    }

    /// Opt into GPU offload for offline tooling. The shipped application does
    /// not call this and always keeps the CPU-only default of zero layers.
    pub fn with_n_gpu_layers(mut self, layers: u32) -> Self {
        self.n_gpu_layers = layers;
        self
    }

    // ------------------------------------------------------------------
    // Internal: ensure model is loaded, then call f
    // ------------------------------------------------------------------

    fn with_model<F, R>(&self, f: F) -> Result<R, EmbeddingError>
    where
        F: FnOnce(&LoadedModel) -> Result<R, EmbeddingError>,
    {
        self.with_model_cancellable(&Default::default(), f)
    }

    fn with_model_cancellable<F, R>(
        &self,
        cancellation: &crate::search::control::SearchCancellation,
        f: F,
    ) -> Result<R, EmbeddingError>
    where
        F: FnOnce(&LoadedModel) -> Result<R, EmbeddingError>,
    {
        // SAFETY INVARIANT: every operation that can touch `LoadedModel`,
        // including model loading and inference, must stay inside this lock.
        // Do not expose `LoadedModel` or add an unlocked fast path.
        let mut guard = loop {
            cancellation.check().map_err(EmbeddingError::Inference)?;
            match self.inner.try_lock() {
                Ok(guard) => break guard,
                Err(std::sync::TryLockError::WouldBlock) => {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    return Err(EmbeddingError::Inference(
                        "embedding engine mutex poisoned".into(),
                    ));
                }
            }
        };

        cancellation.check().map_err(EmbeddingError::Inference)?;
        if guard.is_none() {
            #[cfg(debug_assertions)]
            eprintln!(
                "Loading embedding model {:?}; metadata={:?}",
                self.gguf_path,
                std::fs::metadata(&self.gguf_path)
            );
            // First call — load backend and model now.
            let backend = get_or_init_backend()?;

            let loading_cancellation = cancellation.clone();
            let model_params = LlamaModelParams::default()
                .with_n_gpu_layers(self.n_gpu_layers)
                .with_progress_callback(move |_| !loading_cancellation.is_cancelled());

            let model = LlamaModel::load_from_file(&backend, &self.gguf_path, &model_params)
                .map_err(|e| {
                    if cancellation.is_cancelled() {
                        EmbeddingError::Inference("Search cancelled".into())
                    } else {
                        EmbeddingError::ModelLoad(format!("{e}"))
                    }
                })?;
            #[cfg(debug_assertions)]
            eprintln!("Embedding model loaded successfully");

            *guard = Some(LoadedModel {
                backend,
                model,
                n_threads: self.n_threads,
                gpu_offload: self.n_gpu_layers > 0,
            });
        }

        cancellation.check().map_err(EmbeddingError::Inference)?;
        let result = f(guard.as_ref().unwrap())?;
        cancellation.check().map_err(EmbeddingError::Inference)?;
        Ok(result)
    }

    // ------------------------------------------------------------------
    // Core: tokenise + encode a batch of texts → embeddings
    // ------------------------------------------------------------------

    fn encode_texts(
        loaded: &LoadedModel,
        texts: &[&str],
        cancellation: &SearchCancellation,
    ) -> Result<Vec<Embedding>, EmbeddingError> {
        cancellation.check().map_err(EmbeddingError::Inference)?;
        let model = &loaded.model;
        let backend = &loaded.backend;

        let mut ctx = model
            .new_context(backend, embedding_context_params(loaded))
            .map_err(|e| EmbeddingError::Inference(format!("context init: {e}")))?;

        let mut embeddings: Vec<Embedding> = Vec::with_capacity(texts.len());

        // Process texts one at a time via seq_id to keep memory bounded.
        // For batch embedding (Phase 6 worker) we can extend this to
        // submit multiple sequences per llama_decode call.
        for text in texts {
            cancellation.check().map_err(EmbeddingError::Inference)?;
            // Tokenise (add BOS, no EOS for embedding models).
            let tokens = model
                .str_to_token(text, llama_cpp_2::model::AddBos::Always)
                .map_err(|e| EmbeddingError::Inference(format!("tokenise: {e}")))?;

            if tokens.is_empty() {
                return Err(EmbeddingError::EmptyInput);
            }

            // Truncate silently if over context window.
            let tokens: Vec<_> = tokens.into_iter().take(CTX_SIZE as usize - 1).collect();

            let mut batch = LlamaBatch::new(CTX_SIZE as usize, 1);
            let last_idx = tokens.len() - 1;
            for (i, token) in tokens.into_iter().enumerate() {
                batch
                    .add(token, i as i32, &[0], i == last_idx)
                    .map_err(|e| EmbeddingError::Inference(format!("batch add: {e}")))?;
            }

            ctx.clear_kv_cache();
            cancellation.check().map_err(EmbeddingError::Inference)?;
            let decoded = ctx.decode(&mut batch);
            // The published binding has no inference-abort hook. Let this
            // decode finish, then reject superseded work before reading its
            // output or starting another text. Retain the loaded model.
            cancellation.check().map_err(EmbeddingError::Inference)?;
            decoded.map_err(|e| EmbeddingError::Inference(format!("decode: {e}")))?;

            // CLS pooling — embeddings_seq_ith returns the pooled vector.
            let raw = ctx
                .embeddings_seq_ith(0)
                .map_err(|e| EmbeddingError::Inference(format!("embeddings_seq_ith: {e}")))?;

            let expected = GRANITE_R2_DIMS;
            if raw.len() != expected {
                return Err(EmbeddingError::DimensionMismatch {
                    expected,
                    actual: raw.len(),
                });
            }

            let mut v: Vec<f32> = raw.to_vec();
            l2_normalize(&mut v);
            embeddings.push(v);
        }

        Ok(embeddings)
    }
}

fn embedding_context_params(loaded: &LoadedModel) -> LlamaContextParams {
    LlamaContextParams::default()
        .with_n_ctx(Some(NonZeroU32::new(CTX_SIZE).unwrap()))
        .with_n_batch(CTX_SIZE)
        // Encoder models cannot split one sequence across micro-batches.
        .with_n_ubatch(CTX_SIZE)
        .with_embeddings(true)
        .with_n_threads(loaded.n_threads)
        .with_n_threads_batch(loaded.n_threads)
        // Offline tools may explicitly opt into GPU offload; interactive
        // search keeps both model layers and context operations on the CPU.
        .with_offload_kqv(loaded.gpu_offload)
        .with_op_offload(loaded.gpu_offload)
}

impl EmbeddingEngine for LlamaCppEmbeddingEngine {
    fn embed_query_cancellable(
        &self,
        text: &str,
        cancellation: &crate::search::control::SearchCancellation,
    ) -> Result<Embedding, EmbeddingError> {
        if text.trim().is_empty() {
            return Err(EmbeddingError::EmptyInput);
        }
        self.with_model_cancellable(cancellation, |loaded| {
            let mut embeddings = Self::encode_texts(loaded, &[text], cancellation)?;
            Ok(embeddings.remove(0))
        })
    }

    fn dimensions(&self) -> usize {
        GRANITE_R2_DIMS
    }

    fn embed_query(&self, text: &str) -> Result<Embedding, EmbeddingError> {
        self.embed_query_cancellable(text, &SearchCancellation::default())
    }

    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Embedding>, EmbeddingError> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        self.with_model(|loaded| Self::encode_texts(loaded, &refs, &SearchCancellation::default()))
    }
}

// SAFETY: llama.cpp's model/backend handles are not `Sync`. They are stored in
// `inner` and can only be created or accessed by `with_model_cancellable`
// (also used by `with_model`), which holds the mutex throughout inference and
// context cleanup. No reference to `LoadedModel` escapes
// that closure. If this access pattern changes, these impls must be revisited.
unsafe impl Send for LlamaCppEmbeddingEngine {}
unsafe impl Sync for LlamaCppEmbeddingEngine {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_inference_result_is_discarded_and_model_stays_usable() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("models/granite-r2-q8_0.gguf");
        let engine = LlamaCppEmbeddingEngine::new(path)
            .unwrap()
            .with_n_threads(1);
        let cancellation = SearchCancellation::default();
        let result = engine.with_model_cancellable(&cancellation, |loaded| {
            let embeddings = LlamaCppEmbeddingEngine::encode_texts(
                loaded,
                &["How does water conservation protect river ecosystems?"],
                &cancellation,
            )?;
            // Deterministically model cancellation as inference completes,
            // before its result can escape the engine. This does not assert
            // interruption of the native decode.
            cancellation.cancel();
            Ok(embeddings)
        });
        assert!(
            matches!(result, Err(EmbeddingError::Inference(message)) if message == "Search cancelled")
        );
        assert!(cancellation.is_cancelled());
        assert!(
            engine.inner.lock().unwrap().is_some(),
            "retain the loaded model after cancellation"
        );
        let next = engine.embed_query("River conservation").unwrap();
        assert_eq!(next.len(), GRANITE_R2_DIMS);
        assert!(next.iter().all(|value| value.is_finite()));
        let norm = next.iter().map(|value| value * value).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4);
    }

    #[test]
    fn superseded_waiter_returns_before_busy_model_is_released() {
        let engine = Arc::new(LlamaCppEmbeddingEngine {
            gguf_path: PathBuf::from("must-not-be-opened.gguf"),
            n_threads: 1,
            n_gpu_layers: 0,
            inner: Mutex::new(None),
        });
        let guard = engine.inner.lock().unwrap();
        let cancellation = SearchCancellation::default();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let waiting_engine = Arc::clone(&engine);
        let waiting_cancellation = cancellation.clone();
        let waiter = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = waiting_engine.embed_query_cancellable("water", &waiting_cancellation);
            result_tx.send(result).unwrap();
        });
        started_rx.recv().unwrap();
        cancellation.cancel();
        let result = result_rx.recv_timeout(std::time::Duration::from_secs(2));
        // Release the lock before asserting so a failure cannot strand a
        // worker. A successful result arrived while the model was still busy.
        drop(guard);
        waiter.join().unwrap();
        assert!(
            matches!(result, Ok(Err(EmbeddingError::Inference(message))) if message == "Search cancelled")
        );
        assert!(engine.inner.lock().unwrap().is_none());
    }

    #[test]
    fn cancelled_request_does_not_start_loading_or_inference() {
        let engine = LlamaCppEmbeddingEngine {
            gguf_path: PathBuf::from("must-not-be-opened.gguf"),
            n_threads: 1,
            n_gpu_layers: 0,
            inner: Mutex::new(None),
        };
        let cancellation = SearchCancellation::default();
        cancellation.cancel();
        assert!(
            matches!(engine.embed_query_cancellable("water", &cancellation),
            Err(EmbeddingError::Inference(message)) if message == "Search cancelled")
        );
        assert!(engine.inner.lock().unwrap().is_none());
    }

    #[test]
    fn engine_remains_send_and_sync_for_shared_search_services() {
        fn assert_send_and_sync<T: Send + Sync>() {}
        assert_send_and_sync::<LlamaCppEmbeddingEngine>();
    }

    /// Verify that constructing with a non-existent path fails eagerly.
    #[test]
    fn missing_gguf_returns_error() {
        let result = LlamaCppEmbeddingEngine::new("/tmp/no_such_model.gguf");
        assert!(
            matches!(result, Err(EmbeddingError::ModelLoad(_))),
            "expected ModelLoad error"
        );
    }
}
