//! PrepLoop search subsystem.
//!
//! Architecture: SQLite FTS5 (lexical) + Granite R2 / llama.cpp (semantic)
//! + flat mmap vector index, fused with Reciprocal Rank Fusion.
//!
//! CPU-first. No platform-specific code in this module tree.

pub mod embedding;
pub mod filters;
pub mod indexing;
pub mod lexical;
pub mod metrics;
pub mod ranking;
pub mod request;
pub mod response;
pub mod service;
pub mod vector;
