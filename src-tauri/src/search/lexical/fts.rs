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
        if !hits.is_empty() {
            return Ok(hits);
        }

        // Typo fallback only for single-word queries (e.g. "parliment" -> "parl*")
        if query.terms().len() == 1 {
            let t = &query.terms()[0];
            let char_count = t.chars().count();
            if char_count >= 4 {
                let root: String = t.chars().take(4).collect();
                let fallback_expr = format!("\"{}\"*", root.replace('"', "\"\""));
                let fallback_hits =
                    Self::search_match_str(conn, &fallback_expr, filters, limit, false)?;
                if !fallback_hits.is_empty() {
                    return Ok(fallback_hits);
                }
            }
        }

        Ok(Vec::new())
    }

    fn search_match_str(
        conn: &Connection,
        match_expr: &str,
        filters: &SearchFilter,
        limit: usize,
        relaxed: bool,
    ) -> Result<Vec<LexicalHit>> {
        let mut sql = String::from(
            "SELECT
                d.search_id,
                d.question_id,
                -bm25(question_fts) AS score
            FROM question_fts f
            JOIN search_documents d ON d.search_id = f.rowid
            WHERE question_fts MATCH ?1",
        );

        let mut param_idx = 2;
        let mut filter_clauses = Vec::new();

        // Optional section filter
        if !filters.sections.is_empty() {
            let placeholders: Vec<String> = (0..filters.sections.len())
                .map(|i| format!("?{}", param_idx + i))
                .collect();
            filter_clauses.push(format!("d.section IN ({})", placeholders.join(",")));
            param_idx += filters.sections.len();
        }

        // Optional stage filter
        if !filters.stages.is_empty() {
            let placeholders: Vec<String> = (0..filters.stages.len())
                .map(|i| format!("?{}", param_idx + i))
                .collect();
            filter_clauses.push(format!("d.stage IN ({})", placeholders.join(",")));
            param_idx += filters.stages.len();
        }

        // Optional paper filter
        if !filters.papers.is_empty() {
            let placeholders: Vec<String> = (0..filters.papers.len())
                .map(|i| format!("?{}", param_idx + i))
                .collect();
            filter_clauses.push(format!("d.paper IN ({})", placeholders.join(",")));
            param_idx += filters.papers.len();
        }

        // Optional banks filter
        if !filters.banks.is_empty() {
            let placeholders: Vec<String> = (0..filters.banks.len())
                .map(|i| format!("?{}", param_idx + i))
                .collect();
            filter_clauses.push(format!("d.bank_id IN ({})", placeholders.join(",")));
            param_idx += filters.banks.len();
        }

        // Optional year range filter
        if filters.years.is_some() {
            filter_clauses.push(format!(
                "d.year >= ?{} AND d.year <= ?{}",
                param_idx,
                param_idx + 1
            ));
            param_idx += 2;
        }

        if !filters.tags.is_empty() {
            let mut clauses = Vec::new();
            for tag in &filters.tags {
                if let Some(alias) = crate::taxonomy::legacy_main_tag_alias(tag) {
                    let mut alias_clauses = Vec::new();
                    if !alias.main_tags.is_empty() {
                        let placeholders = (0..alias.main_tags.len())
                            .map(|offset| format!("?{}", param_idx + offset))
                            .collect::<Vec<_>>()
                            .join(",");
                        alias_clauses.push(format!("d.main_tag IN ({placeholders})"));
                        param_idx += alias.main_tags.len();
                    }
                    if !alias.sections.is_empty() {
                        let placeholders = (0..alias.sections.len())
                            .map(|offset| format!("?{}", param_idx + offset))
                            .collect::<Vec<_>>()
                            .join(",");
                        alias_clauses.push(format!("d.section IN ({placeholders})"));
                        param_idx += alias.sections.len();
                    }
                    clauses.push(format!("({})", alias_clauses.join(" OR ")));
                } else {
                    let parameter = param_idx;
                    clauses.push(format!(
                        "(d.main_tag = ?{parameter} OR EXISTS (\
                         SELECT 1 FROM question_taxonomy t, json_each(t.subtags_json) j \
                         WHERE t.question_id = d.question_id AND j.value = ?{parameter}))"
                    ));
                    param_idx += 1;
                }
            }
            filter_clauses.push(format!("({})", clauses.join(" OR ")));
        }

        for clause in filter_clauses {
            sql.push_str(" AND ");
            sql.push_str(&clause);
        }

        sql.push_str(" ORDER BY score DESC LIMIT ?");
        sql.push_str(&param_idx.to_string());

        let mut stmt = conn.prepare(&sql)?;

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        params_vec.push(Box::new(match_expr.to_string()));

        for sec in &filters.sections {
            params_vec.push(Box::new(sec.clone()));
        }
        for stg in &filters.stages {
            params_vec.push(Box::new(stg.clone()));
        }
        for ppr in &filters.papers {
            params_vec.push(Box::new(ppr.clone()));
        }
        for bnk in &filters.banks {
            params_vec.push(Box::new(bnk.clone()));
        }
        if let Some((min_year, max_year)) = filters.years {
            params_vec.push(Box::new(min_year as i64));
            params_vec.push(Box::new(max_year as i64));
        }
        for tag in &filters.tags {
            if let Some(alias) = crate::taxonomy::legacy_main_tag_alias(tag) {
                params_vec.extend(
                    alias
                        .main_tags
                        .iter()
                        .chain(alias.sections.iter())
                        .cloned()
                        .map(|value| Box::new(value) as Box<dyn rusqlite::ToSql>),
                );
            } else {
                params_vec.push(Box::new(tag.clone()));
            }
        }
        params_vec.push(Box::new(limit as i64));

        let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();
        let mut rows = stmt.query(param_refs.as_slice())?;
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
    }
}
