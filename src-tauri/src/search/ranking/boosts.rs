//! Deterministic exact-match boosts applied after Reciprocal Rank Fusion.

use super::rrf::FusedHit;
use std::collections::HashMap;

/// Applies small additive bonuses to items containing exact phrases, terms, or entity references.
pub fn apply_exact_match_boosts(
    fused: &mut [FusedHit],
    query_text: &str,
    question_texts: &HashMap<i64, String>,
) {
    let lower_query = query_text.trim().to_lowercase();
    if lower_query.is_empty() {
        return;
    }

    for hit in fused.iter_mut() {
        if let Some(text) = question_texts.get(&hit.search_id) {
            let lower_text = text.to_lowercase();

            // A bounded exact phrase is decisive. The bonus is deliberately
            // larger than the maximum two-list RRF score, guaranteeing that
            // literal matches precede merely related semantic neighbours.
            if contains_bounded_phrase(&lower_text, &lower_query) {
                hit.score += 0.050;
            }

            // 2. Exact word boundary match for legal/institutional terms
            let is_article_or_act = lower_query.starts_with("article ")
                || lower_query.ends_with(" act")
                || lower_query.ends_with(" amendment");

            if is_article_or_act && contains_bounded_phrase(&lower_text, &lower_query) {
                hit.score += 0.010;
            }
        }
    }

    // Re-sort after applying boosts
    fused.sort_unstable_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.search_id.cmp(&b.search_id))
    });
}

fn contains_bounded_phrase(text: &str, phrase: &str) -> bool {
    text.match_indices(phrase).any(|(start, matched)| {
        let before_is_word = text[..start]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric);
        let after = start + matched.len();
        let after_is_word = text[after..]
            .chars()
            .next()
            .is_some_and(char::is_alphanumeric);
        !before_is_word && !after_is_word
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_phrase_boost() {
        let mut hits = vec![
            FusedHit {
                search_id: 1,
                question_id: "q1".to_string(),
                score: 0.030,
                lexical_rank: Some(2),
                semantic_rank: Some(1),
            },
            FusedHit {
                search_id: 2,
                question_id: "q2".to_string(),
                score: 0.031,
                lexical_rank: Some(1),
                semantic_rank: Some(2),
            },
        ];

        let mut texts = HashMap::new();
        texts.insert(1, "Question specifically mentioning Article 32".to_string());
        texts.insert(2, "General constitutional remedies question".to_string());

        apply_exact_match_boosts(&mut hits, "Article 32", &texts);

        // Hit 1 gets the decisive phrase/legal boost and overtakes Hit 2.
        assert_eq!(hits[0].search_id, 1);
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn exact_match_requires_word_boundaries() {
        assert!(contains_bounded_phrase("A river basin", "river"));
        assert!(!contains_bounded_phrase("A driver licence", "river"));
    }
}
