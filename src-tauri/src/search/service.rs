//! SearchService — central orchestrator for hybrid lexical + semantic search.

use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::search::embedding::engine::EmbeddingEngine;
use crate::search::lexical::fts::{LexicalHit, LexicalSearch};
use crate::search::lexical::query_builder::FtsQueryBuilder;
use crate::search::ranking::boosts::apply_exact_match_boosts;
use crate::search::ranking::rrf::{reciprocal_rank_fusion, SearchTuning};
use crate::search::ranking::topic_evidence::TopicEvidence;
use crate::search::request::{SearchOptions, SearchRequest};
use crate::search::response::{MatchStrength, SearchHit, SearchResponse, SemanticStatus};
use crate::search::vector::traits::VectorSearch;

/// Central search coordinator combining SQLite FTS5 lexical search and dense vector search.
pub struct SearchService {
    embedding_engine: Option<Arc<dyn EmbeddingEngine>>,
    vector_index: Option<Arc<dyn VectorSearch>>,
    vocabulary: OnceLock<SearchVocabulary>,
    topic_evidence: OnceLock<TopicEvidence>,
    tuning: SearchTuning,
}

struct SearchVocabulary {
    terms_by_length: HashMap<usize, Vec<(String, usize)>>,
    exact: std::collections::HashSet<String>,
}

struct HydratedDoc {
    question_id: String,
    question: String,
    options: Vec<crate::backend::types::QuestionOption>,
    bank_id: String,
    bank_name: String,
    year: Option<i64>,
    stage: String,
    paper: String,
    section: String,
    main_tag: String,
    subtags: Vec<String>,
}

// Keep well below SQLite's compile-time variable limit. Taxonomy searches can
// legitimately return the whole corpus, so hydration must always be batched.
const HYDRATION_BATCH_SIZE: usize = 500;

impl SearchService {
    /// Creates a new `SearchService`.
    pub fn new(
        embedding_engine: Option<Arc<dyn EmbeddingEngine>>,
        vector_index: Option<Arc<dyn VectorSearch>>,
    ) -> Self {
        Self {
            embedding_engine,
            vector_index,
            vocabulary: OnceLock::new(),
            topic_evidence: OnceLock::new(),
            tuning: SearchTuning::default(),
        }
    }

