//! Per-query diagnostic fields exposed only by development builds.

/// Latency breakdown for a single search request (all times in milliseconds).
#[derive(Debug, Default)]
pub struct SearchMetrics {
    pub query_parse_ms: f64,
    pub fts_ms: f64,
    pub embedding_ms: f64,
    pub vector_ms: f64,
    pub fusion_ms: f64,
    pub hydrate_ms: f64,
    pub total_ms: f64,

    pub fts_candidate_count: usize,
    pub semantic_candidate_count: usize,
    pub fusion_candidate_count: usize,

    pub semantic_model_loaded: bool,
    pub vector_generation: u32,
}
