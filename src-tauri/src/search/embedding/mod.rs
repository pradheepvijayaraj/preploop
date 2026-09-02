//! Embedding engine abstraction.
//!
//! All semantic code depends on [`EmbeddingEngine`], never on llama.cpp directly.

pub mod engine;
pub mod llama_cpp;