    /// Load the embedding model and run one representative inference.
    ///
    /// This is intentionally separate from `search`: blank searches return
    /// before inference, while a visible query must retain its real ranking.
    pub fn warm_embedding(&self) -> Result<(), String> {
        let engine = self
            .embedding_engine
            .as_ref()
            .ok_or_else(|| "Granite embedding model is unavailable".to_string())?;
        engine
            .embed_query("UPSC Civil Services Examination")
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// Primary search entry point.
    pub fn search(
        &self,
        conn: &Connection,
        request: &SearchRequest,
    ) -> Result<SearchResponse, String> {
        self.search_with_options(conn, request, &SearchOptions::default())
    }

    pub fn search_with_options(
        &self,
        conn: &Connection,
        request: &SearchRequest,
        options: &SearchOptions,
    ) -> Result<SearchResponse, String> {
        options.cancellation.check()?;
        // 1. Return immediately on blank / whitespace query (no model load, 0 ms latency)
        if request.is_empty() {
            return Ok(SearchResponse::default());
        }

        let limit = if request.limit > 0 {
            request.limit
        } else {
            self.tuning.final_limit
        };
        let candidate_limit = limit.max(self.tuning.lexical_fusion_window);
        let mut effective_filters = request.filters.clone();
        if effective_filters.has_constraints() {
            // Resolve the intersection once. Subsequent FTS/correction lookups
            // and the vector scan need only this shared eligibility set.
            effective_filters = crate::search::filters::SearchFilter {
                allowed_search_ids: Some(Arc::new(resolve_allowed_search_ids(
                    conn,
                    &effective_filters,
                )?)),
                ..Default::default()
            };
        }
        if effective_filters
            .allowed_search_ids
            .as_ref()
            .is_some_and(|ids| ids.is_empty())
        {
            return Ok(SearchResponse::default());
        }

        let Some(compiled) = FtsQueryBuilder::build(&request.query) else {
            return Ok(SearchResponse::default());
        };
        let required_ids = LexicalSearch::required_phrase_ids(conn, &compiled, &effective_filters)
            .map_err(|error| error.to_string())?;
        if let Some(ids) = &required_ids {
            let allowed = ids
                .iter()
                .copied()
                .filter(|id| effective_filters.allows_search_id(*id))
                .collect();
            effective_filters.allowed_search_ids = Some(Arc::new(allowed));
            if ids.is_empty() {
                return Ok(SearchResponse::default());
            }
        }

        // Taxonomy contributes candidates; it never replaces literal retrieval.
        let mut taxonomy_hits = Vec::new();
        let trimmed_query = request.query.trim();
        let normalized_query = trimmed_query.to_lowercase().replace(" and ", " & ");

        let matched_main_tag: Option<String> = conn
            .query_row(
                "SELECT main_tag FROM question_taxonomy
                 WHERE LOWER(main_tag) = ?1
                    OR LOWER(main_tag) = ?2
                    OR LOWER(REPLACE(main_tag, ' & ', ' and ')) = ?1
                 LIMIT 1",
                rusqlite::params![trimmed_query.to_lowercase(), normalized_query],
                |r| r.get(0),
            )
            .ok();

        let is_subtag: bool = conn
            .query_row(
                "SELECT 1 FROM question_taxonomy t, json_each(t.subtags_json) j
                 WHERE LOWER(j.value) = LOWER(?1) LIMIT 1",
                rusqlite::params![trimmed_query],
                |_| Ok(true),
            )
            .unwrap_or(false);

        let exact_taxonomy_scope =
            matched_main_tag
                .map(|tag| (vec![tag], Vec::new()))
                .or_else(|| {
                    if is_subtag {
                        None
                    } else {
                        crate::taxonomy::legacy_main_tag_alias(trimmed_query)
                            .map(|alias| (alias.main_tags.clone(), alias.sections.clone()))
                    }
                });

        if let Some((target_tags, target_sections)) = exact_taxonomy_scope {
            let mut sql = String::from(
                "SELECT d.search_id, d.question_id
                 FROM search_documents d
                 JOIN question_taxonomy t ON t.question_id = d.question_id
                 WHERE (",
            );
            let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            let mut scope_clauses = Vec::new();
            if !target_tags.is_empty() {
                let start = params_vec.len() + 1;
                let placeholders = (0..target_tags.len())
                    .map(|offset| format!("?{}", start + offset))
                    .collect::<Vec<_>>()
                    .join(",");
                scope_clauses.push(format!("t.main_tag IN ({placeholders})"));
                params_vec.extend(
                    target_tags
                        .into_iter()
                        .map(|tag| Box::new(tag) as Box<dyn rusqlite::ToSql>),
                );
            }
            if !target_sections.is_empty() {
                let start = params_vec.len() + 1;
                let placeholders = (0..target_sections.len())
                    .map(|offset| format!("?{}", start + offset))
                    .collect::<Vec<_>>()
                    .join(",");
                scope_clauses.push(format!("d.section IN ({placeholders})"));
                params_vec.extend(
                    target_sections
                        .into_iter()
                        .map(|section| Box::new(section) as Box<dyn rusqlite::ToSql>),
                );
            }
            sql.push_str(&scope_clauses.join(" OR "));
            sql.push(')');

            if !request.filters.sections.is_empty() {
                let param_idx = params_vec.len() + 1;
                let placeholders: Vec<String> = (0..request.filters.sections.len())
                    .map(|i| format!("?{}", param_idx + i))
                    .collect();
                sql.push_str(&format!(" AND d.section IN ({})", placeholders.join(",")));
                for s in &request.filters.sections {
                    params_vec.push(Box::new(s.clone()));
                }
            }

            sql.push_str(" ORDER BY d.rowid");

            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let param_refs: Vec<&dyn rusqlite::ToSql> =
                params_vec.iter().map(|b| b.as_ref()).collect();
            let mut rows = stmt
                .query(param_refs.as_slice())
                .map_err(|e| e.to_string())?;
            let mut hits = Vec::new();
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                let search_id: i64 = row.get(0).map_err(|e| e.to_string())?;
                if !effective_filters.allows_search_id(search_id as u64) {
                    continue;
                }
                hits.push(LexicalHit {
                    search_id,
                    question_id: row.get(1).map_err(|e| e.to_string())?,
                    score: 0.0,
                    relaxed: false,
                });
            }
            taxonomy_hits.extend(hits);
        }

        if is_subtag {
            let mut sql = String::from(
                "SELECT d.search_id, d.question_id
                 FROM search_documents d
                 JOIN question_taxonomy t ON t.question_id = d.question_id
                 WHERE EXISTS (
                    SELECT 1 FROM json_each(t.subtags_json) j WHERE LOWER(j.value) = LOWER(?1)
                 )",
            );
            let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            params_vec.push(Box::new(trimmed_query.to_string()));
            let param_idx = 2;

            if !request.filters.sections.is_empty() {
                let placeholders: Vec<String> = (0..request.filters.sections.len())
                    .map(|i| format!("?{}", param_idx + i))
                    .collect();
                sql.push_str(&format!(" AND d.section IN ({})", placeholders.join(",")));
                for s in &request.filters.sections {
                    params_vec.push(Box::new(s.clone()));
                }
            }

            sql.push_str(" ORDER BY d.rowid");

            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let param_refs: Vec<&dyn rusqlite::ToSql> =
                params_vec.iter().map(|b| b.as_ref()).collect();
            let mut rows = stmt
                .query(param_refs.as_slice())
                .map_err(|e| e.to_string())?;
            let mut hits = Vec::new();
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                let search_id: i64 = row.get(0).map_err(|e| e.to_string())?;
                if !effective_filters.allows_search_id(search_id as u64) {
                    continue;
                }
                hits.push(LexicalHit {
                    search_id,
                    question_id: row.get(1).map_err(|e| e.to_string())?,
                    score: 0.0,
                    relaxed: false,
                });
            }
            taxonomy_hits.extend(hits);
        }

        // 2. Lexical retrieval via SQLite FTS5
        let mut spelling_alternatives = Vec::new();
        let mut ranking_query = compiled.clone();
        let query_term_count = compiled.word_patterns().len();
        let mut lexical_hits =
            LexicalSearch::search(conn, &compiled, &effective_filters, candidate_limit)
                .map_err(|error| format!("Lexical search failed: {error}"))?;
        if !lexical_hits.iter().any(|hit| !hit.relaxed) && taxonomy_hits.is_empty() {
            let (recovered, alternatives) =
                self.recover_query(conn, &compiled, &effective_filters)?;
            spelling_alternatives = alternatives;
            if let Some(corrected) = recovered {
                lexical_hits =
                    LexicalSearch::search(conn, &corrected, &effective_filters, candidate_limit)
                        .map_err(|error| format!("Corrected lexical search failed: {error}"))?;
                ranking_query = corrected;
            }
        }

        let taxonomy_ids = taxonomy_hits
            .iter()
            .map(|hit| hit.search_id)
            .collect::<std::collections::HashSet<_>>();
        for hit in &mut lexical_hits {
            if taxonomy_ids.contains(&hit.search_id) {
                hit.relaxed = false;
            }
        }
        let mut seen = lexical_hits
            .iter()
            .map(|hit| hit.search_id)
            .collect::<std::collections::HashSet<_>>();
        lexical_hits.extend(
            taxonomy_hits
                .into_iter()
                .filter(|hit| seen.insert(hit.search_id)),
        );
        if let Some(ids) = &required_ids {
            lexical_hits.retain(|hit| ids.contains(&(hit.search_id as u64)));
        }
        options.cancellation.check()?;

        // A complete topic word has broader intent than a specific phrase or
        // a prefix still being typed. Real lexical evidence establishes that
        // the topic exists in this scope; a high semantic activation floor
        // must not switch off all related concepts for it.
        let broad_topic_query = if lexical_hits.iter().any(|hit| !hit.relaxed)
            && ranking_query.terms().len() == 1
            && !ranking_query.is_phrase(0)
            && ranking_query.terms()[0].chars().count() >= 3
            && ranking_query.terms()[0].chars().all(char::is_alphabetic)
        {
            // No wildcard: `silv` is not yet the complete word `silver`.
            let exact = FtsQueryBuilder::build_with_options(
                &ranking_query.terms()[0],
                &crate::search::lexical::query_builder::FtsQueryOptions {
                    enable_prefix_matching: false,
                    ..Default::default()
                },
            )
            .expect("the compiled topic contains a word");
            !LexicalSearch::search(conn, &exact, &effective_filters, 1)
                .map_err(|error| format!("Topic lookup failed: {error}"))?
                .is_empty()
        } else {
            false
        };

        // 3. Semantic retrieval via dense vector index
        let mut semantic_hits = Vec::new();
        let exact_only = compiled.entirely_quoted();
        let semantic_enabled = options.semantic && !exact_only && spelling_alternatives.is_empty();
        let mut semantic_status = if exact_only || !spelling_alternatives.is_empty() {
            SemanticStatus::NotRequested
        } else if !options.semantic
            && self.embedding_engine.is_some()
            && self.vector_index.is_some()
        {
            SemanticStatus::Pending
        } else {
            SemanticStatus::Unavailable
        };
        let semantic_query = if ranking_query != compiled {
            ranking_query.display_text()
        } else {
            request.query.clone()
        };
        let exact_numeric_pairs = extract_exact_numeric_pairs(&semantic_query);

        if let Some(engine) = self.embedding_engine.as_ref().filter(|_| semantic_enabled) {
            if let Some(index) = &self.vector_index {
                // Generate query embedding (lazy-loads Granite on first call)
                match engine.embed_query_cancellable(&semantic_query, &options.cancellation) {
                    Ok(query_vec) => {
                        options.cancellation.check()?;
                        match index.search(
                            &query_vec,
                            &effective_filters,
                            limit.max(self.tuning.semantic_fusion_window),
                        ) {
                            Ok(hits) => {
                                semantic_status = SemanticStatus::Available;
                                let max_sim = hits.first().map(|h| h.score).unwrap_or(0.0);
                                let has_primary_lexical =
                                    lexical_hits.iter().any(|hit| !hit.relaxed);
                                let has_domain_anchor = query_has_taxonomy_anchor(&semantic_query);
                                let threshold = if broad_topic_query {
                                    self.tuning.semantic_candidate_floor
                                } else if has_primary_lexical || has_domain_anchor {
                                    self.tuning.semantic_floor_with_lexical
                                } else {
                                    self.tuning.semantic_floor_without_lexical
                                };
                                if max_sim >= threshold {
                                    let related_margin = if broad_topic_query {
                                        self.tuning.semantic_topic_margin
                                    } else {
                                        semantic_related_margin(
                                            query_term_count,
                                            has_primary_lexical,
                                            &self.tuning,
                                        )
                                    };
                                    let candidate_threshold = (max_sim - related_margin)
                                        .max(self.tuning.semantic_candidate_floor);
                                    semantic_hits = hits
                                        .into_iter()
                                        .filter(|hit| hit.score >= candidate_threshold)
                                        .collect();
                                }
                            }
                            Err(e) => {
                                log::warn!("Vector search error (falling back to lexical): {e}");
                            }
                        }
                    }
                    Err(e) => {
                        options.cancellation.check()?;
                        log::warn!("Query embedding error (falling back to lexical): {e}");
                    }
                }
            }
        }

        options.cancellation.check()?;
        // Every semantic candidate needs independent topic evidence, regardless
        // of query length. A broad category alone is not sufficient.
        if !semantic_hits.is_empty() {
            if self.topic_evidence.get().is_none() {
                match TopicEvidence::load(conn) {
                    Ok(evidence) => {
                        let _ = self.topic_evidence.set(evidence);
                    }
                    Err(error) => {
                        log::warn!("Topic evidence unavailable; retaining literal results: {error}")
                    }
                }
            }
            let evidence_query = match stemmed_evidence_query(conn, &ranking_query) {
                Ok(query) => Some(query),
                Err(error) => {
                    log::warn!(
                        "Topic query tokenization failed; retaining literal results: {error}"
                    );
                    None
                }
            };
            if let (Some(evidence), Some(evidence_query)) =
                (self.topic_evidence.get(), evidence_query)
            {
                evidence.retain_supported(
                    &mut semantic_hits,
                    &lexical_hits,
                    &evidence_query,
                    &effective_filters,
                );
            } else {
                let primary_ids = lexical_hits
                    .iter()
                    .filter(|hit| !hit.relaxed)
                    .map(|hit| hit.search_id as u64)
                    .collect::<std::collections::HashSet<_>>();
                semantic_hits.retain(|hit| primary_ids.contains(&hit.search_id));
                semantic_status = SemanticStatus::Unavailable;
            }
        }

        // Relaxed lexical matches need retained semantic evidence. Primary
        // all-term matches remain exhaustive even when semantic search fails.
        let semantic_ids = semantic_hits
            .iter()
            .map(|hit| hit.search_id as i64)
            .collect::<std::collections::HashSet<_>>();
        lexical_hits.retain(|hit| !hit.relaxed || semantic_ids.contains(&hit.search_id));

        // 4. If neither returned results, return empty response
        if lexical_hits.is_empty() && semantic_hits.is_empty() {
            return Ok(SearchResponse {
                hits: Vec::new(),
                interpreted_query: Some(ranking_query),
                semantic_status,
                spelling_alternatives,
                ..Default::default()
            });
        }

        // 5. Build ID lookup table for candidates from SQLite
        let mut candidate_search_ids: Vec<i64> = Vec::new();
        for h in &lexical_hits {
            candidate_search_ids.push(h.search_id);
        }
        for h in &semantic_hits {
            candidate_search_ids.push(h.search_id as i64);
        }
        candidate_search_ids.sort_unstable();
        candidate_search_ids.dedup();

        let (id_map, text_map) = fetch_candidate_metadata(conn, &candidate_search_ids)?;

        // 6. Rank the established high-confidence windows with RRF. Retrieval
        // remains exhaustive, but deep candidates must not perturb the top
        // ordering merely because they weakly occur in both long lists.
        let lexical_fusion_len = lexical_hits.len().min(self.tuning.lexical_fusion_window);
        let semantic_fusion_len = semantic_hits.len().min(self.tuning.semantic_fusion_window);
        let mut fused = reciprocal_rank_fusion(
            &lexical_hits[..lexical_fusion_len],
            &semantic_hits[..semantic_fusion_len],
            |sid| id_map.get(&sid).cloned(),
            &self.tuning,
        );
        let core_ids = fused
            .iter()
            .map(|hit| hit.search_id)
            .collect::<std::collections::HashSet<_>>();

        // Append every additional supported lexical/semantic candidate. These
        // remain discoverable as related results but cannot change core ranks.
        let comprehensive = reciprocal_rank_fusion(
            &lexical_hits,
            &semantic_hits,
            |sid| id_map.get(&sid).cloned(),
            &self.tuning,
        );
        fused.extend(
            comprehensive
                .into_iter()
                .filter(|hit| !core_ids.contains(&hit.search_id)),
        );

        // 7. Exact match boosts
        let literal_matches = apply_exact_match_boosts(&mut fused, &ranking_query, &text_map);

        let primary_lexical_scores = lexical_hits
            .iter()
            .filter(|hit| !hit.relaxed)
            .map(|hit| (hit.search_id, hit.score))
            .collect::<HashMap<_, _>>();
        let strongest_lexical_score = primary_lexical_scores
            .values()
            .copied()
            .fold(0.0_f32, f32::max);
        let strong_lexical_cutoff = strongest_lexical_score * 0.35;
        let lexical_ids = lexical_hits
            .iter()
            .map(|hit| hit.search_id)
            .collect::<std::collections::HashSet<_>>();
        let semantic_scores = semantic_hits
            .iter()
            .map(|hit| (hit.search_id as i64, hit.score))
            .collect::<HashMap<_, _>>();

        let mut hits = fused
            .into_iter()
            .map(|fused_hit| {
                let lexical_match = lexical_ids.contains(&fused_hit.search_id);
                let semantic_score = semantic_scores.get(&fused_hit.search_id).copied();
                let semantic_match = semantic_score.is_some();
                let primary_lexical_match =
                    primary_lexical_scores.get(&fused_hit.search_id).copied();
                let strong_lexical =
                    primary_lexical_match.is_some_and(|score| score >= strong_lexical_cutoff);
                let numeric_constraints_supported = exact_numeric_pairs.is_empty()
                    || text_map.get(&fused_hit.search_id).is_some_and(|text| {
                        exact_numeric_pairs
                            .iter()
                            .all(|pair| contains_exact_token_pair(text, pair))
                    });
                // A literal phrase remains decisive even when document length
                // pushes its initial BM25 rank beyond the fusion window.
                let literal_lexical = primary_lexical_match.is_some()
                    && literal_matches.contains(&fused_hit.search_id);
                let match_strength = if numeric_constraints_supported
                    && (literal_lexical
                        || taxonomy_ids.contains(&fused_hit.search_id)
                        || (core_ids.contains(&fused_hit.search_id)
                            && (strong_lexical
                                || (primary_lexical_match.is_some() && semantic_match))))
                {
                    MatchStrength::Strong
                } else {
                    MatchStrength::Related
                };
                SearchHit {
                    search_id: fused_hit.search_id,
                    question_id: fused_hit.question_id,
                    score: fused_hit.score,
                    match_strength,
                    lexical_match,
                    semantic_match,
                }
            })
            .collect::<Vec<_>>();

        // Confidence tiers are stable sections in the UI. Within each tier,
        // preserve the relevance order produced by fusion and exact boosts.
        hits.sort_unstable_by(|left, right| {
            let left_tier = usize::from(left.match_strength == MatchStrength::Related);
            let right_tier = usize::from(right.match_strength == MatchStrength::Related);
            left_tier.cmp(&right_tier).then_with(|| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| left.search_id.cmp(&right.search_id))
            })
        });

        if hits.len() > limit {
            hits.truncate(limit);
        }

        options.cancellation.check()?;
        Ok(SearchResponse {
            hits,
            interpreted_query: Some(ranking_query),
            semantic_status,
            spelling_alternatives,
            ..Default::default()
        })
    }

    /// Executes search and hydrates the results into the full `QuestionSearchResponse` required by the UI.
    pub fn execute_question_search(
        &self,
        conn: &Connection,
        query: &str,
        sections: Option<&[String]>,
    ) -> Result<crate::backend::types::QuestionSearchResponse, String> {
        self.execute_question_search_with_options(conn, query, sections, &SearchOptions::default())
    }

    pub fn execute_question_search_with_options(
        &self,
        conn: &Connection,
        query: &str,
        sections: Option<&[String]>,
        options: &SearchOptions,
    ) -> Result<crate::backend::types::QuestionSearchResponse, String> {
        options.cancellation.check()?;
        let trimmed = query.trim();
        let mut filters = crate::search::filters::SearchFilter::default();
        if let Some(sec_slice) = sections {
            filters.sections = sec_slice.to_vec();
        }

        let mut count_sql = String::from("SELECT COUNT(*) FROM search_documents d WHERE 1 = 1");
        let mut count_values = Vec::new();
        filters
            .append_sql(&mut count_sql, &mut count_values)
            .map_err(|error| error.to_string())?;
        let total_searched = conn
            .query_row(
                &count_sql,
                rusqlite::params_from_iter(count_values.iter()),
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| format!("Could not count searchable questions: {error}"))?
            as usize;

        if trimmed.is_empty() || total_searched == 0 {
            return Ok(crate::backend::types::QuestionSearchResponse {
                query: query.to_string(),
                searched_questions: total_searched,
                total_matches: 0,
                results: Vec::new(),
                corrected_query: None,
                original_spelling_query: None,
                highlight_terms: Vec::new(),
                spelling_alternatives: Vec::new(),
                semantic_status: SemanticStatus::NotRequested,
            });
        }

        let request = SearchRequest {
            query: query.to_string(),
            filters,
            // The UI asks retrieval to evaluate the complete active scope.
            // Relevance thresholds, not a fixed top-K, determine result count.
            limit: total_searched.max(1),
        };

        let response = self.search_with_options(conn, &request, options)?;
        let original = FtsQueryBuilder::build(query);
        let interpreted = response.interpreted_query.as_ref().or(original.as_ref());
        let corrected_query = interpreted
            .zip(original.as_ref())
            .filter(|(effective, original)| effective.terms() != original.terms())
            .map(|(effective, _)| effective.display_text());
        let original_spelling_query = corrected_query
            .as_ref()
            .and_then(|_| original.as_ref().map(|query| query.exact_spelling_text()));
        let highlight_terms = interpreted
            .map(|query| query.word_patterns())
            .unwrap_or_default();

        let search_ids: Vec<i64> = response.hits.iter().map(|h| h.search_id).collect();
        let mut hydrated_map = hydrate_search_documents(conn, &search_ids)?;

        // Restore confidence-tiered ranking and compute normalized relevance.
        // The tier offset keeps every strong match above every related result
        // while preserving score order within each section.
        let max_score = response
            .hits
            .first()
            .map(tiered_relevance_score)
            .unwrap_or(1.0)
            .max(1e-6);
        let mut results = Vec::with_capacity(response.hits.len());

        for hit in &response.hits {
            if let Some(doc) = hydrated_map.remove(&hit.search_id) {
                let norm_sim = (tiered_relevance_score(hit) / max_score).clamp(0.0, 1.0) as f64;
                let question_number = source_question_number(&doc.question_id);
                results.push(crate::backend::types::QuestionSearchResult {
                    question_id: doc.question_id,
                    bank_id: doc.bank_id,
                    bank_name: doc.bank_name,
                    question_number,
                    question: doc.question,
                    options: doc.options,
                    year: doc.year,
                    stage: doc.stage,
                    paper: doc.paper,
                    section: doc.section,
                    main_tag: doc.main_tag,
                    subtags: doc.subtags,
                    similarity: norm_sim,
                    match_strength: hit.match_strength,
                    lexical_match: hit.lexical_match,
                    semantic_match: hit.semantic_match,
                });
            }
        }

        Ok(crate::backend::types::QuestionSearchResponse {
            query: query.to_string(),
            searched_questions: total_searched,
            total_matches: results.len(),
            results,
            corrected_query,
            original_spelling_query,
            highlight_terms,
            spelling_alternatives: response.spelling_alternatives,
            semantic_status: response.semantic_status,
        })
    }

    fn recover_query(
        &self,
        conn: &Connection,
        query: &crate::search::lexical::query_builder::CompiledFtsQuery,
        filters: &crate::search::filters::SearchFilter,
    ) -> Result<
        (
            Option<crate::search::lexical::query_builder::CompiledFtsQuery>,
            Vec<String>,
        ),
        String,
    > {
        let mut prefixes = (0..query.terms().len())
            .map(|index| query.is_prefix(index))
            .collect::<Vec<_>>();
        for (index, term) in query.terms().iter().enumerate() {
            // Recover unfinished earlier words only after the original query
            // fails. Real words, short fragments, identities and quotes retain
            // their meaning. All terms must still match in the active scope.
            if !prefixes[index]
                && !query.is_phrase(index)
                && term.chars().count() >= 3
                && term.chars().all(char::is_alphabetic)
                && !LexicalSearch::term_exists(conn, &query.term_match(index))
                    .map_err(|error| error.to_string())?
            {
                prefixes[index] = true;
            }
        }
        let expanded = query.with_prefixes(prefixes);
        if expanded != *query && has_primary_match(conn, &expanded, filters)? {
            return Ok((Some(expanded), Vec::new()));
        }
        self.corrected_query(conn, &expanded, filters)
    }

    fn corrected_query(
        &self,
        conn: &Connection,
        query: &crate::search::lexical::query_builder::CompiledFtsQuery,
        filters: &crate::search::filters::SearchFilter,
    ) -> Result<
        (
            Option<crate::search::lexical::query_builder::CompiledFtsQuery>,
            Vec<String>,
        ),
        String,
    > {
        let vocabulary = if let Some(vocabulary) = self.vocabulary.get() {
            vocabulary
        } else {
            let candidate = load_search_vocabulary(conn)?;
            let _ = self.vocabulary.set(candidate);
            self.vocabulary
                .get()
                .ok_or_else(|| "Failed to initialize search vocabulary".to_string())?
        };

        // Keep several plausible spellings until the whole query is checked.
        // A frequent dictionary neighbour may be wrong for these other words
        // or this scope. Bound both the candidate set and SQL work per query.
        let mut changed = false;
        let mut candidates = vec![(Vec::new(), 0usize, 0usize)];
        for (index, term) in query.terms().iter().enumerate() {
            let normalized = term.to_lowercase();
            if query.is_phrase(index)
                || normalized.chars().count() < 4
                || normalized.chars().count() > 64
                || normalized.chars().any(|character| character.is_numeric())
                || vocabulary.exact.contains(&normalized)
                || LexicalSearch::term_exists(conn, &query.term_match(index))
                    .map_err(|error| error.to_string())?
            {
                for (terms, _, _) in &mut candidates {
                    terms.push(term.clone());
                }
                continue;
            }

            let max_distance = if normalized.chars().count() >= 8 {
                2
            } else {
                1
            };
            let length = normalized.chars().count();
            let mut corrections = (length.saturating_sub(max_distance)..=length + max_distance)
                .filter_map(|candidate_length| vocabulary.terms_by_length.get(&candidate_length))
                .flatten()
                .filter_map(|(candidate, frequency)| {
                    let distance = bounded_edit_distance(&normalized, candidate, max_distance)?;
                    Some((distance, std::cmp::Reverse(*frequency), candidate))
                })
                .collect::<Vec<_>>();
            corrections.sort_unstable_by(|left, right| {
                left.0
                    .cmp(&right.0)
                    .then_with(|| left.1.cmp(&right.1))
                    .then_with(|| left.2.cmp(right.2))
            });
            corrections.truncate(8);
            if corrections.is_empty() {
                for (terms, _, _) in &mut candidates {
                    terms.push(term.clone());
                }
                continue;
            }
            changed = true;
            candidates = candidates
                .into_iter()
                .flat_map(|(terms, distance, frequency)| {
                    corrections.iter().map(
                        move |(edit_distance, std::cmp::Reverse(count), correction)| {
                            let mut terms = terms.clone();
                            terms.push((*correction).clone());
                            (terms, distance + edit_distance, frequency + count)
                        },
                    )
                })
                .collect();
            candidates.sort_unstable_by(|left, right| {
                left.1
                    .cmp(&right.1)
                    .then_with(|| right.2.cmp(&left.2))
                    .then_with(|| left.0.cmp(&right.0))
            });
            candidates.truncate(24);
        }
        let mut supported = Vec::new();
        let mut best_distance = None;
        if changed {
            for (terms, distance, _) in candidates {
                if best_distance.is_some_and(|best| distance > best) {
                    break;
                }
                let corrected = query.with_terms(terms);
                if has_primary_match(conn, &corrected, filters)? {
                    best_distance = Some(distance);
                    supported.push(corrected);
                }
            }
        }
        if supported.len() == 1 {
            Ok((supported.pop(), Vec::new()))
        } else {
            Ok((
                None,
                supported
                    .into_iter()
                    .take(4)
                    .map(|query| query.display_text())
                    .collect(),
            ))
        }
    }
}

