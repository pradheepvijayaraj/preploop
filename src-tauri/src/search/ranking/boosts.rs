//! Deterministic exact-match boosts applied after Reciprocal Rank Fusion.

use super::rrf::FusedHit;
use std::collections::{HashMap, HashSet};

/// Applies small additive bonuses to items containing exact phrases, terms, or entity references.
pub fn apply_exact_match_boosts(
    fused: &mut [FusedHit],
    query: &crate::search::lexical::query_builder::CompiledFtsQuery,
    question_texts: &HashMap<i64, String>,
) -> HashSet<i64> {
    let mut literal_matches = HashSet::new();
    let query_words = query.word_patterns();
    for hit in fused.iter_mut() {
        if let Some(text) = question_texts.get(&hit.search_id) {
            let words = crate::search::lexical::query_builder::normalized_words(text);
            // Use the same token boundaries as retrieval, so punctuation and
            // repeated whitespace do not discard a literal phrase's ranking.
            // A phrase completed while typing also outranks scattered terms.
            let mut bonus = 0.0_f32;
            for window in words.windows(query_words.len()) {
                if window
                    .iter()
                    .map(String::as_str)
                    .eq(query_words.iter().map(|word| word.text.as_str()))
                {
                    bonus = 0.060;
                    break;
                }
                if window.iter().zip(&query_words).all(|(word, pattern)| {
                    word == &pattern.text || (pattern.prefix && word.starts_with(&pattern.text))
                }) {
                    bonus = bonus.max(0.050);
                }
            }
            hit.score += bonus;
            if bonus > 0.0 {
                literal_matches.insert(hit.search_id);
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
    literal_matches
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

        apply_exact_match_boosts(
            &mut hits,
            &crate::search::lexical::query_builder::FtsQueryBuilder::build("Article 32").unwrap(),
            &texts,
        );

        // Hit 1 gets the decisive phrase/legal boost and overtakes Hit 2.
        assert_eq!(hits[0].search_id, 1);
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn typing_and_punctuation_keep_phrase_ahead_of_scattered_matches() {
        for query in [
            "silver n",
            "silver no",
            "silver/notice",
            "silver  notice",
            "“silver notice”",
        ] {
            let mut hits = vec![
                FusedHit {
                    search_id: 1,
                    question_id: "phrase".into(),
                    score: 0.01,
                    lexical_rank: Some(2),
                    semantic_rank: None,
                },
                FusedHit {
                    search_id: 2,
                    question_id: "scattered".into(),
                    score: 0.03,
                    lexical_rank: Some(1),
                    semantic_rank: Some(1),
                },
            ];
            let texts = HashMap::from([
                (1, "Silver Notice".into()),
                (2, "Silver deposits. Notice the metal.".into()),
            ]);
            apply_exact_match_boosts(
                &mut hits,
                &crate::search::lexical::query_builder::FtsQueryBuilder::build(query).unwrap(),
                &texts,
            );
            assert_eq!(hits[0].question_id, "phrase", "{query}");
        }
        let mut hits = vec![FusedHit {
            search_id: 1,
            question_id: "q".into(),
            score: 0.01,
            lexical_rank: None,
            semantic_rank: Some(1),
        }];
        let texts = HashMap::from([(1, "A driver licence under Article 200".into())]);
        for query in ["river", "Article 20"] {
            apply_exact_match_boosts(
                &mut hits,
                &crate::search::lexical::query_builder::FtsQueryBuilder::build(query).unwrap(),
                &texts,
            );
            assert_eq!(hits[0].score, 0.01);
        }
    }
}
