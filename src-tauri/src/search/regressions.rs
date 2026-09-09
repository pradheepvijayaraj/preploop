//! Deterministic request-path regressions; no model downloads or inference.

use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use super::embedding::engine::{Embedding, EmbeddingEngine, EmbeddingError};
use super::filters::SearchFilter;
use super::request::{SearchOptions, SearchRequest};
use super::response::{MatchStrength, SemanticStatus};
use super::service::SearchService;
use super::vector::traits::{VectorHit, VectorSearch};

#[derive(Default)]
struct RecordingEngine {
    queries: Mutex<Vec<String>>,
    fail: bool,
}

impl EmbeddingEngine for RecordingEngine {
    fn dimensions(&self) -> usize {
        1
    }

    fn embed_query(&self, text: &str) -> Result<Embedding, EmbeddingError> {
        self.queries.lock().unwrap().push(text.into());
        if self.fail {
            Err(EmbeddingError::Inference("fixture failure".into()))
        } else {
            Ok(vec![1.0])
        }
    }

    fn embed_documents(&self, _: &[String]) -> Result<Vec<Embedding>, EmbeddingError> {
        unreachable!("query search must not re-embed documents")
    }
}

struct EqualScoreIndex;

impl VectorSearch for EqualScoreIndex {
    fn search(
        &self,
        _: &[f32],
        filters: &SearchFilter,
        limit: usize,
    ) -> Result<Vec<VectorHit>, String> {
        Ok((1..=7)
            .filter(|id| filters.allows_search_id(*id))
            .take(limit)
            .map(|search_id| VectorHit {
                search_id,
                score: 0.95,
            })
            .collect())
    }
    fn count(&self) -> usize {
        7
    }
    fn generation(&self) -> u32 {
        1
    }
}

fn fixture() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::backend::db::schema::run_migrations(&conn).unwrap();
    conn.execute("INSERT INTO question_banks (id, name, exam, metadata, total_questions, difficulty, default_duration, imported_at)
        VALUES ('b', 'Bank', 'UPSC', '{}', 7, 'medium', 60, 1)", []).unwrap();
    for (id, text, tag) in [
        (
            1,
            "Silver Notice traces criminal assets.",
            "International Institutions",
        ),
        (2, "Silver deposits. Notice the criminal assets.", "Economy"),
        (3, "Discuss legislative accountability.", "Parliament"),
        (
            4,
            "Silver notices trace criminal assets.",
            "International Institutions",
        ),
        (5, "River conservation protects ecosystems.", "Environment"),
        (6, "Rider conservation and rider safety.", "Ethics"),
        (
            7,
            "Parliament can amend the constitution.",
            "Constitutional Amendment",
        ),
    ] {
        let question_id = format!("q{id}");
        conn.execute(
            "INSERT INTO questions (id, bank_id, type, question, correct_answers, marks)
            VALUES (?1, 'b', 'single', ?2, '[]', 1)",
            rusqlite::params![question_id, text],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO question_taxonomy (question_id, main_tag) VALUES (?1, ?2)",
            rusqlite::params![question_id, tag],
        )
        .unwrap();
        conn.execute("INSERT INTO search_documents (search_id, question_id, question, main_tag, bank_id, bank_name, section, content_fingerprint)
            VALUES (?1, ?2, ?3, ?4, 'b', 'Bank', 'prelims-gs1', X'0102030405060708')",
            rusqlite::params![id, question_id, text, tag]).unwrap();
    }
    conn
}

fn hybrid(engine: Arc<RecordingEngine>) -> SearchService {
    SearchService::new(Some(engine), Some(Arc::new(EqualScoreIndex)))
}

#[test]
fn correction_is_shared_by_lexical_and_embedding_retrieval() {
    let conn = fixture();
    let engine = Arc::new(RecordingEngine::default());
    let service = hybrid(engine.clone());
    let result = service
        .execute_question_search(&conn, "silvre notice", None)
        .unwrap();
    assert_eq!(result.corrected_query.as_deref(), Some("silver notice"));
    assert_eq!(*engine.queries.lock().unwrap(), vec!["silver notice"]);
    assert!(result
        .results
        .iter()
        .any(|result| result.question_id == "q1"));
}