/// Ask the same tokenizer as the index for query stems; prefix guesses do not
/// account for substitutions such as `economy` -> `economi`.
fn stemmed_evidence_query(
    conn: &Connection,
    query: &crate::search::lexical::query_builder::CompiledFtsQuery,
) -> Result<crate::search::lexical::query_builder::CompiledFtsQuery, String> {
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS temp.search_query_tokens
        USING fts5(word, tokenize='porter unicode61');
        CREATE VIRTUAL TABLE IF NOT EXISTS temp.search_query_stems
        USING fts5vocab('temp', 'search_query_tokens', 'instance');
        DELETE FROM temp.search_query_tokens;",
    )
    .map_err(|error| error.to_string())?;
    for (index, term) in query.terms().iter().enumerate() {
        conn.execute(
            "INSERT INTO temp.search_query_tokens(rowid, word) VALUES (?1, ?2)",
            rusqlite::params![index as i64 + 1, term],
        )
        .map_err(|error| error.to_string())?;
    }
    let mut statement = conn
        .prepare("SELECT doc, term FROM temp.search_query_stems ORDER BY doc, offset")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;
    let mut stems = vec![Vec::new(); query.terms().len()];
    for row in rows {
        let (document, term) = row.map_err(|error| error.to_string())?;
        if let Some(words) = stems.get_mut((document - 1) as usize) {
            words.push(term);
        }
    }
    Ok(query.with_terms(
        stems
            .into_iter()
            .zip(query.terms())
            .map(|(words, original)| {
                if words.is_empty() {
                    original.clone()
                } else {
                    words.join(" ")
                }
            })
            .collect(),
    ))
}

