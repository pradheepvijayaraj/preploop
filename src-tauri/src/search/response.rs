//! Search response returned by the hybrid retrieval service.

#[cfg(debug_assertions)]
use crate::search::metrics::SearchMetrics;

/// A single result entry.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub search_id: i64,
    pub question_id: String,
    pub score: f32,
    pub match_strength: MatchStrength,
    pub lexical_match: bool,
    pub semantic_match: bool,
}

/// User-facing confidence tier derived from independent retrieval signals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchStrength {
    Strong,
    Related,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SemanticStatus {
    Pending,
    Available,
    Unavailable,
    #[default]
    NotRequested,
}

/// Full response from a search request.
#[derive(Debug, Default)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    /// The lexical interpretation actually supported by documents in this scope.
    pub interpreted_query: Option<crate::search::lexical::query_builder::CompiledFtsQuery>,
    pub semantic_status: SemanticStatus,
    pub spelling_alternatives: Vec<String>,
    #[cfg(debug_assertions)]
    pub metrics: SearchMetrics,
}