#[test]
fn phrases_constrain_surface_words_in_every_retrieval_path() {
    let conn = fixture();
    let engine = Arc::new(RecordingEngine::default());
    let service = hybrid(engine.clone());
    for query in [
        "\"silver notice\" criminal assets",
        "\"silver notice\" criminal missing",
    ] {
        let result = service.execute_question_search(&conn, query, None).unwrap();
        assert!(!result.results.is_empty(), "{query}");
        assert!(
            result
                .results
                .iter()
                .all(|result| result.question_id == "q1"),
            "{query}"
        );
    }
    let calls = engine.queries.lock().unwrap().len();
    for query in [
        "\"silver notice\"",
        "\"silvre\" \"notice\"",
        "\"missing phrase\" assets",
    ] {
        let result = service.execute_question_search(&conn, query, None).unwrap();
        assert_eq!(result.semantic_status, SemanticStatus::NotRequested);
    }
    assert_eq!(engine.queries.lock().unwrap().len(), calls);
}

#[test]
fn exact_topic_search_unions_tagged_and_literal_results() {
    let conn = fixture();
    let result = SearchService::new(None, None)
        .execute_question_search(&conn, "parliament", None)
        .unwrap();
    for expected in ["q3", "q7"] {
        assert!(
            result
                .results
                .iter()
                .any(|result| result.question_id == expected
                    && result.match_strength == MatchStrength::Strong),
            "{expected}"
        );
    }
}

#[test]
fn ambiguity_is_not_resolved_by_frequency_or_embeddings() {
    let conn = fixture();
    let engine = Arc::new(RecordingEngine::default());
    let result = hybrid(engine.clone())
        .execute_question_search(&conn, "riber conservation", None)
        .unwrap();
    assert!(result.corrected_query.is_none());
    assert_eq!(result.spelling_alternatives.len(), 2);
    assert_eq!(result.semantic_status, SemanticStatus::NotRequested);
    assert!(engine.queries.lock().unwrap().is_empty());
}

