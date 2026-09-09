//! SQLite FTS5 lexical retrieval engine.

use rusqlite::{Connection, Result};
use std::collections::HashSet;

use super::query_builder::CompiledFtsQuery;
use crate::search::filters::SearchFilter;

/// A candidate hit from lexical FTS5 retrieval.
#[derive(Debug, Clone, PartialEq)]
pub struct LexicalHit {
    /// SQLite row ID in `search_documents`.
    pub search_id: i64,
    /// Canonical question UUID.
    pub question_id: String,
    /// Raw BM25 score (higher is more relevant; negated from SQLite's default).
    pub score: f32,
    /// Whether this candidate came from the broad meaningful-term fallback.
    pub relaxed: bool,
}

/// Lexical search service operating on SQLite FTS5.
pub struct LexicalSearch;

impl LexicalSearch {
    /// Quoted phrases must match surface text, including during semantic fallback.
    pub fn required_phrase_ids(
        conn: &Connection,
        query: &CompiledFtsQuery,
        filters: &SearchFilter,
    ) -> Result<Option<HashSet<u64>>> {
        query
            .required_phrases_match()
            .map(|expression| {
                Self::search_index(
                    conn,
                    "question_literal_fts",
                    &expression,
                    filters,
                    i64::MAX as usize,
                    false,
                )
                .map(|hits| hits.into_iter().map(|hit| hit.search_id as u64).collect())
            })
            .transpose()
    }

    /// Executes an FTS5 search using the compiled match expression and optional structured filters.
    pub fn search(
        conn: &Connection,
        query: &CompiledFtsQuery,
        filters: &SearchFilter,
        limit: usize,
    ) -> Result<Vec<LexicalHit>> {
        if query.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let mut hits = Self::search_match_str(conn, query.as_fts_match(), filters, limit, false)?;

        // The strict expression uses implicit AND semantics. Preserve those
        // strongest matches first, then fill the remaining candidate budget
        // from a meaningful-term OR query so descriptive searches do not
        // collapse to zero lexical candidates.
        if hits.len() < limit {
            if let Some(relaxed_expr) = query.as_relaxed_fts_match() {
                let relaxed = Self::search_match_str(conn, &relaxed_expr, filters, limit, true)?;
                let mut seen = hits.iter().map(|hit| hit.search_id).collect::<HashSet<_>>();
                for hit in relaxed {
                    if seen.insert(hit.search_id) {
                        hits.push(hit);
                        if hits.len() == limit {
                            break;
                        }
                    }
                }
            }
        }
        Ok(hits)
    }

