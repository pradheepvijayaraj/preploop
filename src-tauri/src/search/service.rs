//! SearchService — central orchestrator for hybrid lexical + semantic search.

use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use crate::search::embedding::engine::EmbeddingEngine;
use crate::search::lexical::fts::LexicalSearch;
use crate::search::lexical::query_builder::FtsQueryBuilder;
use crate::search::ranking::boosts::apply_exact_match_boosts;
use crate::search::ranking::rrf::{reciprocal_rank_fusion, SearchTuning};
use crate::search::request::SearchRequest;
use crate::search::response::{MatchStrength, SearchHit, SearchResponse};
use crate::search::vector::traits::VectorSearch;

/// Central search coordinator combining SQLite FTS5 lexical search and dense vector search.
pub struct SearchService {
    embedding_engine: Option<Arc<dyn EmbeddingEngine>>,
    vector_index: Arc<RwLock<Option<Arc<dyn VectorSearch>>>>,
    vocabulary: OnceLock<SearchVocabulary>,
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
            vector_index: Arc::new(RwLock::new(vector_index)),
            vocabulary: OnceLock::new(),
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
            effective_filters.allowed_search_ids = Some(Arc::new(resolve_allowed_search_ids(
                conn,
                &effective_filters,
            )?));
        }

        // Exact taxonomy query shortcut
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
                 WHERE j.value = ?1 LIMIT 1",
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
                hits.push(SearchHit {
                    search_id,
                    question_id: row.get(1).map_err(|e| e.to_string())?,
                    score: 1.0,
                    match_strength: MatchStrength::Strong,
                    lexical_match: true,
                    semantic_match: false,
                });
            }
            return Ok(SearchResponse {
                hits,
                semantic_available: false,
                ..Default::default()
            });
        }

        if is_subtag {
            let mut sql = String::from(
                "SELECT d.search_id, d.question_id
                 FROM search_documents d
                 JOIN question_taxonomy t ON t.question_id = d.question_id
                 WHERE EXISTS (
                    SELECT 1 FROM json_each(t.subtags_json) j WHERE j.value = ?1
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
                hits.push(SearchHit {
                    search_id,
                    question_id: row.get(1).map_err(|e| e.to_string())?,
                    score: 1.0,
                    match_strength: MatchStrength::Strong,
                    lexical_match: true,
                    semantic_match: false,
                });
            }
            return Ok(SearchResponse {
                hits,
                semantic_available: false,
                ..Default::default()
            });
        }

        // 2. Lexical retrieval via SQLite FTS5
        let compiled_fts = FtsQueryBuilder::build(&request.query);
        let query_term_count = compiled_fts
            .as_ref()
            .map(|query| query.terms().len())
            .unwrap_or(1);
        let exact_numeric_pairs = extract_exact_numeric_pairs(&request.query);
        let mut lexical_hits = if let Some(ref fts_query) = compiled_fts {
            let original =
                LexicalSearch::search(conn, fts_query, &effective_filters, candidate_limit)
                    .map_err(|e| format!("Lexical search failed: {e}"))?;
            if let Some(corrected_query) = self.corrected_query(conn, fts_query)? {
                if let Some(corrected_fts) = FtsQueryBuilder::build(&corrected_query) {
                    let mut corrected = LexicalSearch::search(
                        conn,
                        &corrected_fts,
                        &effective_filters,
                        candidate_limit,
                    )
                    .map_err(|e| format!("Corrected lexical search failed: {e}"))?;
                    let mut seen = corrected
                        .iter()
                        .map(|hit| hit.search_id)
                        .collect::<std::collections::HashSet<_>>();
                    for hit in original {
                        if seen.insert(hit.search_id) && corrected.len() < candidate_limit {
                            corrected.push(hit);
                        }
                    }
                    corrected
                } else {
                    original
                }
            } else {
                original
            }
        } else {
            Vec::new()
        };

        // 3. Semantic retrieval via dense vector index
        let mut semantic_hits = Vec::new();
        let mut semantic_available = false;
        let mut semantic_rejected_query = false;
        let mut semantic_strong_cutoff = None;

        if let Some(engine) = &self.embedding_engine {
            let index_guard = self
                .vector_index
                .read()
                .map_err(|e| format!("Lock error: {e}"))?;
            if let Some(index) = index_guard.as_ref() {
                // Generate query embedding (lazy-loads Granite on first call)
                match engine.embed_query(&request.query) {
                    Ok(query_vec) => {
                        match index.search(
                            &query_vec,
                            &effective_filters,
                            limit.max(self.tuning.semantic_fusion_window),
                        ) {
                            Ok(hits) => {
                                let max_sim = hits.first().map(|h| h.score).unwrap_or(0.0);
                                let has_primary_lexical =
                                    lexical_hits.iter().any(|hit| !hit.relaxed);
                                let has_domain_anchor = query_has_taxonomy_anchor(&request.query);
                                let threshold = if has_primary_lexical || has_domain_anchor {
                                    self.tuning.semantic_floor_with_lexical
                                } else {
                                    self.tuning.semantic_floor_without_lexical
                                };
                                if max_sim >= threshold {
                                    let related_margin = semantic_related_margin(
                                        query_term_count,
                                        has_primary_lexical,
                                        &self.tuning,
                                    );
                                    let candidate_threshold = (max_sim - related_margin)
                                        .max(self.tuning.semantic_candidate_floor);
                                    semantic_strong_cutoff = Some(
                                        (max_sim - self.tuning.semantic_strong_margin)
                                            .max(candidate_threshold),
                                    );
                                    semantic_hits = hits
                                        .into_iter()
                                        .filter(|hit| hit.score >= candidate_threshold)
                                        .collect();
                                    semantic_available = true;
                                } else {
                                    semantic_rejected_query = true;
                                }
                            }
                            Err(e) => {
                                log::warn!("Vector search error (falling back to lexical): {e}");
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("Query embedding error (falling back to lexical): {e}");
                    }
                }
            }
        }

        // Broad lexical recovery must not legitimize an unrelated query just
        // because two incidental words occur in one question. If Granite is
        // available and rejects the query, retain only primary all-term or
        // typo-prefix lexical matches.
        if semantic_rejected_query {
            lexical_hits.retain(|hit| !hit.relaxed);
        }

        // When no semantic index is available, relaxed OR-term matches are
        // not evidence of relevance on their own.  Keep only exhaustive
        // lexical matches so an unrelated multi-word query cannot surface a
        // random question merely because one incidental word occurs in it.
        if !semantic_available && !query_has_taxonomy_anchor(trimmed_query) {
            lexical_hits.retain(|hit| !hit.relaxed);
        }

        // A relaxed OR-term lexical hit is useful for recall only when the
        // semantic model independently supports it. Primary all-term lexical
        // matches remain exhaustive and are never removed here.
        if semantic_available {
            let semantic_ids = semantic_hits
                .iter()
                .map(|hit| hit.search_id as i64)
                .collect::<std::collections::HashSet<_>>();
            lexical_hits.retain(|hit| !hit.relaxed || semantic_ids.contains(&hit.search_id));
        }

        // 4. If neither returned results, return empty response
        if lexical_hits.is_empty() && semantic_hits.is_empty() {
            return Ok(SearchResponse {
                hits: Vec::new(),
                semantic_available,
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
        apply_exact_match_boosts(&mut fused, &request.query, &text_map);

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
        let has_primary_lexical = !primary_lexical_scores.is_empty();
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
                let strong_semantic = semantic_score.is_some_and(|score| {
                    semantic_strong_cutoff.is_some_and(|cutoff| score >= cutoff)
                });
                let numeric_constraints_supported = exact_numeric_pairs.is_empty()
                    || text_map.get(&fused_hit.search_id).is_some_and(|text| {
                        exact_numeric_pairs
                            .iter()
                            .all(|pair| contains_exact_token_pair(text, pair))
                    });
                // For short queries with exact lexical evidence elsewhere, a
                // semantic-only neighbour is context, not a strong answer.
                // Descriptive queries and semantic-only queries retain the
                // model's ability to establish strong conceptual matches.
                let semantic_can_establish_strength =
                    strong_semantic && (!has_primary_lexical || query_term_count >= 3);
                let match_strength = if core_ids.contains(&fused_hit.search_id)
                    && numeric_constraints_supported
                    && (strong_lexical
                        || (primary_lexical_match.is_some() && semantic_match)
                        || semantic_can_establish_strength)
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

        Ok(SearchResponse {
            hits,
            semantic_available,
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
        let trimmed = query.trim();
        let get_searched_count = |sections: &[String]| -> usize {
            if sections.is_empty() {
                conn.query_row("SELECT COUNT(*) FROM search_documents", [], |r| {
                    r.get::<_, i64>(0).map(|c| c as usize)
                })
                .unwrap_or(0)
            } else {
                let placeholders: Vec<String> = sections.iter().map(|_| "?".to_string()).collect();
                let sql = format!(
                    "SELECT COUNT(*) FROM search_documents WHERE section IN ({})",
                    placeholders.join(",")
                );
                let param_refs: Vec<&dyn rusqlite::ToSql> =
                    sections.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
                conn.query_row(&sql, param_refs.as_slice(), |r| {
                    r.get::<_, i64>(0).map(|c| c as usize)
                })
                .unwrap_or(0)
            }
        };

        let mut filters = crate::search::filters::SearchFilter::default();
        if let Some(sec_slice) = sections {
            filters.sections = sec_slice.to_vec();
        }

        let total_searched = get_searched_count(&filters.sections);

        if trimmed.is_empty() {
            return Ok(crate::backend::types::QuestionSearchResponse {
                query: query.to_string(),
                searched_questions: total_searched,
                total_matches: 0,
                results: Vec::new(),
            });
        }

        let request = SearchRequest {
            query: query.to_string(),
            filters: filters.clone(),
            // The UI asks retrieval to evaluate the complete active scope.
            // Relevance thresholds, not a fixed top-K, determine result count.
            limit: total_searched.max(1),
        };

        let response = self.search(conn, &request)?;
        if response.hits.is_empty() {
            return Ok(crate::backend::types::QuestionSearchResponse {
                query: query.to_string(),
                searched_questions: total_searched,
                total_matches: 0,
                results: Vec::new(),
            });
        }

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
        })
    }

    fn corrected_query(
        &self,
        conn: &Connection,
        query: &crate::search::lexical::query_builder::CompiledFtsQuery,
    ) -> Result<Option<String>, String> {
        let vocabulary = if let Some(vocabulary) = self.vocabulary.get() {
            vocabulary
        } else {
            let candidate = load_search_vocabulary(conn)?;
            let _ = self.vocabulary.set(candidate);
            self.vocabulary
                .get()
                .ok_or_else(|| "Failed to initialize search vocabulary".to_string())?
        };

        let mut changed = false;
        let corrected = query
            .terms()
            .iter()
            .map(|term| {
                let normalized = term.to_lowercase();
                if normalized.contains(' ')
                    || normalized.chars().count() < 4
                    || normalized.chars().any(|character| character.is_numeric())
                    || vocabulary.exact.contains(&normalized)
                {
                    return term.clone();
                }

                let max_distance = if normalized.chars().count() >= 8 {
                    2
                } else {
                    1
                };
                let length = normalized.chars().count();
                let correction = (length.saturating_sub(max_distance)..=length + max_distance)
                    .filter_map(|candidate_length| {
                        vocabulary.terms_by_length.get(&candidate_length)
                    })
                    .flatten()
                    .filter_map(|(candidate, frequency)| {
                        let distance = bounded_edit_distance(&normalized, candidate, max_distance)?;
                        Some((distance, std::cmp::Reverse(*frequency), candidate))
                    })
                    .min_by(|left, right| {
                        left.0
                            .cmp(&right.0)
                            .then_with(|| left.1.cmp(&right.1))
                            .then_with(|| left.2.cmp(right.2))
                    })
                    .map(|(_, _, candidate)| candidate.clone());

                if let Some(correction) = correction {
                    changed = true;
                    correction
                } else {
                    term.clone()
                }
            })
            .collect::<Vec<_>>();

        Ok(changed.then(|| corrected.join(" ")))
    }
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
         USING fts5vocab('main', 'question_fts', 'row');",
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
    for (left_index, left_character) in left.iter().enumerate() {
        current[0] = left_index + 1;
        let mut row_minimum = current[0];
        for (right_index, right_character) in right.iter().enumerate() {
            let substitution =
                previous[right_index] + usize::from(left_character != right_character);
            current[right_index + 1] = (current[right_index] + 1)
                .min(previous[right_index + 1] + 1)
                .min(substitution);
            row_minimum = row_minimum.min(current[right_index + 1]);
        }
        if row_minimum > maximum {
            return None;
        }
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
    let (_, suffix) = question_id.rsplit_once("_q")?;
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
    let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    fn append_in_clause(
        sql: &mut String,
        values: &mut Vec<Box<dyn rusqlite::ToSql>>,
        column: &str,
        items: &[String],
    ) {
        if items.is_empty() {
            return;
        }
        let start = values.len() + 1;
        let placeholders = (0..items.len())
            .map(|offset| format!("?{}", start + offset))
            .collect::<Vec<_>>()
            .join(",");
        sql.push_str(&format!(" AND {column} IN ({placeholders})"));
        values.extend(
            items
                .iter()
                .cloned()
                .map(|value| Box::new(value) as Box<dyn rusqlite::ToSql>),
        );
    }

    append_in_clause(&mut sql, &mut values, "d.section", &filters.sections);
    append_in_clause(&mut sql, &mut values, "d.stage", &filters.stages);
    append_in_clause(&mut sql, &mut values, "d.paper", &filters.papers);
    append_in_clause(&mut sql, &mut values, "d.bank_id", &filters.banks);

    if let Some((min_year, max_year)) = filters.years {
        let first = values.len() + 1;
        sql.push_str(&format!(" AND d.year BETWEEN ?{first} AND ?{}", first + 1));
        values.push(Box::new(min_year as i64));
        values.push(Box::new(max_year as i64));
    }

    if !filters.tags.is_empty() {
        let mut clauses = Vec::new();
        for tag in &filters.tags {
            if let Some(alias) = crate::taxonomy::legacy_main_tag_alias(tag) {
                let mut alias_clauses = Vec::new();
                if !alias.main_tags.is_empty() {
                    let start = values.len() + 1;
                    let placeholders = (0..alias.main_tags.len())
                        .map(|offset| format!("?{}", start + offset))
                        .collect::<Vec<_>>()
                        .join(",");
                    alias_clauses.push(format!("d.main_tag IN ({placeholders})"));
                    values.extend(
                        alias
                            .main_tags
                            .iter()
                            .cloned()
                            .map(|value| Box::new(value) as Box<dyn rusqlite::ToSql>),
                    );
                }
                if !alias.sections.is_empty() {
                    let start = values.len() + 1;
                    let placeholders = (0..alias.sections.len())
                        .map(|offset| format!("?{}", start + offset))
                        .collect::<Vec<_>>()
                        .join(",");
                    alias_clauses.push(format!("d.section IN ({placeholders})"));
                    values.extend(
                        alias
                            .sections
                            .iter()
                            .cloned()
                            .map(|value| Box::new(value) as Box<dyn rusqlite::ToSql>),
                    );
                }
                clauses.push(format!("({})", alias_clauses.join(" OR ")));
            } else {
                let parameter = values.len() + 1;
                clauses.push(format!(
                    "(d.main_tag = ?{parameter} OR EXISTS (\
                     SELECT 1 FROM question_taxonomy t, json_each(t.subtags_json) j \
                     WHERE t.question_id = d.question_id AND j.value = ?{parameter}))"
                ));
                values.push(Box::new(tag.clone()));
            }
        }
        sql.push_str(&format!(" AND ({})", clauses.join(" OR ")));
    }

    let params = values
        .iter()
        .map(|value| value.as_ref())
        .collect::<Vec<_>>();
    let mut stmt = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params.as_slice(), |row| row.get::<_, i64>(0))
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

    let placeholders: Vec<String> = search_ids.iter().map(|_| "?".to_string()).collect();
    let sql = format!(
        "SELECT search_id, question_id, question, options_text
         FROM search_documents WHERE search_id IN ({})",
        placeholders.join(",")
    );

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let params_refs: Vec<&dyn rusqlite::ToSql> = search_ids
        .iter()
        .map(|id| id as &dyn rusqlite::ToSql)
        .collect();

    let mut rows = stmt
        .query(params_refs.as_slice())
        .map_err(|e| e.to_string())?;
    let mut id_map = HashMap::new();
    let mut text_map = HashMap::new();

    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let sid: i64 = row.get(0).map_err(|e| e.to_string())?;
        let qid: String = row.get(1).map_err(|e| e.to_string())?;
        let question: String = row.get(2).map_err(|e| e.to_string())?;
        let options: String = row.get(3).map_err(|e| e.to_string())?;
        id_map.insert(sid, qid);
        text_map.insert(sid, format!("{question} {options}"));
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
        assert_eq!(source_question_number("custom-question-id"), None);
        assert_eq!(source_question_number("custom_qabc"), None);
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
        assert!(!resp.semantic_available);
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
        assert!(!resp.semantic_available);
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