fn has_primary_match(
    conn: &Connection,
    query: &crate::search::lexical::query_builder::CompiledFtsQuery,
    filters: &crate::search::filters::SearchFilter,
) -> Result<bool, String> {
    LexicalSearch::search(conn, query, filters, 1)
        .map(|hits| hits.iter().any(|hit| !hit.relaxed))
        .map_err(|error| error.to_string())
}

fn hydrate_search_documents(
    conn: &Connection,
    search_ids: &[i64],
) -> Result<HashMap<i64, HydratedDoc>, String> {
    let mut hydrated = HashMap::with_capacity(search_ids.len());

    for batch in search_ids.chunks(HYDRATION_BATCH_SIZE) {
        let placeholders = std::iter::repeat("?")
            .take(batch.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT d.search_id, d.question_id, d.question, q.options, d.bank_id, d.bank_name,
                    d.year, d.stage, d.paper, d.section, d.main_tag, t.subtags_json
             FROM search_documents d
             LEFT JOIN questions q ON q.id = d.question_id
             LEFT JOIN question_taxonomy t ON t.question_id = d.question_id
             WHERE d.search_id IN ({placeholders})"
        );
        let parameters = batch
            .iter()
            .map(|id| id as &dyn rusqlite::ToSql)
            .collect::<Vec<_>>();
        let mut stmt = conn.prepare(&sql).map_err(|error| error.to_string())?;
        let mut rows = stmt
            .query(parameters.as_slice())
            .map_err(|error| error.to_string())?;

        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            let search_id = row.get(0).map_err(|error| error.to_string())?;
            let options = row
                .get::<_, Option<String>>(3)
                .map_err(|error| error.to_string())?
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default();
            let subtags = row
                .get::<_, Option<String>>(11)
                .map_err(|error| error.to_string())?
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default();

            hydrated.insert(
                search_id,
                HydratedDoc {
                    question_id: row.get(1).map_err(|error| error.to_string())?,
                    question: row.get(2).map_err(|error| error.to_string())?,
                    options,
                    bank_id: row.get(4).map_err(|error| error.to_string())?,
                    bank_name: row.get(5).map_err(|error| error.to_string())?,
                    year: row.get(6).map_err(|error| error.to_string())?,
                    stage: row.get(7).map_err(|error| error.to_string())?,
                    paper: row.get(8).map_err(|error| error.to_string())?,
                    section: row.get(9).map_err(|error| error.to_string())?,
                    main_tag: row.get(10).map_err(|error| error.to_string())?,
                    subtags,
                },
            );
        }
    }

    Ok(hydrated)
}

