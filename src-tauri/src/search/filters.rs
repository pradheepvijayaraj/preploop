//! Search filters for scoping queries by section, stage, paper, year, bank, tag.

use std::collections::HashSet;
use std::sync::Arc;

use rusqlite::types::Value;

/// Scoping filters applied during both FTS and vector retrieval.
#[derive(Debug, Default, Clone)]
pub struct SearchFilter {
    pub sections: Vec<String>,
    pub stages: Vec<String>,
    pub papers: Vec<String>,
    pub years: Option<(u16, u16)>,
    pub banks: Vec<String>,
    pub tags: Vec<String>,
    /// Resolved once from SQLite for semantic scanning. This avoids a SQL
    /// lookup per vector while still applying filters inside the mmap scan.
    pub(crate) allowed_search_ids: Option<Arc<HashSet<u64>>>,
}

impl SearchFilter {
    pub fn has_constraints(&self) -> bool {
        !self.sections.is_empty()
            || !self.stages.is_empty()
            || !self.papers.is_empty()
            || self.years.is_some()
            || !self.banks.is_empty()
            || !self.tags.is_empty()
    }

    pub fn allows_search_id(&self, search_id: u64) -> bool {
        self.allowed_search_ids
            .as_ref()
            .map_or(true, |allowed| allowed.contains(&search_id))
    }

    /// Append the same predicates for lexical retrieval and vector eligibility.
    /// The caller aliases search_documents as `d`; values remain SQL parameters.
    pub(crate) fn append_sql(
        &self,
        sql: &mut String,
        values: &mut Vec<Value>,
    ) -> rusqlite::Result<()> {
        for (column, items) in [
            ("d.section", &self.sections),
            ("d.stage", &self.stages),
            ("d.paper", &self.papers),
            ("d.bank_id", &self.banks),
        ] {
            if let Some(clause) = in_clause(column, items, values) {
                sql.push_str(&format!(" AND {clause}"));
            }
        }
        if let Some((min_year, max_year)) = self.years {
            let first = values.len() + 1;
            sql.push_str(&format!(" AND d.year BETWEEN ?{first} AND ?{}", first + 1));
            values.extend([
                Value::Integer(min_year.into()),
                Value::Integer(max_year.into()),
            ]);
        }
        if !self.tags.is_empty() {
            let mut clauses = Vec::new();
            for tag in &self.tags {
                if let Some(alias) = crate::taxonomy::legacy_main_tag_alias(tag) {
                    let alias_clauses = [
                        in_clause("d.main_tag", &alias.main_tags, values),
                        in_clause("d.section", &alias.sections, values),
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>();
                    clauses.push(format!("({})", alias_clauses.join(" OR ")));
                } else {
                    let parameter = values.len() + 1;
                    clauses.push(format!(
                        "(d.main_tag = ?{parameter} OR EXISTS (\
                         SELECT 1 FROM question_taxonomy t, json_each(t.subtags_json) j \
                         WHERE t.question_id = d.question_id AND j.value = ?{parameter}))"
                    ));
                    values.push(Value::Text(tag.clone()));
                }
            }
            sql.push_str(&format!(" AND ({})", clauses.join(" OR ")));
        }
        if let Some(ids) = &self.allowed_search_ids {
            let parameter = values.len() + 1;
            sql.push_str(&format!(
                " AND d.search_id IN (SELECT value FROM json_each(?{parameter}))"
            ));
            let encoded = serde_json::to_string(ids.as_ref())
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            values.push(Value::Text(encoded));
        }
        Ok(())
    }
}

fn in_clause(column: &str, items: &[String], values: &mut Vec<Value>) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let start = values.len() + 1;
    let placeholders = (0..items.len())
        .map(|offset| format!("?{}", start + offset))
        .collect::<Vec<_>>()
        .join(",");
    values.extend(items.iter().cloned().map(Value::Text));
    Some(format!("{column} IN ({placeholders})"))
}
