//! SearchRequest — input to SearchService. (Phase 5 — stub)

use crate::search::filters::SearchFilter;

#[derive(Clone)]
pub struct SearchOptions {
    pub semantic: bool,
    pub cancellation: crate::search::control::SearchCancellation,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            semantic: true,
            cancellation: Default::default(),
        }
    }
}

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