fn load_search_vocabulary(conn: &Connection) -> Result<SearchVocabulary, String> {
    // Read the compact FTS term dictionary instead of scanning and tokenizing
    // every source document each time a service cache is initialized.
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS temp.question_fts_vocabulary
         USING fts5vocab('main', 'question_literal_fts', 'row');",
    )
    .map_err(|error| error.to_string())?;
    let mut stmt = conn
        .prepare("SELECT term, cnt FROM temp.question_fts_vocabulary WHERE length(term) >= 3")
        .map_err(|error| error.to_string())?;
    let mut rows = stmt.query([]).map_err(|error| error.to_string())?;
    let mut frequencies = HashMap::<String, usize>::new();
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        let term: String = row.get(0).map_err(|error| error.to_string())?;
        let frequency: i64 = row.get(1).map_err(|error| error.to_string())?;
        frequencies.insert(term, frequency.max(0) as usize);
    }
    let exact = frequencies.keys().cloned().collect();
    let mut terms_by_length = HashMap::<usize, Vec<(String, usize)>>::new();
    for (term, frequency) in frequencies {
        terms_by_length
            .entry(term.chars().count())
            .or_default()
            .push((term, frequency));
    }
    for terms in terms_by_length.values_mut() {
        terms.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    }
    Ok(SearchVocabulary {
        terms_by_length,
        exact,
    })
}

fn bounded_edit_distance(left: &str, right: &str, maximum: usize) -> Option<usize> {
    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    let length_difference = if left.len() >= right.len() {
        left.len() - right.len()
    } else {
        right.len() - left.len()
    };
    if length_difference > maximum {
        return None;
    }
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];
    let mut before_previous = previous.clone();
    for (left_index, left_character) in left.iter().enumerate() {
        current[0] = left_index + 1;
        let mut row_minimum = current[0];
        for (right_index, right_character) in right.iter().enumerate() {
            let substitution =
                previous[right_index] + usize::from(left_character != right_character);
            current[right_index + 1] = (current[right_index] + 1)
                .min(previous[right_index + 1] + 1)
                .min(substitution);
            if left_index > 0
                && right_index > 0
                && left_character == &right[right_index - 1]
                && &left[left_index - 1] == right_character
            {
                current[right_index + 1] =
                    current[right_index + 1].min(before_previous[right_index - 1] + 1);
            }
            row_minimum = row_minimum.min(current[right_index + 1]);
        }
        if row_minimum > maximum {
            return None;
        }
        std::mem::swap(&mut before_previous, &mut previous);
        std::mem::swap(&mut previous, &mut current);
    }
    (previous[right.len()] <= maximum).then_some(previous[right.len()])
}