    /// Use SQLite's own tokenizers when deciding whether a term is a typo.
    /// The Porter vocabulary contains stems, not the user's surface words.
    pub fn term_exists(conn: &Connection, expression: &str) -> Result<bool> {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM question_literal_fts WHERE question_literal_fts MATCH ?1)
                 OR EXISTS(SELECT 1 FROM question_fts WHERE question_fts MATCH ?1)",
            [expression],
            |row| row.get(0),
        )
    }

    fn search_match_str(
        conn: &Connection,
        match_expr: &str,
        filters: &SearchFilter,
        limit: usize,
        relaxed: bool,
    ) -> Result<Vec<LexicalHit>> {
        // Keep the established stemmed ordering, then recover literal prefixes
        // that stemming erased. Exact phrase boosts rank these after fusion.
        let mut hits =
            Self::search_index(conn, "question_fts", match_expr, filters, limit, relaxed)?;
        let literal = Self::search_index(
            conn,
            "question_literal_fts",
            match_expr,
            filters,
            limit,
            relaxed,
        )?;
        let mut seen = hits.iter().map(|hit| hit.search_id).collect::<HashSet<_>>();
        for hit in literal {
            if seen.insert(hit.search_id) {
                hits.push(hit);
            }
        }
        Ok(hits)
    }

    fn search_index(
        conn: &Connection,
        index: &str,
        match_expr: &str,
        filters: &SearchFilter,
        limit: usize,
        relaxed: bool,
    ) -> Result<Vec<LexicalHit>> {
        let mut sql = format!(
            "SELECT
                d.search_id,
                d.question_id,
                -bm25({index}) AS score
            FROM {index} f
            JOIN search_documents d ON d.search_id = f.rowid
            WHERE {index} MATCH ?1",
        );

        let mut values = vec![rusqlite::types::Value::Text(match_expr.to_string())];
        filters.append_sql(&mut sql, &mut values)?;
        sql.push_str(&format!(
            " ORDER BY score DESC, d.search_id ASC LIMIT ?{}",
            values.len() + 1
        ));
        values.push(rusqlite::types::Value::Integer(
            i64::try_from(limit).unwrap_or(i64::MAX),
        ));

        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(values.iter()))?;
        let mut hits = Vec::new();

        while let Some(row) = rows.next()? {
            hits.push(LexicalHit {
                search_id: row.get(0)?,
                question_id: row.get(1)?,
                score: row.get::<_, f64>(2)? as f32,
                relaxed,
            });
        }

        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::db::schema::run_migrations;
    use crate::search::lexical::query_builder::FtsQueryBuilder;

    #[test]
    fn bundled_corpus_keeps_literal_matches_through_typing_and_punctuation() {
        // Exercise the production tokenizer against one source phrase per
        // eligible question, including every prefix of its second word.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE question_fts USING fts5(question, tokenize='porter unicode61', prefix='2 3'); CREATE VIRTUAL TABLE question_literal_fts USING fts5(question, tokenize='unicode61', prefix='1 2 3');",
        ).unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../static/upsc");
        let catalog: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join("catalog.json")).unwrap())
                .unwrap();
        let mut cases = Vec::new();
        let mut documents = 0;
        for paper in catalog["papers"].as_array().unwrap() {
            let bank: crate::backend::types::QuestionBank = serde_json::from_str(
                &std::fs::read_to_string(root.join(paper["path"].as_str().unwrap())).unwrap(),
            )
            .unwrap();
            for question in bank.questions {
                documents += 1;
                conn.execute(
                    "INSERT INTO question_fts(rowid, question) VALUES (?1, ?2)",
                    rusqlite::params![documents, question.question],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO question_literal_fts(rowid, question) VALUES (?1, ?2)",
                    rusqlite::params![documents, question.question],
                )
                .unwrap();
                let words = question
                    .question
                    .split(|c: char| !c.is_alphanumeric())
                    .collect::<Vec<_>>();
                if let Some(pair) = words.windows(2).find(|pair| {
                    pair.iter().all(|word| {
                        word.len() >= 4 && word.chars().all(|c| c.is_ascii_alphabetic())
                    })
                }) {
                    cases.push((
                        documents,
                        question.id,
                        pair[0].to_string(),
                        pair[1].to_string(),
                    ));
                }
            }
        }
        let mut lookup = conn.prepare(
            "SELECT EXISTS(SELECT 1 FROM question_fts WHERE question_fts MATCH ?1 AND rowid = ?2) OR EXISTS(SELECT 1 FROM question_literal_fts WHERE question_literal_fts MATCH ?1 AND rowid = ?2)",
        ).unwrap();
        let mut tested = 0;
        let mut failures = Vec::new();
        for (rowid, id, first, second) in &cases {
            let mut queries = (1..=second.len())
                .map(|length| format!("{first} {}", &second[..length]))
                .collect::<Vec<_>>();
            queries.extend(
                ["/", "—", ",", "-", "_", "\t", "  "]
                    .map(|separator| format!("{first}{separator}{second}")),
            );
            for query in queries {
                tested += 1;
                let compiled = FtsQueryBuilder::build(&query).unwrap();
                let found: bool = lookup
                    .query_row(rusqlite::params![compiled.as_fts_match(), rowid], |row| {
                        row.get(0)
                    })
                    .unwrap();
                if !found {
                    failures.push(format!("{id}: {query}"));
                }
            }
        }
        println!("Literal typing audit: {documents} documents, {} sampled phrases, {tested} queries, {} misses", cases.len(), failures.len());
        assert!(cases.len() > 4_000);
        assert!(
            failures.is_empty(),
            "{} misses; examples: {:?}",
            failures.len(),
            &failures[..failures.len().min(20)]
        );
    }

    #[test]
    fn test_fts5_lexical_search_flow() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        // Insert parent question banks
        conn.execute(
            "INSERT INTO question_banks (id, name, exam, metadata, total_questions, difficulty, default_duration, imported_at)
             VALUES ('bank-1', 'UPSC 2023', 'UPSC', '{}', 2, 'medium', 7200, 1000),
                    ('bank-2', 'UPSC 2022', 'UPSC', '{}', 1, 'medium', 7200, 1000)",
            [],
        ).unwrap();

        // Insert parent questions
        conn.execute(
            "INSERT INTO questions (id, bank_id, type, question, correct_answers, marks)
             VALUES ('q1', 'bank-1', 'single', 'Which Article deals with Constitutional Remedies?', '[]', 2.0),
                    ('q2', 'bank-1', 'single', 'The FRBM Act was enacted in which year?', '[]', 2.0),
                    ('q3', 'bank-2', 'single', 'Kesavananda Bharati case established the basic structure doctrine.', '[]', 2.0)",
            [],
        ).unwrap();

        // Insert mock search documents
        conn.execute(
            "INSERT INTO search_documents (
                question_id, question, options_text, main_tag, subtags_text,
                bank_id, bank_name, year, stage, paper, section, content_fingerprint
            ) VALUES
            ('q1', 'Which Article of the Constitution deals with Constitutional Remedies?', '(A) Article 30 (B) Article 32 (C) Article 226', 'Polity', 'Fundamental Rights', 'bank-1', 'UPSC 2023', 2023, 'prelims', 'GS-1', 'polity', X'0102030405060708'),
            ('q2', 'The FRBM Act was enacted in which year?', '(A) 2000 (B) 2003 (C) 2005', 'Economy', 'Fiscal Policy', 'bank-1', 'UPSC 2023', 2023, 'prelims', 'GS-1', 'economy', X'0102030405060709'),
            ('q3', 'Kesavananda Bharati case established the basic structure doctrine.', '(A) Yes (B) No', 'Polity', 'Judiciary', 'bank-2', 'UPSC 2022', 2022, 'prelims', 'GS-1', 'polity', X'0102030405060710')",
            [],
        ).unwrap();

        // Query 1: Article 32
        let q1 = FtsQueryBuilder::build("Article 32").unwrap();
        let hits = LexicalSearch::search(&conn, &q1, &SearchFilter::default(), 10).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].question_id, "q1");

        // Query 2: FRBM Act
        let q2 = FtsQueryBuilder::build("FRBM Act").unwrap();
        let hits2 = LexicalSearch::search(&conn, &q2, &SearchFilter::default(), 10).unwrap();
        assert_eq!(hits2.len(), 1);
        assert_eq!(hits2[0].question_id, "q2");

        // Query 3: With section filter
        let filter = SearchFilter {
            sections: vec!["economy".to_string()],
            ..Default::default()
        };
        let q3 = FtsQueryBuilder::build("Article").unwrap();
        let hits3 = LexicalSearch::search(&conn, &q3, &filter, 10).unwrap();
        assert!(hits3.is_empty(), "Economy section has no Article questions");

        conn.execute(
            "UPDATE search_documents SET main_tag = 'Constitution' WHERE question_id = 'q1'",
            [],
        )
        .unwrap();
        let legacy_tag_filter = SearchFilter {
            tags: vec!["Polity & Constitution".to_string()],
            ..Default::default()
        };
        let hits4 = LexicalSearch::search(&conn, &q1, &legacy_tag_filter, 10).unwrap();
        assert_eq!(hits4.len(), 1);
        assert_eq!(hits4[0].question_id, "q1");

        // Lexical retrieval does not broaden arbitrary typos to four letters.
        // Vocabulary-based correction is tested through SearchService.
        let typo = FtsQueryBuilder::build("constituton").unwrap();
        let typo_hits = LexicalSearch::search(&conn, &typo, &SearchFilter::default(), 10).unwrap();
        assert!(typo_hits.is_empty());

        let numeric = FtsQueryBuilder::build("20230").unwrap();
        let numeric_hits =
            LexicalSearch::search(&conn, &numeric, &SearchFilter::default(), 10).unwrap();
        assert!(numeric_hits.is_empty());
    }
}
