//! Reciprocal Rank Fusion (RRF) for combining lexical and semantic retrieval.

use std::collections::HashMap;

use crate::search::lexical::fts::LexicalHit;
use crate::search::vector::traits::VectorHit;

/// Search candidate counts and fusion tuning parameters.
#[derive(Debug, Clone)]
pub struct SearchTuning {
    /// Maximum lexical rank allowed to influence the core fused ordering.
    pub lexical_fusion_window: usize,
    /// Maximum semantic rank allowed to influence the core fused ordering.
    pub semantic_fusion_window: usize,
    /// Maximum number of final hydrated results to return.
    pub final_limit: usize,
    /// RRF smoothing constant (default: 60.0).
    pub rrf_k: f32,
    /// Relative weight for lexical rank scores.
    pub lexical_weight: f32,
    /// Relative weight for semantic rank scores.
    pub semantic_weight: f32,
    /// Minimum Granite cosine score when lexical retrieval also has evidence.
    pub semantic_floor_with_lexical: f32,
    /// Stricter minimum for semantic-only results (out-of-domain rejection).
    pub semantic_floor_without_lexical: f32,
    /// Maximum cosine distance retained for a one-term semantic query.
    pub semantic_single_term_margin: f32,
    /// Tighter semantic-only band for short queries with lexical evidence.
    pub semantic_with_lexical_margin: f32,
    /// Recall band for three-term queries with lexical evidence.
    pub semantic_with_lexical_multi_margin: f32,
    /// Recall band for descriptive queries with lexical evidence.
    pub semantic_with_lexical_descriptive_margin: f32,
    /// Maximum cosine distance retained for a two/three-term semantic query.
    pub semantic_multi_term_margin: f32,
    /// Wider margin for descriptive queries whose relevant paraphrases vary.
    pub semantic_descriptive_margin: f32,
    /// Inner cosine band considered a strong semantic match.
    pub semantic_strong_margin: f32,
    /// Absolute lower bound applied after the query-level activation check.
    pub semantic_candidate_floor: f32,
}

impl Default for SearchTuning {
    fn default() -> Self {
        Self {
            lexical_fusion_window: 300,
            semantic_fusion_window: 300,
            final_limit: 300,
            rrf_k: 60.0,
            lexical_weight: 1.0,
            semantic_weight: 1.4,
            semantic_floor_with_lexical: 0.82,
            semantic_floor_without_lexical: 0.85,
            semantic_single_term_margin: 0.08,
            semantic_with_lexical_margin: 0.05,
            semantic_with_lexical_multi_margin: 0.07,
            semantic_with_lexical_descriptive_margin: 0.09,
            semantic_multi_term_margin: 0.10,
            semantic_descriptive_margin: 0.10,
            semantic_strong_margin: 0.04,
            semantic_candidate_floor: 0.75,
        }
    }
}

/// A fused search hit combining lexical and semantic signals.
#[derive(Debug, Clone, PartialEq)]
pub struct FusedHit {
    /// SQLite row ID in `search_documents`.
    pub search_id: i64,
    /// Canonical question UUID.
    pub question_id: String,
    /// Fused score from RRF and post-boosts.
    pub score: f32,
    /// 1-based rank in lexical search (None if not in lexical top candidates).
    pub lexical_rank: Option<usize>,
    /// 1-based rank in semantic search (None if not in semantic top candidates).
    pub semantic_rank: Option<usize>,
}

/// Fuses lexical hits and semantic hits into a unified ranked list using Reciprocal Rank Fusion.
pub fn reciprocal_rank_fusion(
    lexical_hits: &[LexicalHit],
    semantic_hits: &[VectorHit],
    question_id_lookup: impl Fn(i64) -> Option<String>,
    tuning: &SearchTuning,
) -> Vec<FusedHit> {
    let mut scores: HashMap<i64, f32> = HashMap::new();
    let mut lex_ranks: HashMap<i64, usize> = HashMap::new();
    let mut sem_ranks: HashMap<i64, usize> = HashMap::new();

    // 1. Lexical reciprocal rank scores: w_l / (k + r_l)
    for (i, hit) in lexical_hits.iter().enumerate() {
        let rank = i + 1;
        lex_ranks.insert(hit.search_id, rank);
        let rrf_score = tuning.lexical_weight / (tuning.rrf_k + rank as f32);
        *scores.entry(hit.search_id).or_insert(0.0) += rrf_score;
    }

    // 2. Semantic reciprocal rank scores: w_s / (k + r_s)
    for (i, hit) in semantic_hits.iter().enumerate() {
        let rank = i + 1;
        let search_id = hit.search_id as i64;
        sem_ranks.insert(search_id, rank);
        let rrf_score = tuning.semantic_weight / (tuning.rrf_k + rank as f32);
        *scores.entry(search_id).or_insert(0.0) += rrf_score;
    }

    // 3. Build fused items
    let mut fused: Vec<FusedHit> = scores
        .into_iter()
        .filter_map(|(search_id, score)| {
            let question_id = question_id_lookup(search_id)?;
            Some(FusedHit {
                search_id,
                question_id,
                score,
                lexical_rank: lex_ranks.get(&search_id).copied(),
                semantic_rank: sem_ranks.get(&search_id).copied(),
            })
        })
        .collect();

    // Sort descending by fused score
    fused.sort_unstable_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.search_id.cmp(&b.search_id))
    });

    fused
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rrf_scoring_and_ranking() {
        let lexical = vec![
            LexicalHit {
                search_id: 1,
                question_id: "q1".to_string(),
                score: 10.0,
                relaxed: false,
            },
            LexicalHit {
                search_id: 2,
                question_id: "q2".to_string(),
                score: 8.0,
                relaxed: false,
            },
        ];
        let semantic = vec![
            VectorHit {
                search_id: 2,
                score: 0.95,
            },
            VectorHit {
                search_id: 3,
                score: 0.90,
            },
        ];

        let lookup = |id: i64| Some(format!("q{id}"));
        let tuning = SearchTuning::default();

        let fused = reciprocal_rank_fusion(&lexical, &semantic, lookup, &tuning);

        // q2 appears in both and stays first. With the development-tuned
        // semantic weight, a semantic-only q3 outranks lexical-only q1.
        assert_eq!(fused[0].search_id, 2);
        assert_eq!(fused[1].search_id, 3);
        assert_eq!(fused[2].search_id, 1);
    }
}