fn semantic_related_margin(
    query_term_count: usize,
    has_primary_lexical: bool,
    tuning: &SearchTuning,
) -> f32 {
    if has_primary_lexical {
        return match query_term_count {
            0..=2 => tuning.semantic_with_lexical_margin,
            3 => tuning.semantic_with_lexical_multi_margin,
            _ => tuning.semantic_with_lexical_descriptive_margin,
        };
    }
    match query_term_count {
        0 | 1 => tuning.semantic_single_term_margin,
        2 | 3 => tuning.semantic_multi_term_margin,
        _ => tuning.semantic_descriptive_margin,
    }
}

fn tiered_relevance_score(hit: &SearchHit) -> f32 {
    hit.score
        + if hit.match_strength == MatchStrength::Strong {
            1.0
        } else {
            0.0
        }
}

fn source_question_number(question_id: &str) -> Option<i64> {
    let canonical_id = question_id
        .split_once("::revision:")
        .map_or(question_id, |(canonical, _)| canonical);
    let (_, suffix) = canonical_id.rsplit_once("_q")?;
    (!suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit()))
        .then(|| suffix.parse().ok())
        .flatten()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExactTokenPair {
    left: String,
    right: String,
}

/// Numbers carry exact identity across domains: constitutional provisions,
/// years, targets, percentages, standards, and question identifiers. Keep
/// every adjacent query pair involving a number as an exact Strong-match
/// constraint; semantic proximity may still recover looser Related results.
fn extract_exact_numeric_pairs(query: &str) -> Vec<ExactTokenPair> {
    let tokens = normalized_tokens(query);
    tokens
        .windows(2)
        .filter(|pair| is_numeric_token(&pair[0]) || is_numeric_token(&pair[1]))
        .map(|pair| ExactTokenPair {
            left: pair[0].clone(),
            right: pair[1].clone(),
        })
        .collect()
}

fn contains_exact_token_pair(text: &str, expected: &ExactTokenPair) -> bool {
    normalized_tokens(text)
        .windows(2)
        .any(|pair| pair[0] == expected.left && pair[1] == expected.right)
}

fn is_numeric_token(token: &str) -> bool {
    !token.is_empty() && token.chars().all(|character| character.is_ascii_digit())
}

fn normalized_tokens(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn query_has_taxonomy_anchor(query: &str) -> bool {
    static TAXONOMY_TERMS: OnceLock<std::collections::HashSet<String>> = OnceLock::new();
    let taxonomy_terms = TAXONOMY_TERMS.get_or_init(|| {
        crate::taxonomy::labels()
            .main_tags
            .iter()
            .flat_map(|tag| [tag.label.as_str(), tag.description.as_str()])
            .chain(
                crate::taxonomy::labels()
                    .subtags
                    .iter()
                    .flat_map(|tag| [tag.label.as_str(), tag.description.as_str()]),
            )
            .flat_map(anchor_tokens)
            .collect()
    });
    anchor_tokens(query)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .filter(|term| taxonomy_terms.contains(term))
        .take(2)
        .count()
        >= 2
}

fn anchor_tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|character: char| !character.is_alphanumeric())
        .filter_map(|token| {
            let mut normalized = token.to_lowercase();
            if normalized.len() > 4 && normalized.ends_with('s') {
                normalized.pop();
            }
            if normalized.len() < 4 || is_anchor_stop_word(&normalized) {
                None
            } else {
                Some(normalized)
            }
        })
}

fn is_anchor_stop_word(term: &str) -> bool {
    matches!(
        term,
        "about"
            | "across"
            | "also"
            | "between"
            | "from"
            | "into"
            | "other"
            | "their"
            | "these"
            | "this"
            | "through"
            | "with"
    )
}

fn resolve_allowed_search_ids(
    conn: &Connection,
    filters: &crate::search::filters::SearchFilter,
) -> Result<std::collections::HashSet<u64>, String> {
    let mut sql = String::from("SELECT d.search_id FROM search_documents d WHERE 1 = 1");
    let mut values = Vec::new();
    filters
        .append_sql(&mut sql, &mut values)
        .map_err(|error| error.to_string())?;
    let mut stmt = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(values.iter()), |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|error| error.to_string())?;

    rows.map(|row| row.map(|id| id as u64).map_err(|error| error.to_string()))
        .collect()
}

type CandidateMetadata = (HashMap<i64, String>, HashMap<i64, String>);

