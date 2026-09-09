//! Corroborate semantic queries with topic associations in the local corpus.
//!
//! A cosine score is not a relevance probability. Require independent evidence
//! before expanding beyond literal matches, without a query/topic allowlist.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use crate::search::filters::SearchFilter;
use crate::search::lexical::fts::LexicalHit;
use crate::search::lexical::query_builder::{is_relaxed_stop_word, CompiledFtsQuery};
use crate::search::vector::traits::VectorHit;

struct TopicDocument {
    terms: Vec<usize>,
    occurrences: Vec<TopicOccurrence>,
    main_tag: String,
}

struct TopicOccurrence {
    term: usize,
    field: u8,
    offset: u32,
}

/// Term IDs keep this reusable projection compact; text is stemmed by the same
/// SQLite tokenizer used for retrieval. The service cache owns its lifetime.
pub(crate) struct TopicEvidence {
    terms: Vec<String>,
    documents: HashMap<u64, TopicDocument>,
}

impl TopicEvidence {
    pub(crate) fn load(conn: &Connection) -> Result<Self, String> {
        let mut documents = HashMap::new();
        let mut statement = conn
            .prepare("SELECT search_id, main_tag FROM search_documents")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            let (id, main_tag) = row.map_err(|error| error.to_string())?;
            let id = u64::try_from(id).map_err(|_| format!("Invalid negative search ID: {id}"))?;
            documents.insert(
                id,
                TopicDocument {
                    terms: Vec::new(),
                    occurrences: Vec::new(),
                    main_tag,
                },
            );
        }

        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS temp.question_topic_vocabulary
            USING fts5vocab('main', 'question_fts', 'instance');",
        )
        .map_err(|error| error.to_string())?;
        let mut statement = conn
            .prepare(
                "SELECT term, doc, col, offset FROM temp.question_topic_vocabulary
            WHERE length(term) >= 3 ORDER BY term, doc, col, offset",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u32>(3)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        let mut terms = Vec::new();
        for row in rows {
            let (term, id, field, offset) = row.map_err(|error| error.to_string())?;
            let id = u64::try_from(id).map_err(|_| format!("Invalid negative search ID: {id}"))?;
            if !informative_term(&term) {
                continue;
            }
            if terms.last() != Some(&term) {
                terms.push(term);
            }
            if let Some(document) = documents.get_mut(&id) {
                let field = match field.as_str() {
                    "question" => 0,
                    "options_text" => 1,
                    "main_tag" => 2,
                    "subtags_text" => 3,
                    _ => continue,
                };
                let term = terms.len() - 1;
                if document.terms.last() != Some(&term) {
                    document.terms.push(term);
                }
                document.occurrences.push(TopicOccurrence {
                    term,
                    field,
                    offset,
                });
            }
        }
        Ok(Self { terms, documents })
    }

    pub(crate) fn retain_supported(
        &self,
        candidates: &mut Vec<VectorHit>,
        lexical: &[LexicalHit],
        query: &CompiledFtsQuery,
        filters: &SearchFilter,
    ) {
        let primary = lexical
            .iter()
            .filter(|hit| !hit.relaxed)
            .map(|hit| hit.search_id as u64)
            .filter(|id| filters.allows_search_id(*id) && self.documents.contains_key(id))
            .collect::<HashSet<_>>();
        let mut background = vec![0usize; self.terms.len()];
        let mut foreground = vec![0usize; self.terms.len()];
        let query_words = query.word_patterns();
        let query_terms = self
            .terms
            .iter()
            .map(|term| {
                query_words
                    .iter()
                    .any(|word| word.text == *term || (word.prefix && term.starts_with(&word.text)))
            })
            .collect::<Vec<_>>();
        // Descriptive queries may have no all-word match. Use the best
        // vocabulary-overlap documents as evidence, not cosine neighbours.
        let anchors = if primary.is_empty() {
            let coverage = self
                .documents
                .iter()
                .filter(|(id, _)| filters.allows_search_id(**id))
                .map(|(&id, document)| {
                    (
                        id,
                        document
                            .terms
                            .iter()
                            .filter(|&&term| query_terms[term])
                            .count(),
                    )
                })
                .collect::<Vec<_>>();
            let best = coverage.iter().map(|(_, count)| *count).max().unwrap_or(0);
            coverage
                .into_iter()
                .filter(|(_, count)| *count > 0 && *count == best)
                .map(|(id, _)| id)
                .collect::<HashSet<_>>()
        } else {
            primary.clone()
        };
        if anchors.is_empty() {
            candidates.clear();
            return;
        }
        for &id in &anchors {
            let document = &self.documents[&id];
            let locations = document
                .occurrences
                .iter()
                .filter(|token| query_terms[token.term])
                .collect::<Vec<_>>();
            let mut nearby = document
                .occurrences
                .iter()
                .filter(|token| {
                    locations.iter().any(|location| {
                        token.field == location.field
                            && token.offset.abs_diff(location.offset) <= 24
                    })
                })
                .map(|token| token.term)
                .collect::<Vec<_>>();
            nearby.sort_unstable();
            nearby.dedup();
            for &term in &nearby {
                foreground[term] += 1;
            }
        }
        let mut tag_background = HashMap::<&str, usize>::new();
        let mut scope_count = 0usize;
        for (id, document) in &self.documents {
            if !filters.allows_search_id(*id) {
                continue;
            }
            scope_count += 1;
            *tag_background.entry(&document.main_tag).or_default() += 1;
            for &term in &document.terms {
                background[term] += 1;
            }
        }

        let mut associated = vec![false; self.terms.len()];
        let mut decisive = vec![false; self.terms.len()];
        for index in 0..self.terms.len() {
            // Partial coverage of the original query isn't independent topic
            // evidence. Include Porter stems when excluding its own words.
            if query_terms[index]
                || background[index] < 2
                || foreground[index] == 0
                || background[index] as f64 > (scope_count as f64 * 0.05).max(2.0)
            {
                continue;
            }
            let lift = (foreground[index] as f64 / anchors.len() as f64)
                / (background[index] as f64 / scope_count as f64);
            associated[index] = lift >= 2.0;
            // One shared word needs repeated independent support. Otherwise a
            // rare incidental word in a single source can connect unrelated topics.
            decisive[index] = lift >= 6.0 && foreground[index] >= 2;
        }

        let directly_supported = |document: &TopicDocument| {
            // Evidence must occur in the candidate's question/options, too.
            // Two distant incidental words or metadata labels are insufficient.
            let body = document
                .occurrences
                .iter()
                .filter(|token| token.field < 2)
                .collect::<Vec<_>>();
            if body.iter().any(|token| decisive[token.term]) {
                return true;
            }
            for (index, left) in body.iter().enumerate() {
                if !associated[left.term] && !query_terms[left.term] {
                    continue;
                }
                for right in &body[index + 1..] {
                    if left.term == right.term
                        || left.field != right.field
                        || left.offset.abs_diff(right.offset) > 24
                    {
                        continue;
                    }
                    if primary.is_empty() && query_terms[left.term] && query_terms[right.term] {
                        return true;
                    }
                    // Each term is independently associated with the query.
                    // Their nearby co-occurrence in this candidate connects the
                    // evidence even when it came from different literal sources.
                    if associated[left.term] && associated[right.term] {
                        return true;
                    }
                }
            }
            false
        };
        let mut supported = primary.clone();
        for candidate in candidates.iter() {
            if filters.allows_search_id(candidate.search_id)
                && self
                    .documents
                    .get(&candidate.search_id)
                    .is_some_and(&directly_supported)
            {
                supported.insert(candidate.search_id);
            }
        }

        // Taxonomy can support a weaker individual textual connection, but
        // a shared category never admits a candidate on its own.
        // A tag must be overrepresented and backed by multiple direct sources
        // or a connection independently repeated near the query in literal sources.
        // Newly admitted results never become evidence for further expansion.
        let mut tag_support = HashMap::<&str, usize>::new();
        let mut decisive_tags = HashSet::new();
        for id in &supported {
            let document = &self.documents[id];
            *tag_support.entry(&document.main_tag).or_default() += 1;
            if document.terms.iter().any(|term| decisive[*term]) {
                decisive_tags.insert(document.main_tag.as_str());
            }
        }
        let supported_tags = tag_support
            .into_iter()
            .filter_map(|(tag, count)| {
                let background_count = tag_background.get(tag).copied().unwrap_or(0);
                if tag.is_empty()
                    || (count < 2 && !decisive_tags.contains(tag))
                    || background_count == 0
                {
                    return None;
                }
                let lift = (count as f64 / supported.len() as f64)
                    / (background_count as f64 / scope_count as f64);
                (lift >= 3.0).then_some(tag)
            })
            .collect::<HashSet<_>>();
        candidates.retain(|candidate| {
            filters.allows_search_id(candidate.search_id)
                && (supported.contains(&candidate.search_id)
                    || self
                        .documents
                        .get(&candidate.search_id)
                        .is_some_and(|document| {
                            supported_tags.contains(document.main_tag.as_str())
                                && document
                                    .occurrences
                                    .iter()
                                    .any(|token| token.field < 2 && associated[token.term])
                        }))
        });
    }
}

