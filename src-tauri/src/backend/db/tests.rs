//! Integration tests for the database layer.
//!
//! All tests use an in-memory SQLite database so they are fast and
//! isolated.  Each test creates a fresh connection with FK enforcement
//! and runs all migrations from scratch.
//!
//! TEST STRATEGY:
//! - `schema_migration_is_idempotent`: Ensures re-running migrations is safe.
//! - `save_answer_rejects_completed_attempt`: Validates the TOCTOU fix (#14)
//!   by confirming that answers can't be saved after submission.

#[cfg(test)]
// The outer `db::tests` module is file-based; this inner module keeps all
// integration-only imports and helpers scoped away from production code.
#[allow(clippy::module_inception)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use rusqlite::Connection;

    use crate::backend::db::question_bank::import_question_bank;
    use crate::backend::db::schema::run_migrations;
    use crate::backend::db::search::{
        invalidate_search_index, search_questions_cached, semantic_index_freshness,
        semantic_tag_coverage, SearchIndexState,
    };
    use crate::backend::types::{
        Difficulty, Question, QuestionBank, QuestionBankMetadata, QuestionMarkBreakdown,
        QuestionSearchResponse, QuestionType,
    };
    use crate::search::response::MatchStrength;
    use crate::taxonomy::{MainTag, QuestionTaxonomy, Subtag};

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    fn assert_confidence_groups_are_ordered(response: &QuestionSearchResponse) {
        let first_related = response
            .results
            .iter()
            .position(|result| result.match_strength == MatchStrength::Related);
        if let Some(index) = first_related {
            assert!(response.results[index..]
                .iter()
                .all(|result| result.match_strength == MatchStrength::Related));
        }
        assert!(response.results.iter().all(|result| {
            result.lexical_match
                || result.semantic_match
                || result.match_strength == MatchStrength::Strong
        }));
        assert!(response
            .results
            .windows(2)
            .all(|pair| pair[0].similarity >= pair[1].similarity));
    }

    fn sample_bank() -> QuestionBank {
        QuestionBank {
            metadata: QuestionBankMetadata {
                name: "Sample Bank".to_string(),
                exam: "Mock Exam".to_string(),
                total_questions: 1,
                difficulty: Difficulty::Medium,
                default_duration: 600,
                extra: Default::default(),
            },
            questions: vec![Question {
                id: "q-1".to_string(),
                question_type: QuestionType::SingleChoice,
                question: "Sample question?".to_string(),
                options: Some(vec![
                    crate::backend::types::QuestionOption {
                        id: "a".to_string(),
                        text: "A".to_string(),
                    },
                    crate::backend::types::QuestionOption {
                        id: "b".to_string(),
                        text: "B".to_string(),
                    },
                ]),
                correct_answers: vec!["a".to_string()],
                explanation: "Because".to_string(),
                is_open_ended: false,
                marks: 2.0,
                mark_breakdown: Vec::new(),
                negative_marks: 0.5,
                negative_marks_unanswered: 0.0,
                time_estimate: Some(30),
                difficulty: Some(Difficulty::Medium),
                tags: vec!["logic".to_string()],
                taxonomy: None,
            }],
        }
    }

    #[test]
    fn schema_migration_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        // Running migrations a second time should be a no-op (not error)
        // because every CREATE TABLE uses IF NOT EXISTS and the schema_version
        // check skips already-applied migrations.
        run_migrations(&conn).unwrap();
    }

    #[test]
    fn schema_version_is_normalized_to_one_constrained_row() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        conn.execute_batch(
            "DROP TABLE schema_version;
             CREATE TABLE schema_version (version INTEGER NOT NULL);
             INSERT INTO schema_version VALUES (4), (5), (5);",
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        let rows: (i64, i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), MIN(id), MAX(version) FROM schema_version",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(rows, (1, 1, 6));
        assert!(conn
            .execute("INSERT INTO schema_version (id, version) VALUES (2, 6)", [])
            .is_err());
    }

    #[test]
    fn v6_recovers_when_column_exists_before_version_update() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        // Simulate interruption after ALTER TABLE committed but before the old
        // runner recorded version 6.
        conn.execute("UPDATE schema_version SET version = 5 WHERE id = 1", [])
            .unwrap();

        run_migrations(&conn).unwrap();

        let version: i64 = conn
            .query_row(
                "SELECT version FROM schema_version WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 6);
        let mark_breakdown_columns: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('questions') WHERE name = 'mark_breakdown'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mark_breakdown_columns, 1);
    }

    #[test]
    fn failed_migration_rolls_back_schema_and_version_then_retries() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        conn.execute_batch(
            "UPDATE schema_version SET version = 4 WHERE id = 1;
             DROP TABLE question_fts;
             DROP TABLE search_documents;
             DROP TABLE search_index_jobs;
             DROP TABLE question_taxonomy;
             CREATE TABLE search_documents (search_id INTEGER PRIMARY KEY);",
        )
        .unwrap();

        // The malformed pre-existing table makes v5 fail after it has already
        // created other schema objects. Those objects and the version update
        // must be rolled back together.
        assert!(run_migrations(&conn).is_err());
        let version_after_failure: i64 = conn
            .query_row(
                "SELECT version FROM schema_version WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version_after_failure, 4);
        for object in ["question_fts", "search_index_jobs", "question_taxonomy"] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE name = ?1",
                    [object],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "{object} was not rolled back");
        }

        conn.execute("DROP TABLE search_documents", []).unwrap();
        run_migrations(&conn).unwrap();

        let version_after_retry: i64 = conn
            .query_row(
                "SELECT version FROM schema_version WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version_after_retry, 6);
        for object in ["search_documents", "question_fts", "search_index_jobs"] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE name = ?1",
                    [object],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "{object} was not created on retry");
        }
    }

    #[test]
    fn question_search_covers_the_corpus_and_ranks_related_concepts() {
        let mut conn = setup_conn();
        let mut bank = sample_bank();
        bank.metadata.name = "UPSC CSE Mains GS 2 2024".to_string();
        bank.metadata.exam = "UPSC CSE".to_string();
        bank.metadata.total_questions = 2;
        bank.metadata
            .extra
            .insert("year".to_string(), serde_json::json!(2024));
        bank.metadata
            .extra
            .insert("stage".to_string(), serde_json::json!("mains"));
        bank.metadata
            .extra
            .insert("paper".to_string(), serde_json::json!("GS2"));
        bank.metadata
            .extra
            .insert("section".to_string(), serde_json::json!("mains-gs2"));
        bank.questions[0].question =
            "Explain how Parliament ensures constitutional accountability.".to_string();
        bank.questions[0].tags = vec!["constitution".to_string()];

        let mut climate_question = bank.questions[0].clone();
        climate_question.id = "q-2".to_string();
        climate_question.question =
            "Assess the effects of climate change on Himalayan ecology.".to_string();
        climate_question.tags = vec!["environment".to_string()];
        bank.questions.push(climate_question);

        import_question_bank(&mut conn, &bank).unwrap();
        let search_index = SearchIndexState::default();

        let constitution =
            search_questions_cached(&conn, &search_index, "constitutional", None).unwrap();
        assert_eq!(constitution.searched_questions, 2);
        assert_eq!(constitution.results[0].question_id, "q-1");

        let typo = search_questions_cached(&conn, &search_index, "parliment", None).unwrap();
        assert_eq!(typo.results[0].question_id, "q-1");

        let ecology = search_questions_cached(&conn, &search_index, "ecology", None).unwrap();
        assert_eq!(ecology.results[0].question_id, "q-2");
    }

    #[test]
    fn question_search_respects_section_scope_and_cache_invalidation() {
        let mut conn = setup_conn();
        let mut prelims_bank = sample_bank();
        prelims_bank.metadata.name = "UPSC CSE Prelims GS 1 2024".to_string();
        prelims_bank.metadata.exam = "UPSC CSE".to_string();
        prelims_bank
            .metadata
            .extra
            .insert("year".to_string(), serde_json::json!(2024));
        prelims_bank
            .metadata
            .extra
            .insert("section".to_string(), serde_json::json!("prelims-gs1"));
        prelims_bank.questions[0].id = "prelims-q-1".to_string();
        prelims_bank.questions[0].question =
            "Which constitutional body audits public expenditure?".to_string();
        import_question_bank(&mut conn, &prelims_bank).unwrap();

        let mut mains_bank = prelims_bank.clone();
        mains_bank.metadata.name = "UPSC CSE Mains GS 2 2024".to_string();
        mains_bank
            .metadata
            .extra
            .insert("section".to_string(), serde_json::json!("mains-gs2"));
        mains_bank.questions[0].id = "mains-q-1".to_string();
        mains_bank.questions[0].question =
            "Discuss constitutional accountability in public expenditure.".to_string();
        import_question_bank(&mut conn, &mains_bank).unwrap();

        let prelims_sections = vec!["prelims-gs1".to_string()];
        let cache = SearchIndexState::default();
        let scoped = search_questions_cached(
            &conn,
            &cache,
            "constitutional expenditure",
            Some(&prelims_sections),
        )
        .unwrap();
        assert_eq!(scoped.searched_questions, 1);
        assert_eq!(scoped.results.len(), 1);
        assert_eq!(scoped.results[0].section, "prelims-gs1");

        let cached =
            search_questions_cached(&conn, &cache, "constitutional", Some(&prelims_sections))
                .unwrap();
        assert_eq!(cached.searched_questions, 1);

        let mut second_prelims = prelims_bank.clone();
        second_prelims.metadata.name = "UPSC CSE Prelims GS 1 2025".to_string();
        second_prelims
            .metadata
            .extra
            .insert("year".to_string(), serde_json::json!(2025));
        second_prelims.questions[0].id = "prelims-q-2".to_string();
        import_question_bank(&mut conn, &second_prelims).unwrap();

        invalidate_search_index(&cache).unwrap();
        let refreshed =
            search_questions_cached(&conn, &cache, "constitutional", Some(&prelims_sections))
                .unwrap();
        assert_eq!(refreshed.searched_questions, 2);
    }

    #[test]
    fn bundled_upsc_corpus_is_fully_searchable_and_similarity_sorted() {
        let mut conn = setup_conn();
        let corpus_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../static/upsc");
        let catalog: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(corpus_root.join("catalog.json")).unwrap())
                .unwrap();
        let papers = catalog["papers"].as_array().unwrap();
        let mut expected_questions = 0usize;
        let mut expected_math_questions = 0usize;
        let default_search_section_ids = [
            "prelims-gs1",
            "mains-essay",
            "mains-gs1",
            "mains-gs2",
            "mains-gs3",
            "mains-gs4",
        ];
        let mut expected_default_questions = 0usize;
        let mut main_tag_counts = BTreeMap::<String, usize>::new();
        let mut subtag_counts = BTreeMap::<String, usize>::new();
        let mut section_main_tag_counts = BTreeMap::<String, BTreeMap<String, usize>>::new();

        for paper in papers {
            let relative_path = paper["path"].as_str().unwrap();
            let section = paper["section"].as_str().unwrap();
            let bank: QuestionBank =
                serde_json::from_str(&fs::read_to_string(corpus_root.join(relative_path)).unwrap())
                    .unwrap();
            expected_questions += bank.questions.len();
            if default_search_section_ids.contains(&section) {
                expected_default_questions += bank.questions.len();
            }
            if section.starts_with("mains-maths") {
                expected_math_questions += bank.questions.len();
            }
            for question in &bank.questions {
                let resolved = question
                    .taxonomy
                    .as_ref()
                    .expect("bundled question must have taxonomy")
                    .resolve()
                    .unwrap();
                *main_tag_counts
                    .entry(resolved.main_tag.to_string())
                    .or_default() += 1;
                *section_main_tag_counts
                    .entry(section.to_string())
                    .or_default()
                    .entry(resolved.main_tag.to_string())
                    .or_default() += 1;
                for subtag in resolved.subtags {
                    *subtag_counts.entry(subtag.to_string()).or_default() += 1;
                }
            }
            import_question_bank(&mut conn, &bank).unwrap();
        }

        assert!(expected_questions > 4_000);
        let (fresh_semantic_records, indexed_documents) = semantic_index_freshness(&conn).unwrap();
        assert_eq!(indexed_documents, expected_questions);
        assert_eq!(
            fresh_semantic_records, expected_questions,
            "the bundled search projection is stale"
        );
        let (tagged_documents, indexed_documents) = semantic_tag_coverage(&conn).unwrap();
        assert_eq!(tagged_documents, indexed_documents);
        let search_index = SearchIndexState::default();

        // Exact taxonomy labels are exhaustive filters. Their result counts
        // come from the typed taxonomy stored on each bundled question.
        for (main_tag, expected) in &main_tag_counts {
            let response = search_questions_cached(&conn, &search_index, main_tag, None).unwrap();
            assert_eq!(
                response.results.len(),
                *expected,
                "incomplete {main_tag} result set"
            );
            assert_eq!(response.total_matches, *expected);
            assert!(response
                .results
                .iter()
                .all(|result| result.main_tag == *main_tag));
            assert!(response.results.iter().all(|result| {
                result.match_strength == MatchStrength::Strong
                    && result.lexical_match
                    && !result.semantic_match
            }));
        }

        for (subtag, expected) in &subtag_counts {
            // A few essay dimensions intentionally reuse a plain-language
            // label that is also a main tag (for example, "Culture"). The
            // exact-query shortcut gives the main tag precedence, so those
            // ambiguous labels are covered by the main-tag loop above.
            if main_tag_counts.contains_key(subtag) {
                continue;
            }
            let response = search_questions_cached(&conn, &search_index, subtag, None).unwrap();
            assert_eq!(
                response.results.len(),
                *expected,
                "incomplete {subtag} result set"
            );
            assert_eq!(response.total_matches, *expected);
            assert!(response.results.iter().all(|result| {
                result
                    .subtags
                    .iter()
                    .any(|result_subtag| result_subtag == subtag)
            }));
            assert!(response.results.iter().all(|result| {
                result.match_strength == MatchStrength::Strong
                    && result.lexical_match
                    && !result.semantic_match
            }));
        }

        for (section, tag_counts) in &section_main_tag_counts {
            let sections = vec![section.clone()];
            for (main_tag, expected) in tag_counts {
                let response =
                    search_questions_cached(&conn, &search_index, main_tag, Some(&sections))
                        .unwrap();
                assert_eq!(response.results.len(), *expected);
                assert_eq!(response.total_matches, *expected);
                assert!(response
                    .results
                    .iter()
                    .all(|result| { result.section == *section && result.main_tag == *main_tag }));
            }
        }

        let normalized_taxonomy_query =
            search_questions_cached(&conn, &search_index, "polity and constitution", None).unwrap();
        let polity_alias = crate::taxonomy::legacy_main_tag_alias("Polity & Constitution")
            .expect("retired polity tag must retain a compatibility scope");
        let expected_polity = polity_alias
            .main_tags
            .iter()
            .map(|tag| main_tag_counts.get(tag).copied().unwrap_or_default())
            .sum::<usize>();
        assert_eq!(normalized_taxonomy_query.results.len(), expected_polity);
        assert!(normalized_taxonomy_query.results.iter().all(|result| {
            polity_alias
                .main_tags
                .iter()
                .any(|tag| tag == &result.main_tag)
        }));

        let response =
            search_questions_cached(&conn, &search_index, "climate change", None).unwrap();
        assert_eq!(response.searched_questions, expected_questions);
        assert!(!response.results.is_empty());
        assert!(response
            .results
            .iter()
            .all(|result| { !result.main_tag.trim().is_empty() && result.subtags.len() <= 4 }));
        assert!(response
            .results
            .windows(2)
            .all(|pair| pair[0].similarity >= pair[1].similarity));

        let reused_index =
            search_questions_cached(&conn, &search_index, "parliament", None).unwrap();
        assert_eq!(reused_index.searched_questions, expected_questions);
        assert!(!reused_index.results.is_empty());

        let water = search_questions_cached(&conn, &search_index, "water", None).unwrap();
        assert_confidence_groups_are_ordered(&water);
        assert!(water
            .results
            .iter()
            .all(|result| result.question_id != "upsc_2013_csat_q13"));
        assert!(water
            .results
            .windows(2)
            .all(|pair| { pair[0].similarity >= pair[1].similarity }));
        assert!(water.results.iter().any(|result| {
            let visible_text = format!(
                "{} {}",
                result.question,
                result
                    .options
                    .iter()
                    .map(|option| option.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            )
            .to_lowercase();
            !visible_text.contains("water")
                && ["river", "ocean", "sea", "lake", "wetland", "marine"]
                    .iter()
                    .any(|concept| visible_text.contains(concept))
        }));

        let river = search_questions_cached(&conn, &search_index, "river", None).unwrap();
        assert_confidence_groups_are_ordered(&river);
        for (query, expected_question) in [
            ("river", "upsc_2016_gs1_q23"),
            ("ocean", "upsc_2021_gs1_q58"),
            ("groundwater depletion", "upsc_2025_mains_gs3_q13"),
            ("forest conservation", "upsc_2016_gs1_q69"),
            ("inflation", "upsc_2015_gs1_q87"),
            ("parliament accountability", "upsc_2021_mains_gs2_q4"),
            ("cross-border cyber attacks", "upsc_2021_mains_gs3_q10"),
            ("maternal health", "upsc_2020_mains_gs2_q6"),
            ("unemployment", "upsc_2023_mains_gs3_q11"),
            ("food security", "upsc_2021_mains_gs3_q13"),
        ] {
            let response = search_questions_cached(&conn, &search_index, query, None).unwrap();
            assert_confidence_groups_are_ordered(&response);
            assert!(
                response
                    .results
                    .iter()
                    .any(|result| result.question_id == expected_question),
                "expected {expected_question} in results for {query}"
            );
        }

        // Numeric constraints are exact-intent across query families. Granite
        // may recover conceptual neighbours as Related, but only a document
        // containing each requested text-number pair may enter Strong.
        for (query, expected_pair) in [
            ("Article 14", "article 14"),
            ("Article 20", "article 20"),
            ("Article 21", "article 21"),
            ("Article 32", "article 32"),
            ("Article 44", "article 44"),
            ("calendar 2025", "calendar 2025"),
            ("SDG 5", "sdg 5"),
            ("20 percent", "20 percent"),
        ] {
            let response = search_questions_cached(&conn, &search_index, query, None).unwrap();
            assert_confidence_groups_are_ordered(&response);
            assert!(
                response.results.iter().all(|result| {
                    if result.match_strength == MatchStrength::Related {
                        return true;
                    }
                    let visible_text = format!(
                        "{} {}",
                        result.question,
                        result
                            .options
                            .iter()
                            .map(|option| option.text.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    )
                    .to_lowercase();
                    visible_text
                        .split(|character: char| !character.is_alphanumeric())
                        .filter(|token| !token.is_empty())
                        .collect::<Vec<_>>()
                        .windows(2)
                        .any(|pair| format!("{} {}", pair[0], pair[1]) == expected_pair)
                }),
                "strong result did not contain exact numeric constraint for {query}"
            );
        }

        for query in ["Article 14", "Article 21", "Article 32", "Article 44"] {
            let response = search_questions_cached(&conn, &search_index, query, None).unwrap();
            assert!(response
                .results
                .iter()
                .any(|result| result.match_strength == MatchStrength::Strong));
        }

        // Query families exercise the generic confidence filter. These cover
        // exact entities, broad concepts, descriptive paraphrases, CSAT
        // reasoning, and typo recovery rather than one special-case term.
        for (query, expected_question) in [
            ("Article 32", "upsc_2021_gs1_q85"),
            ("different cities", "upsc_2013_csat_q13"),
            ("parliament accountability", "upsc_2021_mains_gs2_q4"),
            ("probability", "upsc_2016_csat_q15"),
            ("Bhakti movement", "upsc_2018_mains_gs1_q11"),
        ] {
            let family = search_questions_cached(&conn, &search_index, query, None).unwrap();
            assert_confidence_groups_are_ordered(&family);
            let expected = family
                .results
                .iter()
                .find(|result| result.question_id == expected_question)
                .unwrap_or_else(|| panic!("expected {expected_question} for {query}"));
            assert_eq!(
                expected.question_number,
                expected_question
                    .rsplit_once("_q")
                    .and_then(|(_, value)| value.parse().ok())
            );
        }

        let typo_recovery =
            search_questions_cached(&conn, &search_index, "parliment accountability", None)
                .unwrap();
        assert_confidence_groups_are_ordered(&typo_recovery);
        assert!(typo_recovery.results.iter().take(10).any(|result| {
            result.question.to_lowercase().contains("parliament")
                && result.match_strength == MatchStrength::Strong
        }));

        for query in [
            "zxqvplmnr",
            "sourdough pizza recipe",
            "crochet pattern for a stuffed dinosaur",
            "video game speedrunning tutorial",
            "guitar chord fingering exercise",
        ] {
            let response = search_questions_cached(&conn, &search_index, query, None).unwrap();
            assert!(
                response.results.is_empty(),
                "out-of-domain query {query:?} returned {} arbitrary neighbours",
                response.results.len(),
            );
        }

        let former_river_false_positives = [
            "upsc_2024_gs1_q70",
            "upsc_2026_gs1_q100",
            "upsc_2025_gs1_q66",
            "upsc_2013_csat_q35",
        ];
        assert!(river.results.iter().take(40).all(|result| {
            !former_river_false_positives.contains(&result.question_id.as_str())
        }));
        assert!(river
            .results
            .windows(2)
            .all(|pair| { pair[0].similarity >= pair[1].similarity }));
        assert!(river.results.iter().any(|result| {
            let visible_text = format!(
                "{} {}",
                result.question,
                result
                    .options
                    .iter()
                    .map(|option| option.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            )
            .to_lowercase();
            !visible_text.contains("river")
                && [
                    "water",
                    "ocean",
                    "sea",
                    "lake",
                    "wetland",
                    "marine",
                    "groundwater",
                    "aquifer",
                ]
                .iter()
                .any(|concept| visible_text.contains(concept))
        }));
        assert!(river.results.iter().any(|result| {
            [
                "Physical Geography",
                "Indian Geography",
                "World Geography",
                "Oceanography",
                "Water Resources",
            ]
            .contains(&result.main_tag.as_str())
                || result
                    .subtags
                    .iter()
                    .any(|subtag| subtag.contains("Water") || subtag.contains("Ocean"))
        }));

        let default_search_sections = vec![
            "prelims-gs1".to_string(),
            "mains-essay".to_string(),
            "mains-gs1".to_string(),
            "mains-gs2".to_string(),
            "mains-gs3".to_string(),
            "mains-gs4".to_string(),
        ];
        let default_river = search_questions_cached(
            &conn,
            &search_index,
            "river",
            Some(&default_search_sections),
        )
        .unwrap();
        assert_eq!(default_river.searched_questions, expected_default_questions);
        assert!(default_river.results.iter().all(|result| {
            !["mains-maths1", "mains-maths2", "prelims-csat"].contains(&result.section.as_str())
        }));

        let default_article_20 = search_questions_cached(
            &conn,
            &search_index,
            "Article 20",
            Some(&default_search_sections),
        )
        .unwrap();
        assert!(default_article_20
            .results
            .iter()
            .all(|result| result.section != "prelims-csat"));

        let math_sections = vec!["mains-maths1".to_string(), "mains-maths2".to_string()];
        let math_search = search_questions_cached(
            &conn,
            &search_index,
            "differential equation",
            Some(&math_sections),
        )
        .unwrap();
        assert_eq!(math_search.searched_questions, expected_math_questions);
        assert!(!math_search.results.is_empty());
        assert!(math_search.results.iter().all(|result| {
            ["mains-maths1", "mains-maths2"].contains(&result.section.as_str())
                && !["Mathematics", "Quantitative & Mathematical Methods"]
                    .contains(&result.main_tag.as_str())
                && !result.main_tag.trim().is_empty()
        }));

        let gs2_sections = vec!["mains-gs2".to_string()];
        let foreign_policy = search_questions_cached(
            &conn,
            &search_index,
            "energy security foreign policy Middle Eastern countries",
            Some(&gs2_sections),
        )
        .unwrap();
        let foreign_policy_question = foreign_policy
            .results
            .iter()
            .find(|result| result.question_id == "upsc_2025_mains_gs2_q19")
            .expect("known GS-II foreign-policy question should be retrievable");
        assert_eq!(foreign_policy_question.main_tag, "Foreign Policy");

        let gs3_sections = vec!["mains-gs3".to_string()];
        let security = search_questions_cached(
            &conn,
            &search_index,
            "internal security intelligence investigative agencies",
            Some(&gs3_sections),
        )
        .unwrap();
        let security_question = security
            .results
            .iter()
            .find(|result| result.question_id == "upsc_2023_mains_gs3_q19")
            .expect("known GS-III internal-security question should be retrievable");
        assert_eq!(security_question.main_tag, "External Security Threats");

        let cyber_security = search_questions_cached(
            &conn,
            &search_index,
            "cross-border cyber attacks",
            Some(&gs3_sections),
        )
        .unwrap();
        let cyber_security_question = cyber_security
            .results
            .iter()
            .find(|result| result.question_id == "upsc_2021_mains_gs3_q10")
            .expect("known GS-III cyber-security question should be retrievable");
        assert_eq!(cyber_security_question.main_tag, "Border Security");

        assert!(river.results.iter().any(|result| {
            let visible_text = format!(
                "{} {}",
                result.question,
                result
                    .options
                    .iter()
                    .map(|option| option.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            )
            .to_lowercase();
            !visible_text.contains("river")
                && [
                    "water",
                    "ocean",
                    "sea",
                    "lake",
                    "wetland",
                    "marine",
                    "groundwater",
                    "aquifer",
                ]
                .iter()
                .any(|concept| visible_text.contains(concept))
        }));

        for (query, related_terms) in [
            (
                "ocean",
                &["water", "river", "sea", "lake", "marine", "coast"][..],
            ),
            (
                "groundwater depletion",
                &[
                    "aquifer",
                    "water scarcity",
                    "irrigation",
                    "well",
                    "depleting",
                    "depletion",
                    "mitigate",
                ][..],
            ),
            (
                "protecting forests from climate change",
                &[
                    "biodiversity",
                    "ecosystem",
                    "conservation",
                    "wildlife",
                    "deforestation",
                    "afforestation",
                    "resources",
                    "environment",
                    "ecological",
                    "vegetation",
                ][..],
            ),
        ] {
            let related = search_questions_cached(&conn, &search_index, query, None).unwrap();
            assert!(!related.results.is_empty(), "no results for {query}");
            assert!(related
                .results
                .windows(2)
                .all(|pair| pair[0].similarity >= pair[1].similarity));
            assert!(
                related.results.iter().any(|result| {
                    let visible_text = format!(
                        "{} {}",
                        result.question,
                        result
                            .options
                            .iter()
                            .map(|option| option.text.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    )
                    .to_lowercase();
                    related_terms
                        .iter()
                        .any(|concept| visible_text.contains(concept))
                }),
                "no conceptually related result for {query}"
            );
        }
    }

    #[test]
    fn save_answer_rejects_completed_attempt() {
        // This test validates the TOCTOU transaction fix (#14):
        // Once an attempt is marked as 'completed', no further answers
        // should be accepted, even if the save_answer call happens
        // concurrently with submission.
        let mut conn = setup_conn();

        let bank_id = import_question_bank(&mut conn, &sample_bank()).unwrap();
        let attempt_id = crate::backend::db::attempt::create_test_attempt(
            &mut conn,
            &bank_id,
            crate::backend::types::TestMode::Test,
            None,
        )
        .unwrap();

        // Submit the test to move it to Completed.
        let questions =
            crate::backend::db::question_bank::fetch_questions_by_bank_id(&conn, &bank_id).unwrap();
        let responses =
            crate::backend::db::attempt::fetch_responses_by_attempt_id(&conn, &attempt_id).unwrap();
        let analysis = crate::backend::scoring::analyze_submission(
            &questions,
            &responses,
            &std::collections::HashMap::new(),
        );
        crate::backend::db::attempt::finalize_submission(
            &conn,
            &attempt_id,
            analysis.score,
            analysis.max_score,
            crate::backend::db::now_ms(),
            None,
        )
        .unwrap();

        // Now saving an answer should fail.
        let result = crate::backend::db::attempt::save_answer(
            &mut conn,
            &attempt_id,
            "q-1",
            Some(&serde_json::json!("a")),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().message().contains("not in progress"));
    }

    #[test]
    fn mark_breakdown_round_trips_through_question_bank_storage() {
        let mut conn = setup_conn();
        let mut bank = sample_bank();
        bank.questions[0].mark_breakdown = vec![
            QuestionMarkBreakdown {
                label: "a".to_string(),
                marks: 1.0,
                main_tag: None,
                subtags: Vec::new(),
                subparts: Vec::new(),
            },
            QuestionMarkBreakdown {
                label: "b".to_string(),
                marks: 1.0,
                main_tag: None,
                subtags: Vec::new(),
                subparts: Vec::new(),
            },
        ];
        let bank_id = import_question_bank(&mut conn, &bank).unwrap();
        let stored =
            crate::backend::db::question_bank::fetch_questions_by_bank_id(&conn, &bank_id).unwrap();
        assert_eq!(stored[0].mark_breakdown, bank.questions[0].mark_breakdown);
    }

    #[test]
    fn taxonomy_round_trips_through_question_bank_storage() {
        let mut conn = setup_conn();
        let mut bank = sample_bank();
        bank.questions[0].taxonomy = Some(QuestionTaxonomy {
            main_tag: MainTag::Nationalism,
            subtags: vec![Subtag::RevolutionaryMovement],
        });

        let bank_id = import_question_bank(&mut conn, &bank).unwrap();
        let stored =
            crate::backend::db::question_bank::fetch_questions_by_bank_id(&conn, &bank_id).unwrap();
        assert_eq!(stored[0].taxonomy, bank.questions[0].taxonomy);
    }

    #[test]
    fn taxonomy_refresh_preserves_attempts_answers_vectors_and_question_content() {
        let mut conn = setup_conn();
        let mut bank = sample_bank();
        bank.metadata
            .extra
            .insert("section".to_string(), serde_json::json!("mains-gs1"));
        bank.metadata
            .extra
            .insert("year".to_string(), serde_json::json!(2024));
        bank.metadata
            .extra
            .insert("paper".to_string(), serde_json::json!("GS1"));
        bank.metadata
            .extra
            .insert("taxonomyVersion".to_string(), serde_json::json!(2));
        bank.questions[0].mark_breakdown = vec![QuestionMarkBreakdown {
            label: "question".to_string(),
            marks: 2.0,
            main_tag: None,
            subtags: Vec::new(),
            subparts: Vec::new(),
        }];
        let bank_id = import_question_bank(&mut conn, &bank).unwrap();
        let attempt_id = crate::backend::db::attempt::create_test_attempt(
            &mut conn,
            &bank_id,
            crate::backend::types::TestMode::Practice,
            None,
        )
        .unwrap();
        crate::backend::db::attempt::save_answer(
            &mut conn,
            &attempt_id,
            "q-1",
            Some(&serde_json::json!("a")),
        )
        .unwrap();
        conn.execute(
            "UPDATE question_taxonomy
             SET main_tag = 'Polity & Constitution', subtags_json = '[]', taxonomy_version = 2
             WHERE question_id = 'q-1'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE search_documents
             SET main_tag = 'Polity & Constitution', subtags_text = ''
             WHERE question_id = 'q-1'",
            [],
        )
        .unwrap();

        let fingerprint_before: Vec<u8> = conn
            .query_row(
                "SELECT content_fingerprint FROM search_documents WHERE question_id = 'q-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let jobs_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM search_index_jobs", [], |row| {
                row.get(0)
            })
            .unwrap();

        let mut refreshed = bank.clone();
        refreshed.metadata.extra.insert(
            "taxonomyVersion".to_string(),
            serde_json::json!(crate::taxonomy::TAXONOMY_VERSION),
        );
        refreshed.questions[0].question = "This source text must not replace stored prose".into();
        refreshed.questions[0].taxonomy = Some(QuestionTaxonomy {
            main_tag: MainTag::Nationalism,
            subtags: vec![Subtag::RevolutionaryMovement],
        });
        refreshed.questions[0].mark_breakdown[0].main_tag = Some(MainTag::Nationalism);
        refreshed.questions[0].mark_breakdown[0].subtags = vec![Subtag::RevolutionaryMovement.id()];

        crate::backend::db::question_bank::refresh_question_bank_taxonomy(
            &mut conn, &bank_id, &refreshed,
        )
        .unwrap();

        let stored =
            crate::backend::db::question_bank::fetch_questions_by_bank_id(&conn, &bank_id).unwrap();
        assert_eq!(stored[0].question, "Sample question?");
        assert_eq!(stored[0].taxonomy, refreshed.questions[0].taxonomy);
        assert_eq!(
            stored[0].mark_breakdown[0].main_tag,
            Some(MainTag::Nationalism)
        );
        assert_eq!(stored[0].mark_breakdown[0].label, "question");
        assert_eq!(stored[0].mark_breakdown[0].marks, 2.0);

        let attempt = crate::backend::db::attempt::fetch_test_attempt(&conn, &attempt_id)
            .unwrap()
            .expect("attempt must survive taxonomy refresh");
        assert_eq!(attempt.bank_id, bank_id);
        let responses =
            crate::backend::db::attempt::fetch_responses_by_attempt_id(&conn, &attempt_id).unwrap();
        assert_eq!(responses[0].answer, Some(serde_json::json!("a")));

        let (taxonomy_version, fingerprint_after): (i64, Vec<u8>) = conn
            .query_row(
                "SELECT t.taxonomy_version, d.content_fingerprint
                 FROM question_taxonomy t
                 JOIN search_documents d ON d.question_id = t.question_id
                 WHERE t.question_id = 'q-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            taxonomy_version,
            i64::from(crate::taxonomy::TAXONOMY_VERSION)
        );
        assert_eq!(fingerprint_after, fingerprint_before);
        let jobs_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM search_index_jobs", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(jobs_after, jobs_before);
    }

    #[test]
    fn cleared_answer_shapes_remove_sparse_response_rows() {
        let mut conn = setup_conn();
        let bank_id = import_question_bank(&mut conn, &sample_bank()).unwrap();
        let attempt_id = crate::backend::db::attempt::create_test_attempt(
            &mut conn,
            &bank_id,
            crate::backend::types::TestMode::Test,
            None,
        )
        .unwrap();

        crate::backend::db::attempt::save_answer(
            &mut conn,
            &attempt_id,
            "q-1",
            Some(&serde_json::json!("a")),
        )
        .unwrap();
        assert_eq!(
            crate::backend::db::attempt::fetch_responses_by_attempt_id(&conn, &attempt_id)
                .unwrap()
                .len(),
            1
        );

        for cleared in [
            serde_json::json!([]),
            serde_json::json!("  "),
            serde_json::Value::Null,
        ] {
            crate::backend::db::attempt::save_answer(&mut conn, &attempt_id, "q-1", Some(&cleared))
                .unwrap();
            assert!(
                crate::backend::db::attempt::fetch_responses_by_attempt_id(&conn, &attempt_id)
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn toggle_flag_round_trip_cleans_up_flag_only_row() {
        let mut conn = setup_conn();
        let bank_id = import_question_bank(&mut conn, &sample_bank()).unwrap();
        let attempt_id = crate::backend::db::attempt::create_test_attempt(
            &mut conn,
            &bank_id,
            crate::backend::types::TestMode::Test,
            None,
        )
        .unwrap();

        assert!(crate::backend::db::attempt::toggle_flag(&conn, &attempt_id, "q-1").unwrap());
        assert_eq!(
            crate::backend::db::attempt::fetch_responses_by_attempt_id(&conn, &attempt_id)
                .unwrap()
                .len(),
            1
        );
        assert!(!crate::backend::db::attempt::toggle_flag(&conn, &attempt_id, "q-1").unwrap());
        assert!(
            crate::backend::db::attempt::fetch_responses_by_attempt_id(&conn, &attempt_id)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn deleting_bank_cascades_owned_rows_and_queues_vector_deletion() {
        let mut conn = setup_conn();
        let bank_id = import_question_bank(&mut conn, &sample_bank()).unwrap();
        let attempt_id = crate::backend::db::attempt::create_test_attempt(
            &mut conn,
            &bank_id,
            crate::backend::types::TestMode::Test,
            None,
        )
        .unwrap();
        crate::backend::db::attempt::save_answer(
            &mut conn,
            &attempt_id,
            "q-1",
            Some(&serde_json::json!("a")),
        )
        .unwrap();

        crate::backend::db::question_bank::delete_question_bank(&mut conn, &bank_id).unwrap();

        for table in [
            "question_banks",
            "questions",
            "test_attempts",
            "question_responses",
            "search_documents",
            "question_taxonomy",
        ] {
            let count: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "{table} retained rows after bank deletion");
        }
        let delete_jobs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM search_index_jobs
                 WHERE question_id = 'q-1' AND operation = 'delete' AND status = 'pending'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(delete_jobs, 1);
    }

    #[test]
    fn practice_feedback_requires_a_saved_answer_and_never_opens_for_test_mode() {
        let mut conn = setup_conn();
        let bank_id = import_question_bank(&mut conn, &sample_bank()).unwrap();
        let practice_id = crate::backend::db::attempt::create_test_attempt(
            &mut conn,
            &bank_id,
            crate::backend::types::TestMode::Practice,
            None,
        )
        .unwrap();
        assert!(
            crate::backend::db::attempt::fetch_practice_question_feedback(
                &conn,
                &practice_id,
                "q-1"
            )
            .unwrap()
            .is_none()
        );

        crate::backend::db::attempt::save_answer(
            &mut conn,
            &practice_id,
            "q-1",
            Some(&serde_json::json!("b")),
        )
        .unwrap();
        let feedback = crate::backend::db::attempt::fetch_practice_question_feedback(
            &conn,
            &practice_id,
            "q-1",
        )
        .unwrap()
        .unwrap();
        assert_eq!(feedback.correct_answers, vec!["a"]);
        assert_eq!(feedback.explanation, "Because");

        let test_id = crate::backend::db::attempt::create_test_attempt(
            &mut conn,
            &bank_id,
            crate::backend::types::TestMode::Test,
            None,
        )
        .unwrap();
        crate::backend::db::attempt::save_answer(
            &mut conn,
            &test_id,
            "q-1",
            Some(&serde_json::json!("b")),
        )
        .unwrap();
        assert!(
            crate::backend::db::attempt::fetch_practice_question_feedback(&conn, &test_id, "q-1")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn v4_repairs_legacy_temporary_foreign_key_targets() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = OFF;
             CREATE TABLE schema_version (version INTEGER NOT NULL);
             INSERT INTO schema_version VALUES (3);
             CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE question_banks (
                id TEXT PRIMARY KEY, name TEXT NOT NULL, exam TEXT NOT NULL,
                metadata TEXT NOT NULL, total_questions INTEGER NOT NULL,
                difficulty TEXT NOT NULL, default_duration INTEGER NOT NULL,
                imported_at INTEGER NOT NULL
             );
             CREATE TABLE questions (
                id TEXT PRIMARY KEY, bank_id TEXT NOT NULL, type TEXT NOT NULL,
                question TEXT NOT NULL, options TEXT, correct_answers TEXT NOT NULL,
                explanation TEXT NOT NULL DEFAULT '', marks REAL NOT NULL,
                negative_marks REAL NOT NULL DEFAULT 0,
                negative_marks_unanswered REAL NOT NULL DEFAULT 0,
                time_estimate INTEGER, difficulty TEXT, tags TEXT,
                FOREIGN KEY (bank_id) REFERENCES question_banks_v3(id) ON DELETE CASCADE
             );
             CREATE TABLE test_attempts (
                id TEXT PRIMARY KEY, bank_id TEXT NOT NULL, mode TEXT NOT NULL,
                status TEXT NOT NULL, duration INTEGER NOT NULL,
                time_remaining INTEGER NOT NULL, started_at INTEGER NOT NULL,
                completed_at INTEGER, score REAL, max_score REAL,
                FOREIGN KEY (bank_id) REFERENCES question_banks_v3(id) ON DELETE CASCADE
             );
             CREATE TABLE question_responses (
                attempt_id TEXT NOT NULL, question_id TEXT NOT NULL, answer TEXT,
                is_flagged INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (attempt_id, question_id),
                FOREIGN KEY (attempt_id) REFERENCES test_attempts_v3(id) ON DELETE CASCADE,
                FOREIGN KEY (question_id) REFERENCES questions_v3(id) ON DELETE CASCADE
             ) WITHOUT ROWID;
             INSERT INTO question_banks VALUES ('b', 'Bank', 'UPSC', '{}', 1, 'medium', 60, 1);
             INSERT INTO questions VALUES ('q', 'b', 'single-choice', 'Question?', NULL, '[\"a\"]', '', 1, 0, 0, NULL, NULL, NULL);
             INSERT INTO test_attempts VALUES ('a', 'b', 'test', 'in_progress', 60, 60, 1, NULL, NULL, NULL);
             INSERT INTO question_responses VALUES ('a', 'q', '\"a\"', 1);"
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        for table in ["questions", "test_attempts", "question_responses"] {
            let mut statement = conn
                .prepare(&format!("PRAGMA foreign_key_list({table})"))
                .unwrap();
            let targets = statement
                .query_map([], |row| row.get::<_, String>(2))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(targets.iter().all(|target| !target.ends_with("_v3")));
        }

        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn.execute("DELETE FROM question_banks WHERE id = 'b'", [])
            .unwrap();
        for table in ["questions", "test_attempts", "question_responses"] {
            let count: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "{table} did not cascade after repair");
        }
    }

    #[test]
    fn attempt_state_transitions_are_enforced() {
        let mut conn = setup_conn();
        let bank_id = import_question_bank(&mut conn, &sample_bank()).unwrap();
        let attempt_id = crate::backend::db::attempt::create_test_attempt(
            &mut conn,
            &bank_id,
            crate::backend::types::TestMode::Test,
            None,
        )
        .unwrap();

        assert!(crate::backend::db::attempt::toggle_flag(&conn, &attempt_id, "q-1").unwrap());
        crate::backend::db::attempt::update_time_remaining(&conn, &attempt_id, 500).unwrap();
        crate::backend::db::attempt::update_time_remaining(&conn, &attempt_id, 550).unwrap();
        assert_eq!(
            crate::backend::db::attempt::fetch_test_attempt(&conn, &attempt_id)
                .unwrap()
                .unwrap()
                .time_remaining,
            500
        );
        crate::backend::db::attempt::pause_test(&conn, &attempt_id, 480).unwrap();
        assert!(crate::backend::db::attempt::pause_test(&conn, &attempt_id, 499).is_err());
        assert!(crate::backend::db::attempt::finalize_submission(
            &conn,
            &attempt_id,
            0.0,
            2.0,
            crate::backend::db::now_ms(),
            None,
        )
        .is_err());
        assert!(crate::backend::db::attempt::toggle_flag(&conn, &attempt_id, "q-1").is_err());
        assert!(
            crate::backend::db::attempt::fetch_responses_by_attempt_id(&conn, &attempt_id).unwrap()
                [0]
            .is_flagged
        );
        crate::backend::db::attempt::resume_test(&conn, &attempt_id).unwrap();
        assert!(!crate::backend::db::attempt::toggle_flag(&conn, &attempt_id, "q-1").unwrap());
        crate::backend::db::attempt::finalize_submission(
            &conn,
            &attempt_id,
            0.0,
            2.0,
            crate::backend::db::now_ms(),
            Some(520),
        )
        .unwrap();
        let submitted = crate::backend::db::attempt::fetch_test_attempt(&conn, &attempt_id)
            .unwrap()
            .unwrap();
        assert_eq!(submitted.time_remaining, 480);
        assert!(crate::backend::db::attempt::finalize_submission(
            &conn,
            &attempt_id,
            0.0,
            2.0,
            crate::backend::db::now_ms(),
            None,
        )
        .is_err());
        assert!(
            crate::backend::db::attempt::update_time_remaining(&conn, &attempt_id, 10).is_err()
        );
        assert!(crate::backend::db::attempt::toggle_flag(&conn, &attempt_id, "q-1").unwrap());
        assert!(!crate::backend::db::attempt::toggle_flag(&conn, &attempt_id, "q-1").unwrap());
    }

    #[test]
    fn invalid_duration_and_cross_bank_question_ids_are_rejected_cleanly() {
        let mut conn = setup_conn();
        let bank = sample_bank();
        let bank_id = import_question_bank(&mut conn, &bank).unwrap();
        assert!(crate::backend::db::attempt::create_test_attempt(
            &mut conn,
            &bank_id,
            crate::backend::types::TestMode::Test,
            Some(0),
        )
        .is_err());

        let conflicts =
            crate::backend::db::question_bank::question_id_conflicts(&conn, &bank).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].path, "questions[0].id");
    }
}