fn fetch_candidate_metadata(
    conn: &Connection,
    search_ids: &[i64],
) -> Result<CandidateMetadata, String> {
    if search_ids.is_empty() {
        return Ok((HashMap::new(), HashMap::new()));
    }

    let mut id_map = HashMap::new();
    let mut text_map = HashMap::new();
    for batch in search_ids.chunks(HYDRATION_BATCH_SIZE) {
        let placeholders: Vec<String> = batch.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "SELECT search_id, question_id, question, options_text
         FROM search_documents WHERE search_id IN ({})",
            placeholders.join(",")
        );

        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let params_refs: Vec<&dyn rusqlite::ToSql> =
            batch.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

        let mut rows = stmt
            .query(params_refs.as_slice())
            .map_err(|e| e.to_string())?;

        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let sid: i64 = row.get(0).map_err(|e| e.to_string())?;
            let qid: String = row.get(1).map_err(|e| e.to_string())?;
            let question: String = row.get(2).map_err(|e| e.to_string())?;
            let options: String = row.get(3).map_err(|e| e.to_string())?;
            id_map.insert(sid, qid);
            text_map.insert(sid, format!("{question} {options}"));
        }
    }

    Ok((id_map, text_map))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::db::schema::run_migrations;
    use crate::search::embedding::engine::{Embedding, EmbeddingError};
    use crate::search::filters::SearchFilter;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingEmbeddingEngine {
        query_calls: Arc<AtomicUsize>,
    }

    impl EmbeddingEngine for CountingEmbeddingEngine {
        fn dimensions(&self) -> usize {
            384
        }

        fn embed_query(&self, text: &str) -> Result<Embedding, EmbeddingError> {
            assert!(!text.trim().is_empty());
            self.query_calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![0.0; self.dimensions()])
        }

        fn embed_documents(&self, texts: &[String]) -> Result<Vec<Embedding>, EmbeddingError> {
            Ok(texts.iter().map(|_| vec![0.0; self.dimensions()]).collect())
        }
    }

    #[test]
    fn warm_embedding_performs_real_non_empty_inference() {
        let query_calls = Arc::new(AtomicUsize::new(0));
        let engine = Arc::new(CountingEmbeddingEngine {
            query_calls: Arc::clone(&query_calls),
        });
        let service = SearchService::new(Some(engine), None);

        service.warm_embedding().unwrap();

        assert_eq!(query_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn taxonomy_anchors_distinguish_upsc_topics_from_unrelated_hobbies() {
        assert!(query_has_taxonomy_anchor(
            "price surge eroding purchasing ability of low income earners"
        ));
        assert!(query_has_taxonomy_anchor(
            "division of taxation authority between union and provinces"
        ));
        assert!(!query_has_taxonomy_anchor(
            "crochet pattern for a stuffed dinosaur"
        ));
        assert!(!query_has_taxonomy_anchor(
            "video game speedrunning tutorial"
        ));
    }

    #[test]
    fn bounded_edit_distance_accepts_typo_but_rejects_unrelated_word() {
        assert_eq!(bounded_edit_distance("silvre", "silver", 1), Some(1));
        assert_eq!(bounded_edit_distance("notcie", "notice", 1), Some(1));
        assert_eq!(bounded_edit_distance("parliment", "parliament", 2), Some(1));
        assert_eq!(
            bounded_edit_distance("enviroment", "environment", 2),
            Some(1)
        );
        assert_eq!(bounded_edit_distance("crochet", "climate", 2), None);
    }

    #[test]
    fn semantic_margin_is_query_adaptive_and_tighter_with_lexical_support() {
        let tuning = SearchTuning::default();
        assert_eq!(semantic_related_margin(1, false, &tuning), 0.08);
        assert_eq!(semantic_related_margin(2, false, &tuning), 0.10);
        assert_eq!(semantic_related_margin(6, false, &tuning), 0.10);
        assert_eq!(semantic_related_margin(2, true, &tuning), 0.05);
        assert_eq!(semantic_related_margin(3, true, &tuning), 0.07);
        assert_eq!(semantic_related_margin(6, true, &tuning), 0.09);
    }

    #[test]
    fn source_question_number_is_not_confused_with_result_rank() {
        assert_eq!(source_question_number("upsc_2013_csat_q13"), Some(13));
        assert_eq!(source_question_number("upsc_2026_gs1_q100"), Some(100));
        assert_eq!(
            source_question_number("upsc_2025_mains_gs1_q17::revision:abcdef01-bank1234"),
            Some(17)
        );
        assert_eq!(source_question_number("custom-question-id"), None);
        assert_eq!(source_question_number("custom_qabc"), None);
        assert_eq!(
            source_question_number("custom_qabc::revision:abcdef01-bank1234"),
            None
        );
    }

    #[test]
    fn numeric_constraints_are_generic_exact_token_pairs() {
        for (query, matching, non_matching) in [
            (
                "Article 20",
                "Protection under Article 20 of the Constitution",
                "An article costs Rs. 20",
            ),
            (
                "calendar 2025",
                "calendar 2025 repeats in a later year",
                "calendar for the year 2025",
            ),
            (
                "SDG 5",
                "Progress under SDG 5",
                "SDG targets across 5 regions",
            ),
            (
                "20 percent",
                "A 20 percent reduction",
                "The percentage reduction has a value of 20",
            ),
        ] {
            let pairs = extract_exact_numeric_pairs(query);
            assert_eq!(pairs.len(), 1);
            assert!(contains_exact_token_pair(matching, &pairs[0]));
            assert!(!contains_exact_token_pair(non_matching, &pairs[0]));
        }
        assert!(extract_exact_numeric_pairs("Article twenty").is_empty());
    }

    #[test]
    fn multi_word_typo_is_corrected_before_lexical_search() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO question_banks (id, name, exam, metadata, total_questions, difficulty, default_duration, imported_at)
             VALUES ('b1', 'Bank 1', 'UPSC', '{}', 1, 'medium', 7200, 1000)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO questions (id, bank_id, type, question, correct_answers, marks)
             VALUES ('q1', 'b1', 'single', 'How does Parliament ensure accountability?', '[]', 2.0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO search_documents (
                question_id, question, options_text, main_tag, subtags_text,
                bank_id, bank_name, year, stage, paper, section, content_fingerprint
            ) VALUES ('q1', 'How does Parliament ensure accountability?', '', 'Polity', 'Parliament', 'b1', 'Bank 1', 2023, 'mains', 'GS-2', 'mains-gs2', X'0102030405060708')",
            [],
        )
        .unwrap();

        let service = SearchService::new(None, None);
        let response = service
            .search(
                &conn,
                &SearchRequest {
                    query: "parliment accountability".to_string(),
                    filters: SearchFilter::default(),
                    limit: 10,
                },
            )
            .unwrap();
        assert_eq!(response.hits[0].question_id, "q1");
    }

    #[test]
    fn recovery_checks_context_scope_and_preserves_explicit_words() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute("INSERT INTO question_banks (id, name, exam, metadata, total_questions, difficulty, default_duration, imported_at)
            VALUES ('b', 'Bank', 'UPSC', '{}', 8, 'medium', 60, 1)", []).unwrap();
        for (id, question, section) in [
            (
                "river",
                "River conservation protects ecosystems.",
                "prelims-gs1",
            ),
            (
                "rider",
                "Rider rider rider rider conservation.",
                "mains-gs2",
            ),
            (
                "notice",
                "Silver Notice traces criminal assets.",
                "prelims-gs1",
            ),
            (
                "law",
                "Parliament accountability and constitutional amendments.",
                "prelims-gs1",
            ),
            ("art", "Art criticism", "prelims-gs1"),
            ("artisan", "Artisans history", "prelims-gs1"),
            ("numeric", "Article 201 protects rights.", "prelims-gs1"),
            (
                "boundaries",
                "Silver. nitrate | Silver; nebula",
                "mains-gs2",
            ),
        ] {
            conn.execute(
                "INSERT INTO questions (id, bank_id, type, question, correct_answers, marks)
                VALUES (?1, 'b', 'single', ?2, '[]', 1)",
                rusqlite::params![id, question],
            )
            .unwrap();
            conn.execute("INSERT INTO search_documents (question_id, question, bank_id, bank_name, section, content_fingerprint)
                VALUES (?1, ?2, 'b', 'Bank', ?3, X'0102030405060708')", rusqlite::params![id, question, section]).unwrap();
        }
        let service = SearchService::new(None, None);
        for (query, expected) in [
            ("silv not", "notice"),
            ("silv notcie", "notice"),
            ("parli acc", "law"),
            ("constit amend", "law"),
            ("riber conservation", "river"),
        ] {
            let response = service
                .execute_question_search(&conn, query, Some(&["prelims-gs1".into()]))
                .unwrap();
            assert_eq!(response.results[0].question_id, expected, "{query}");
            assert_eq!(
                response.results[0].match_strength,
                MatchStrength::Strong,
                "{query}"
            );
        }
        let corrected = service
            .execute_question_search(&conn, "riber conservation", Some(&["prelims-gs1".into()]))
            .unwrap();
        assert_eq!(
            corrected.corrected_query.as_deref(),
            Some("river conservation")
        );
        assert!(service
            .execute_question_search(
                &conn,
                corrected.original_spelling_query.as_deref().unwrap(),
                None
            )
            .unwrap()
            .corrected_query
            .is_none());
        let other_scope = service
            .execute_question_search(&conn, "riber conservation", Some(&["mains-gs2".into()]))
            .unwrap();
        assert_eq!(
            other_scope.corrected_query.as_deref(),
            Some("rider conservation")
        );
        let ambiguous = service
            .execute_question_search(&conn, "riber conservation", None)
            .unwrap();
        assert!(ambiguous.corrected_query.is_none());
        assert_eq!(ambiguous.semantic_status, SemanticStatus::NotRequested);
        assert_eq!(ambiguous.spelling_alternatives.len(), 2);
        assert!(ambiguous
            .spelling_alternatives
            .contains(&"river conservation".into()));
        assert!(ambiguous
            .spelling_alternatives
            .contains(&"rider conservation".into()));
        for query in [
            "art history",
            "\"silv\" not",
            "si not",
            "article 20",
            "riber zzzzzzzz",
        ] {
            let response = service.execute_question_search(&conn, query, None).unwrap();
            assert!(response.results.is_empty(), "over-broadened {query}");
            assert!(
                response.corrected_query.is_none(),
                "unsupported correction: {query}"
            );
        }
        let completion = service
            .execute_question_search(&conn, "silv n", Some(&["prelims-gs1".into()]))
            .unwrap();
        assert!(completion
            .results
            .iter()
            .any(|hit| hit.question_id == "notice"));
        assert!(completion.corrected_query.is_none());
        assert!(completion.highlight_terms.iter().all(|term| term.prefix));
        let boundaries = service
            .execute_question_search(&conn, "silver n", Some(&["mains-gs2".into()]))
            .unwrap();
        assert!(!boundaries.results.is_empty());
    }

    #[test]
    fn literal_typing_typos_and_options_work_without_a_semantic_model() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute("INSERT INTO question_banks (id, name, exam, metadata, total_questions, difficulty, default_duration, imported_at)
            VALUES ('b', 'Bank', 'UPSC', '{}', 4, 'medium', 60, 1)", []).unwrap();
        for (id, question, options, section) in [
            (
                "notice",
                "INTERPOL Silver Notice traces criminal assets. International cooperation.",
                "",
                "prelims-gs1",
            ),
            (
                "scattered",
                "Silver deposits. Notice the metal.",
                "",
                "prelims-gs1",
            ),
            (
                "options",
                "Identify the global institution.",
                "World Organization",
                "prelims-gs1",
            ),
            ("excluded", "Silver Notice", "", "mains-gs2"),
        ] {
            conn.execute(
                "INSERT INTO questions (id, bank_id, type, question, correct_answers, marks)
                VALUES (?1, 'b', 'single', ?2, '[]', 1)",
                rusqlite::params![id, question],
            )
            .unwrap();
            conn.execute("INSERT INTO search_documents (question_id, question, options_text, bank_id, bank_name, section, content_fingerprint)
                VALUES (?1, ?2, ?3, 'b', 'Bank', ?4, X'0102030405060708')", rusqlite::params![id, question, options, section]).unwrap();
        }
        let service = SearchService::new(None, None);
        for query in [
            "silver n",
            "silv not",
            "silv notcie",
            "silver no",
            "silver not",
            "silver noti",
            "silver notic",
            "silver notice",
            "silver/notice",
            "silver—notice",
            "silver,notice",
            "silver  notice",
            "SILVER NOTICE",
            "silver notice ***",
            "“silver notice”",
            "\"silver n",
            "silvre notice",
            "silver notcie",
            "\"silver notice\" crimnal",
            "international c",
            "international co",
            "international coop",
            "international cooper",
        ] {
            let response = service
                .execute_question_search(&conn, query, Some(&["prelims-gs1".into()]))
                .unwrap();
            assert_eq!(
                response.results.first().map(|hit| hit.question_id.as_str()),
                Some("notice"),
                "{query}"
            );
            assert_eq!(
                response.results[0].match_strength,
                MatchStrength::Strong,
                "{query}"
            );
            assert!(response
                .results
                .iter()
                .all(|hit| hit.section == "prelims-gs1"));
        }
        for query in [
            "world o",
            "world or",
            "world org",
            "world organ",
            "world organi",
            "world organizat",
        ] {
            let response = service.execute_question_search(&conn, query, None).unwrap();
            assert_eq!(response.results[0].question_id, "options", "{query}");
        }
        for query in ["silvzzzz", "\"silvre notice\"", "silver 20250", "***"] {
            assert!(
                service
                    .execute_question_search(&conn, query, None)
                    .unwrap()
                    .results
                    .is_empty(),
                "{query}"
            );
        }
        for query in ["international n", "silver not", "world organizat"] {
            assert!(
                service
                    .corrected_query(
                        &conn,
                        &FtsQueryBuilder::build(query).unwrap(),
                        &SearchFilter::default()
                    )
                    .unwrap()
                    .0
                    .is_none(),
                "valid stem or prefix was corrected: {query}"
            );
        }

        // A long question with the literal phrase must still outrank hundreds
        // of short questions containing the two words in unrelated positions.
        for index in 0..320 {
            let id = format!("distractor-{index}");
            conn.execute(
                "INSERT INTO questions (id, bank_id, type, question, correct_answers, marks)
                VALUES (?1, 'b', 'single', 'Silver deposits. Notice the metal.', '[]', 1)",
                [&id],
            )
            .unwrap();
            conn.execute("INSERT INTO search_documents (question_id, question, bank_id, bank_name, section, content_fingerprint)
                VALUES (?1, 'Silver deposits. Notice the metal.', 'b', 'Bank', 'prelims-gs1', X'0102030405060708')", [&id]).unwrap();
        }
        let response = service
            .execute_question_search(&conn, "silver notice", Some(&["prelims-gs1".into()]))
            .unwrap();
        assert_eq!(
            response.results[0].question_id, "notice",
            "literal phrase was buried beyond the fusion window"
        );
    }

    #[test]
    fn test_search_service_empty_query_returns_immediately() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let service = SearchService::new(None, None);
        let req = SearchRequest {
            query: "   ".to_string(),
            filters: SearchFilter::default(),
            limit: 10,
        };

        let resp = service.search(&conn, &req).unwrap();
        assert!(resp.hits.is_empty());
        assert_eq!(resp.semantic_status, SemanticStatus::NotRequested);
    }

    #[test]
    fn test_search_service_lexical_only_degrades_gracefully() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        // Populate mock banks and questions
        conn.execute(
            "INSERT INTO question_banks (id, name, exam, metadata, total_questions, difficulty, default_duration, imported_at)
             VALUES ('b1', 'Bank 1', 'UPSC', '{}', 1, 'medium', 7200, 1000)",
            [],
        ).unwrap();

        conn.execute(
            "INSERT INTO questions (id, bank_id, type, question, correct_answers, marks)
             VALUES ('q1', 'b1', 'single', 'What is Article 32?', '[]', 2.0)",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO search_documents (
                question_id, question, options_text, main_tag, subtags_text,
                bank_id, bank_name, year, stage, paper, section, content_fingerprint
            ) VALUES ('q1', 'What is Article 32?', '(A) Right (B) Duty', 'Polity', 'Rights', 'b1', 'Bank 1', 2023, 'prelims', 'GS-1', 'polity', X'0102030405060708')",
            [],
        ).unwrap();

        let service = SearchService::new(None, None);
        let req = SearchRequest {
            query: "Article 32".to_string(),
            filters: SearchFilter::default(),
            limit: 10,
        };

        let resp = service.search(&conn, &req).unwrap();
        assert_eq!(resp.hits.len(), 1);
        assert_eq!(resp.hits[0].question_id, "q1");
        assert_eq!(resp.semantic_status, SemanticStatus::Unavailable);
    }

    #[test]
    fn retired_saved_tag_filters_expand_to_current_atomic_tags() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO question_banks (id, name, exam, metadata, total_questions, difficulty, default_duration, imported_at)
             VALUES ('b1', 'Bank 1', 'UPSC', '{}', 1, 'medium', 7200, 1000)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO questions (id, bank_id, type, question, correct_answers, marks)
             VALUES ('q1', 'b1', 'single', 'What is Article 32?', '[]', 2.0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO search_documents (
                question_id, question, options_text, main_tag, subtags_text,
                bank_id, bank_name, year, stage, paper, section, content_fingerprint
             ) VALUES (
                'q1', 'What is Article 32?', '', 'Constitution', '',
                'b1', 'Bank 1', 2023, 'prelims', 'GS-1', 'prelims-gs1', X'0102030405060708'
             )",
            [],
        )
        .unwrap();

        let service = SearchService::new(None, None);
        let response = service
            .search(
                &conn,
                &SearchRequest {
                    query: "Article 32".to_string(),
                    filters: SearchFilter {
                        tags: vec!["Polity & Constitution".to_string()],
                        ..Default::default()
                    },
                    limit: 10,
                },
            )
            .unwrap();
        assert_eq!(response.hits.len(), 1);
        assert_eq!(response.hits[0].question_id, "q1");
    }
}