#[test]
fn keyword_preview_does_not_embed_and_failure_retains_literal_matches() {
    let conn = fixture();
    let engine = Arc::new(RecordingEngine {
        fail: true,
        ..Default::default()
    });
    let service = hybrid(engine.clone());
    let preview = service
        .execute_question_search_with_options(
            &conn,
            "silver notice",
            None,
            &SearchOptions {
                semantic: false,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(preview.semantic_status, SemanticStatus::Pending);
    assert!(!preview.results.is_empty());
    assert!(engine.queries.lock().unwrap().is_empty());
    let completed = service
        .execute_question_search(&conn, "silver notice", None)
        .unwrap();
    assert_eq!(completed.semantic_status, SemanticStatus::Unavailable);
    assert_eq!(
        preview
            .results
            .iter()
            .map(|result| &result.question_id)
            .collect::<Vec<_>>(),
        completed
            .results
            .iter()
            .map(|result| &result.question_id)
            .collect::<Vec<_>>()
    );
    let cancelled = SearchOptions::default();
    cancelled.cancellation.cancel();
    assert!(service
        .search_with_options(
            &conn,
            &SearchRequest {
                query: "silver".into(),
                filters: SearchFilter::default(),
                limit: 10,
            },
            &cancelled
        )
        .is_err());
    assert_eq!(engine.queries.lock().unwrap().len(), 1);
}

#[test]
fn no_supported_semantic_matches_is_distinct_from_unavailable_search() {
    let conn = fixture();
    let service = hybrid(Arc::new(RecordingEngine::default()));
    let result = service
        .execute_question_search(&conn, "unfamiliar hypothetical subject", None)
        .unwrap();
    assert!(result.results.is_empty());
    assert_eq!(result.semantic_status, SemanticStatus::Available);
}

#[test]
fn structured_filters_intersect_existing_ids_in_both_retrieval_paths() {
    let conn = fixture();
    conn.execute(
        "UPDATE search_documents SET year = 2025, stage = 'prelims', paper = 'GS1'",
        [],
    )
    .unwrap();
    let service = hybrid(Arc::new(RecordingEngine::default()));
    let filters = SearchFilter {
        sections: vec!["prelims-gs1".into()],
        stages: vec!["prelims".into()],
        papers: vec!["GS1".into()],
        banks: vec!["b".into()],
        years: Some((2024, 2026)),
        tags: vec!["International Relations".into()],
        allowed_search_ids: Some(Arc::new([1, 2].into_iter().collect())),
    };
    for query in ["silver", "\"silver notice\""] {
        let compiled = super::lexical::query_builder::FtsQueryBuilder::build(query).unwrap();
        let lexical =
            super::lexical::fts::LexicalSearch::search(&conn, &compiled, &filters, 20).unwrap();
        assert_eq!(
            lexical.iter().map(|hit| hit.search_id).collect::<Vec<_>>(),
            vec![1]
        );
        let result = service
            .search(
                &conn,
                &SearchRequest {
                    query: query.into(),
                    filters: filters.clone(),
                    limit: 20,
                },
            )
            .unwrap();
        assert_eq!(
            result
                .hits
                .iter()
                .map(|hit| hit.search_id)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }
    let empty = service
        .search(
            &conn,
            &SearchRequest {
                query: "silver".into(),
                filters: SearchFilter {
                    allowed_search_ids: Some(Arc::new(Default::default())),
                    ..filters
                },
                limit: 20,
            },
        )
        .unwrap();
    assert!(
        empty.hits.is_empty(),
        "an empty restriction must not expand to the whole scope"
    );
}

#[test]
fn long_queries_keep_primary_and_two_word_fallback_matches() {
    let conn = fixture();
    let query = (0..70)
        .map(|index| format!("word{index}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(query.chars().count() <= 512);
    conn.execute(
        "UPDATE search_documents SET question = ?1, main_tag = '' WHERE search_id = 1",
        [&query],
    )
    .unwrap();
    conn.execute(
        "UPDATE search_documents SET question = 'word1 word68', main_tag = '' WHERE search_id = 2",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE search_documents SET question = 'word1', main_tag = '' WHERE search_id = 3",
        [],
    )
    .unwrap();
    let compiled = super::lexical::query_builder::FtsQueryBuilder::build(&query).unwrap();
    let hits =
        super::lexical::fts::LexicalSearch::search(&conn, &compiled, &SearchFilter::default(), 20)
            .unwrap();
    assert!(hits.iter().any(|hit| hit.search_id == 1 && !hit.relaxed));
    assert!(hits.iter().any(|hit| hit.search_id == 2 && hit.relaxed));
    assert!(!hits.iter().any(|hit| hit.search_id == 3));
    let response = SearchService::new(None, None)
        .execute_question_search(&conn, &query, None)
        .unwrap();
    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].question_id, "q1");
}

#[test]
fn missing_semantic_resources_finish_the_keyword_phase() {
    let conn = fixture();
    let engine = Arc::new(RecordingEngine::default());
    for service in [
        SearchService::new(None, Some(Arc::new(EqualScoreIndex))),
        SearchService::new(Some(engine.clone()), None),
    ] {
        let response = service
            .execute_question_search_with_options(
                &conn,
                "silver",
                None,
                &SearchOptions {
                    semantic: false,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(!response.results.is_empty());
        assert_eq!(response.semantic_status, SemanticStatus::Unavailable);
    }
    assert!(engine.queries.lock().unwrap().is_empty());
}

#[test]
fn database_failure_is_not_reported_as_an_empty_corpus() {
    let conn = Connection::open_in_memory().unwrap();
    let error = SearchService::new(None, None)
        .execute_question_search(&conn, "silver", None)
        .unwrap_err();
    assert!(error.contains("Could not count searchable questions"));
}
