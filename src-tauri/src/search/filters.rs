//! Search filters for scoping queries by section, stage, paper, year, bank, tag.

use std::collections::HashSet;
use std::sync::Arc;

/// Scoping filters applied during both FTS and vector retrieval.
#[derive(Debug, Default, Clone)]
pub struct SearchFilter {
    pub sections: Vec<String>,
    pub stages: Vec<String>,
    pub papers: Vec<String>,
    pub years: Option<(u16, u16)>,
    pub banks: Vec<String>,
    pub tags: Vec<String>,
    /// Resolved once from SQLite for semantic scanning. This avoids a SQL
    /// lookup per vector while still applying filters inside the mmap scan.
    pub(crate) allowed_search_ids: Option<Arc<HashSet<u64>>>,
}

impl SearchFilter {
    pub fn has_constraints(&self) -> bool {
        !self.sections.is_empty()
            || !self.stages.is_empty()
            || !self.papers.is_empty()
            || self.years.is_some()
            || !self.banks.is_empty()
            || !self.tags.is_empty()
    }

    pub fn allows_search_id(&self, search_id: u64) -> bool {
        self.allowed_search_ids
            .as_ref()
            .map_or(true, |allowed| allowed.contains(&search_id))
    }
}
