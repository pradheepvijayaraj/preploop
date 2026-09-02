//! Vector search abstractions and hit representations.

use crate::search::filters::SearchFilter;

/// A candidate hit from dense vector retrieval.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorHit {
    /// SQLite row ID in `search_documents`.
    pub search_id: u64,
    /// Cosine similarity score in range [-1.0, 1.0].
    pub score: f32,
}

/// Abstract interface for dense vector retrieval indexes.
pub trait VectorSearch: Send + Sync {
    /// Retrieve the top `limit` items closest to the query vector.
    fn search(
        &self,
        query: &[f32],
        filters: &SearchFilter,
        limit: usize,
    ) -> Result<Vec<VectorHit>, String>;

    /// Number of active records in the index.
    fn count(&self) -> usize;

    /// Current index generation ID.
    fn generation(&self) -> u32;
}