fn informative_term(term: &str) -> bool {
    // Generic grammatical/question wording, expressed in Porter stems. These
    // remain searchable; only their use as corroborating evidence is excluded.
    term.chars().all(char::is_alphabetic)
        && !is_relaxed_stop_word(term)
        && !matches!(
            term,
            "about"
                | "abov"
                | "after"
                | "again"
                | "all"
                | "also"
                | "alwai"
                | "ani"
                | "anoth"
                | "answer"
                | "around"
                | "becaus"
                | "becom"
                | "been"
                | "befor"
                | "below"
                | "best"
                | "between"
                | "both"
                | "but"
                | "can"
                | "code"
                | "consid"
                | "correct"
                | "could"
                | "describ"
                | "discuss"
                | "doe"
                | "each"
                | "either"
                | "els"
                | "etc"
                | "even"
                | "ever"
                | "everi"
                | "examin"
                | "explain"
                | "few"
                | "find"
                | "first"
                | "follow"
                | "further"
                | "give"
                | "given"
                | "had"
                | "has"
                | "have"
                | "here"
                | "howev"
                | "into"
                | "its"
                | "itself"
                | "last"
                | "make"
                | "mani"
                | "may"
                | "might"
                | "more"
                | "most"
                | "much"
                | "must"
                | "name"
                | "neither"
                | "never"
                | "next"
                | "none"
                | "nor"
                | "not"
                | "now"
                | "often"
                | "one"
                | "onli"
                | "other"
                | "our"
                | "out"
                | "own"
                | "per"
                | "question"
                | "rather"
                | "refer"
                | "regard"
                | "same"
                | "say"
                | "second"
                | "see"
                | "seem"
                | "select"
                | "sever"
                | "shall"
                | "should"
                | "show"
                | "since"
                | "some"
                | "state"
                | "statement"
                | "still"
                | "such"
                | "take"
                | "than"
                | "that"
                | "their"
                | "them"
                | "then"
                | "there"
                | "these"
                | "thi"
                | "those"
                | "though"
                | "through"
                | "thu"
                | "togeth"
                | "too"
                | "two"
                | "under"
                | "until"
                | "upon"
                | "use"
                | "variou"
                | "veri"
                | "via"
                | "was"
                | "well"
                | "what"
                | "when"
                | "where"
                | "whether"
                | "whi"
                | "while"
                | "who"
                | "whose"
                | "will"
                | "within"
                | "without"
                | "would"
                | "yet"
                | "you"
                | "your"
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::lexical::query_builder::FtsQueryBuilder;
    use std::sync::Arc;

    fn fixture() -> TopicEvidence {
        let mut documents = (1..=100)
            .map(|id| {
                (
                    id,
                    TopicDocument {
                        terms: vec![0],
                        occurrences: vec![TopicOccurrence {
                            term: 0,
                            field: 0,
                            offset: 0,
                        }],
                        main_tag: format!("background-{}", id % 5),
                    },
                )
            })
            .collect::<HashMap<_, _>>();
        for (id, terms, tag) in [
            (1, vec![0, 1, 2, 3], "literal-topic"),
            (2, vec![0, 1, 2, 3], "literal-topic"),
            (3, vec![0, 2], "related-topic"),
            (4, vec![0, 3], "related-topic"),
            (5, vec![0, 4], "related-topic"),
            (6, vec![0, 4], "unrelated-topic"),
            (7, vec![0, 5], "unrelated-topic"),
        ] {
            documents.insert(
                id,
                TopicDocument {
                    occurrences: terms
                        .iter()
                        .enumerate()
                        .map(|(offset, &term)| TopicOccurrence {
                            term,
                            field: 0,
                            offset: offset as u32,
                        })
                        .collect(),
                    terms,
                    main_tag: tag.into(),
                },
            );
        }
        TopicEvidence {
            terms: [
                "common",
                "topic",
                "bridge",
                "connection",
                "neighbour",
                "abstract",
            ]
            .map(str::to_string)
            .to_vec(),
            documents,
        }
    }

    #[test]
    fn rejects_cosine_only_hubs_and_keeps_independently_supported_neighbours() {
        let evidence = fixture();
        let lexical = [1, 2].map(|id| LexicalHit {
            search_id: id,
            question_id: id.to_string(),
            score: 1.0,
            relaxed: false,
        });
        let mut candidates = [7, 6, 5, 4, 3, 2, 1]
            .map(|id| VectorHit {
                search_id: id,
                score: 0.9,
            })
            .to_vec();
        evidence.retain_supported(
            &mut candidates,
            &lexical,
            &FtsQueryBuilder::build("topic").unwrap(),
            &SearchFilter::default(),
        );
        let ids = candidates
            .iter()
            .map(|hit| hit.search_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![4, 3, 2, 1]);
        // A shared taxonomy label alone (5) cannot establish a relationship.
        assert!(!ids.contains(&6));
    }

    #[test]
    fn background_frequencies_follow_the_active_scope_and_literals_are_retained() {
        let evidence = fixture();
        let lexical = [1, 2].map(|id| LexicalHit {
            search_id: id,
            question_id: id.to_string(),
            score: 1.0,
            relaxed: false,
        });
        let filters = SearchFilter {
            allowed_search_ids: Some(Arc::new([1, 2, 3, 7].into_iter().collect())),
            ..SearchFilter::default()
        };
        let mut candidates = [7, 6, 3, 2, 1]
            .map(|id| VectorHit {
                search_id: id,
                score: 0.9,
            })
            .to_vec();
        evidence.retain_supported(
            &mut candidates,
            &lexical,
            &FtsQueryBuilder::build("topic").unwrap(),
            &filters,
        );
        assert_eq!(
            candidates
                .iter()
                .map(|hit| hit.search_id)
                .collect::<Vec<_>>(),
            vec![2, 1]
        );
    }

    #[test]
    fn distant_words_and_other_fields_do_not_establish_a_topic_connection() {
        let mut evidence = fixture();
        for id in [1, 2] {
            let document = evidence.documents.get_mut(&id).unwrap();
            for token in &mut document.occurrences {
                if token.term == 2 {
                    token.offset = 100;
                }
                if token.term == 3 {
                    token.field = 1;
                }
            }
        }
        let lexical = [1, 2].map(|id| LexicalHit {
            search_id: id,
            question_id: id.to_string(),
            score: 1.0,
            relaxed: false,
        });
        let mut candidates = [3, 4, 1]
            .map(|id| VectorHit {
                search_id: id,
                score: 0.9,
            })
            .to_vec();
        evidence.retain_supported(
            &mut candidates,
            &lexical,
            &FtsQueryBuilder::build("topic").unwrap(),
            &SearchFilter::default(),
        );
        assert_eq!(
            candidates
                .iter()
                .map(|hit| hit.search_id)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn descriptive_queries_without_a_complete_lexical_match_still_require_evidence() {
        let evidence = fixture();
        let mut candidates = [7, 6, 5, 4, 3]
            .map(|search_id| VectorHit {
                search_id,
                score: 0.95,
            })
            .to_vec();
        evidence.retain_supported(
            &mut candidates,
            &[],
            &FtsQueryBuilder::build("topic connection absent").unwrap(),
            &SearchFilter::default(),
        );
        assert_eq!(
            candidates
                .iter()
                .map(|hit| hit.search_id)
                .collect::<Vec<_>>(),
            vec![3]
        );
        evidence.retain_supported(
            &mut candidates,
            &[],
            &FtsQueryBuilder::build("unfamiliar hypothetical subject").unwrap(),
            &SearchFilter::default(),
        );
        assert!(candidates.is_empty());
    }

    #[test]
    fn candidate_evidence_must_also_be_nearby_and_in_the_body() {
        for (second_field, second_offset, expected) in
            [(0, 2, true), (0, 100, false), (2, 2, false)]
        {
            let mut evidence = fixture();
            // With one source, neither bridge word is decisive by itself.
            let candidate = evidence.documents.get_mut(&8).unwrap();
            candidate.terms = vec![0, 2, 3];
            candidate.occurrences = vec![
                TopicOccurrence {
                    term: 2,
                    field: 0,
                    offset: 1,
                },
                TopicOccurrence {
                    term: 3,
                    field: second_field,
                    offset: second_offset,
                },
            ];
            let lexical = [LexicalHit {
                search_id: 1,
                question_id: "1".into(),
                score: 1.0,
                relaxed: false,
            }];
            let mut candidates = vec![VectorHit {
                search_id: 8,
                score: 0.95,
            }];
            evidence.retain_supported(
                &mut candidates,
                &lexical,
                &FtsQueryBuilder::build("topic").unwrap(),
                &SearchFilter::default(),
            );
            assert_eq!(!candidates.is_empty(), expected);
        }
    }

    #[test]
    fn nearby_candidate_terms_can_connect_independent_literal_sources() {
        for (field, offset, expected) in [(0, 2, true), (0, 100, false), (2, 2, false)] {
            let mut evidence = fixture();
            // Neither bridge term has repeated support or shares an anchor
            // with the other. Both are independently connected to the query.
            for (id, removed) in [(1, 3), (2, 2)] {
                let source = evidence.documents.get_mut(&id).unwrap();
                source.terms.retain(|term| *term != removed);
                source.occurrences.retain(|token| token.term != removed);
            }
            let candidate = evidence.documents.get_mut(&8).unwrap();
            candidate.terms = vec![0, 2, 3];
            candidate.occurrences = vec![
                TopicOccurrence {
                    term: 2,
                    field: 0,
                    offset: 1,
                },
                TopicOccurrence {
                    term: 3,
                    field,
                    offset,
                },
            ];
            let lexical = [1, 2].map(|search_id| LexicalHit {
                search_id,
                question_id: search_id.to_string(),
                score: 1.0,
                relaxed: false,
            });
            let mut candidates = vec![VectorHit {
                search_id: 8,
                score: 0.95,
            }];
            evidence.retain_supported(
                &mut candidates,
                &lexical,
                &FtsQueryBuilder::build("topic").unwrap(),
                &SearchFilter::default(),
            );
            assert_eq!(!candidates.is_empty(), expected);
        }
    }

    #[test]
    fn generic_wording_is_not_topic_evidence() {
        for term in ["find", "one", "see", "best", "answer", "statement", "201"] {
            assert!(!informative_term(term));
        }
        for term in ["river", "metal", "dam", "carbon", "philosophi"] {
            assert!(informative_term(term));
        }
    }
}
