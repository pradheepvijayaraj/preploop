//! SearchRequest — input to SearchService. (Phase 5 — stub)

use crate::search::filters::SearchFilter;

/// A normalised search request from the UI.
#[derive(Debug, Clone)]
pub struct SearchRequest {
    /// Raw query text from the user (may be empty).
    pub query: String,
    /// Structured filters to scope retrieval.
    pub filters: SearchFilter,
    /// Maximum number of results to hydrate and return.
    pub limit: usize,
}

impl SearchRequest {
    pub fn is_empty(&self) -> bool {
        self.query.trim().is_empty()
    }
}
