//! Offline, similarity-ranked search across every stored question.
//!
//! SQLite remains the canonical data store. Retrieval combines SQLite FTS5 Porter-stemmed
//! lexical indexing with dense int8 vector embeddings (Granite R2 GGUF Q8_0) and reciprocal
//! rank fusion.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use rusqlite::Connection;

use crate::backend::error::ResultExt;
use crate::search::vector::traits::VectorSearch;

use super::DbResult;
use crate::backend::error::LoopError;
use crate::backend::types::{Question, QuestionSearchResponse};

struct SearchDocumentSnapshot {
    search_id: u64,
    fingerprint: u64,
    text: String,
}

/// Short-lived SQLite snapshot used by the background index builder after the
/// database mutex has been released.
pub(crate) struct SearchRebuildSnapshot {
    epoch: u64,
    queued_jobs: usize,
    documents: Vec<SearchDocumentSnapshot>,
}

pub(crate) enum PreparedSearchRebuild {
    Noop {
        epoch: u64,
    },
    Staged {
        epoch: u64,
        changed: usize,
        generation: crate::search::indexing::generation::StagedGeneration,
    },
}

/// Lightweight wrapper for managed search index state.
#[derive(Clone)]
pub struct SearchIndexState {
    service: Arc<RwLock<Option<Arc<crate::search::service::SearchService>>>>,
    embedding_engine:
        Arc<RwLock<Option<Arc<dyn crate::search::embedding::engine::EmbeddingEngine>>>>,
    model_path: Option<PathBuf>,
    index_dir: PathBuf,
    bundled_vector_path: Option<PathBuf>,
    rebuild_running: Arc<AtomicBool>,
    mutation_epoch: Arc<AtomicU64>,
}

impl SearchIndexState {
    pub fn new(
        model_path: Option<PathBuf>,
        index_dir: PathBuf,
        bundled_vector_path: Option<PathBuf>,
    ) -> Self {
        Self {
            service: Arc::new(RwLock::new(None)),
            embedding_engine: Arc::new(RwLock::new(None)),
            model_path,
            index_dir,
            bundled_vector_path,
            rebuild_running: Arc::new(AtomicBool::new(false)),
            mutation_epoch: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn note_search_mutation(&self) -> bool {
        self.mutation_epoch.fetch_add(1, Ordering::SeqCst);
        self.try_begin_rebuild()
    }

    pub fn try_begin_rebuild(&self) -> bool {
        self.rebuild_running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    pub fn mutation_epoch(&self) -> u64 {
        self.mutation_epoch.load(Ordering::SeqCst)
    }

    pub fn finish_rebuild(&self) {
        self.rebuild_running.store(false, Ordering::SeqCst);
    }

    fn embedding_engine(
        &self,
    ) -> Option<Arc<dyn crate::search::embedding::engine::EmbeddingEngine>> {
        if let Ok(slot) = self.embedding_engine.read() {
            if let Some(engine) = slot.as_ref() {
                return Some(engine.clone());
            }
        }
        let engine = self
            .model_path
            .as_ref()
            .filter(|path| path.exists())
            .and_then(|path| {
                crate::search::embedding::llama_cpp::LlamaCppEmbeddingEngine::new(path)
                    .ok()
                    .map(|engine| {
                        Arc::new(engine)
                            as Arc<dyn crate::search::embedding::engine::EmbeddingEngine>
                    })
            })?;
        let mut slot = self.embedding_engine.write().ok()?;
        Some(slot.get_or_insert_with(|| engine.clone()).clone())
    }

    fn bundled_vector_index(&self) -> Option<crate::search::vector::flat::FlatExactVectorIndex> {
        let path = self
            .bundled_vector_path
            .as_ref()
            .filter(|path| path.exists())?;
        let manifest_path = path.parent()?.join("manifest.json");
        match crate::search::vector::flat::FlatExactVectorIndex::open_with_manifest(
            path,
            manifest_path,
        ) {
            Ok(index) => Some(index),
            Err(error) => {
                log::warn!("Ignoring incompatible bundled search index: {error}");
                None
            }
        }
    }

    fn active_vector_index(&self) -> Option<crate::search::vector::flat::FlatExactVectorIndex> {
        match crate::search::indexing::generation::GenerationManager::load_active_index(
            &self.index_dir,
        ) {
            Ok(index) => index,
            Err(error) => {
                log::warn!("Ignoring incompatible active search index: {error}");
                if let Err(clear_error) = crate::search::indexing::generation::GenerationManager::deactivate_active_generation(&self.index_dir) {
                    log::warn!("Could not deactivate incompatible active search index: {clear_error}");
                }
                None
            }
        }
    }
}

impl Default for SearchIndexState {
    fn default() -> Self {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        Self::new(
            Some(manifest_dir.join("models/granite-r2-q8_0.gguf")),
            manifest_dir.join("search-index"),
            Some(manifest_dir.join("models/search-index/generation-001/vectors.bin")),
        )
    }
}

pub fn search_questions_cached(
    conn: &Connection,
    state: &SearchIndexState,
    query: &str,
    sections: Option<&[String]>,
) -> DbResult<QuestionSearchResponse> {
    // Keep Granite and the mmap index alive across queries. Building a new
    // service per keystroke would repeatedly parse the bundled index and,
    // after the first semantic query, repeatedly load the GGUF model.
    let service = prepare_question_search(conn, state)?;

    Ok(service.execute_question_search(conn, query, sections)?)
}

/// Initialize and cache the search service without performing model inference.
/// Callers can release SQLite before invoking `SearchService::warm_embedding`.
pub fn prepare_question_search(
    conn: &Connection,
    state: &SearchIndexState,
) -> DbResult<Arc<crate::search::service::SearchService>> {
    let service = {
        let cached = state
            .service
            .read()
            .map_err(|_| LoopError::internal("Search service lock was poisoned"))?
            .clone();

        match cached {
            Some(service) => service,
            None => {
                let candidate = create_default_search_service(conn, state);
                let mut slot = state
                    .service
                    .write()
                    .map_err(|_| LoopError::internal("Search service lock was poisoned"))?;
                slot.get_or_insert_with(|| candidate.clone()).clone()
            }
        }
    };

    Ok(service)
}

pub fn invalidate_search_index(state: &SearchIndexState) -> DbResult<()> {
    *state
        .service
        .write()
        .map_err(|_| LoopError::internal("Search service lock was poisoned"))? = None;
    Ok(())
}

/// Creates a default `SearchService` populating vector index from active generation or bundled records.
pub fn create_default_search_service(
    conn: &Connection,
    state: &SearchIndexState,
) -> Arc<crate::search::service::SearchService> {
    // A record's search_id is a SQLite row ID. Never apply a generation built
    // for another database ordering; degrade to lexical search and rebuild.
    let vector_index = state
        .active_vector_index()
        .or_else(|| state.bundled_vector_index())
        .filter(|index| vector_index_matches_database(conn, index))
        .or_else(|| {
            state
                .bundled_vector_index()
                .filter(|index| vector_index_matches_database(conn, index))
        })
        .map(|index| Arc::new(index) as Arc<dyn crate::search::vector::traits::VectorSearch>);

    let engine = state.embedding_engine();

    Arc::new(crate::search::service::SearchService::new(
        engine,
        vector_index,
    ))
}

fn matching_record_count(
    index: &crate::search::vector::flat::FlatExactVectorIndex,
    fingerprints: &HashMap<u64, u64>,
) -> usize {
    let record_count = crate::search::vector::traits::VectorSearch::count(index);
    (0..record_count)
        .filter_map(|record_index| index.get_record(record_index).ok())
        .filter(|record| fingerprints.get(&record.search_id) == Some(&record.fingerprint))
        .count()
}

fn vector_index_matches_database(
    conn: &Connection,
    index: &crate::search::vector::flat::FlatExactVectorIndex,
) -> bool {
    let Ok(mut stmt) = conn.prepare("SELECT search_id, content_fingerprint FROM search_documents")
    else {
        return false;
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)? as u64, row.get::<_, Vec<u8>>(1)?))
    }) else {
        return false;
    };
    let fingerprints = rows
        .flatten()
        .filter_map(|(search_id, bytes)| {
            let fingerprint = u64::from_le_bytes(bytes.as_slice().try_into().ok()?);
            Some((search_id, fingerprint))
        })
        .collect::<HashMap<_, _>>();

    if index.count() != fingerprints.len() {
        return false;
    }

    let mut seen_search_ids = std::collections::HashSet::with_capacity(index.count());
    index.record_metadata().all(|(search_id, fingerprint)| {
        seen_search_ids.insert(search_id) && fingerprints.get(&search_id) == Some(&fingerprint)
    })
}

/// Copy the canonical search rows while holding SQLite, then release the
/// connection before model inference or generation-file I/O begins.
pub(crate) fn capture_search_rebuild(
    conn: &Connection,
    state: &SearchIndexState,
) -> DbResult<SearchRebuildSnapshot> {
    let queued_jobs = conn
        .query_row(
            "SELECT COUNT(*) FROM search_index_jobs
             WHERE status IN ('pending', 'running', 'failed')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .stringify_err()? as usize;
    let mut stmt = conn
        .prepare(
            "SELECT search_id, question, options_text, content_fingerprint
             FROM search_documents ORDER BY search_id",
        )
        .stringify_err()?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)? as u64,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })
        .stringify_err()?;

    let mut documents = Vec::new();
    for row in rows {
        let (search_id, question, options, fingerprint_bytes) = row.stringify_err()?;
        let text = if options.is_empty() {
            question
        } else {
            format!("{question}\n{options}")
        };
        let fingerprint = fingerprint_bytes
            .as_slice()
            .try_into()
            .ok()
            .map(u64::from_le_bytes)
            .unwrap_or_else(|| crate::search::indexing::fingerprint::content_fingerprint(&text));
        documents.push(SearchDocumentSnapshot {
            search_id,
            fingerprint,
            text,
        });
    }

    Ok(SearchRebuildSnapshot {
        epoch: state.mutation_epoch(),
        queued_jobs,
        documents,
    })
}

/// Embed changed documents and write an unpublished immutable generation.
/// This function deliberately has no SQLite connection parameter.
pub(crate) fn prepare_search_rebuild(
    snapshot: SearchRebuildSnapshot,
    state: &SearchIndexState,
) -> DbResult<PreparedSearchRebuild> {
    let active_index = state.active_vector_index();
    let bundled_index = state.bundled_vector_index();
    let has_compatible_active_index = active_index.is_some();
    let fingerprints = snapshot
        .documents
        .iter()
        .map(|document| (document.search_id, document.fingerprint))
        .collect::<HashMap<_, _>>();
    let seed_index = match (active_index, bundled_index) {
        (Some(active), Some(bundled)) => {
            if matching_record_count(&bundled, &fingerprints)
                > matching_record_count(&active, &fingerprints)
            {
                Some(bundled)
            } else {
                Some(active)
            }
        }
        (Some(active), None) => Some(active),
        (None, Some(bundled)) => Some(bundled),
        (None, None) => None,
    };
    let mut existing = seed_index
        .as_ref()
        .and_then(|index| index.records().ok())
        .unwrap_or_default()
        .into_iter()
        .map(|record| (record.search_id, record))
        .collect::<HashMap<_, _>>();

    let mut records = Vec::new();
    let mut pending = Vec::new();
    for document in snapshot.documents {
        if let Some(record) = existing.remove(&document.search_id) {
            if record.fingerprint == document.fingerprint {
                records.push(record);
                continue;
            }
        }
        pending.push((document.search_id, document.fingerprint, document.text));
    }

    if has_compatible_active_index
        && pending.is_empty()
        && existing.is_empty()
        && snapshot.queued_jobs == 0
    {
        return Ok(PreparedSearchRebuild::Noop {
            epoch: snapshot.epoch,
        });
    }

    if !pending.is_empty() {
        let engine = state.embedding_engine().ok_or_else(|| {
            "Granite model is unavailable; lexical search remains active".to_string()
        })?;
        for batch in pending.chunks(32) {
            let texts = batch
                .iter()
                .map(|(_, _, text)| text.clone())
                .collect::<Vec<_>>();
            let embeddings = engine.embed_documents(&texts).map_err(|error| {
                LoopError::unavailable(format!("Granite indexing failed: {error}"))
            })?;
            for ((search_id, fingerprint, _), embedding) in batch.iter().zip(embeddings) {
                records.push(crate::search::vector::format::VectorRecord::from_embedding(
                    *search_id,
                    *fingerprint,
                    0,
                    &embedding,
                )?);
            }
        }
    }
    records.sort_unstable_by_key(|record| record.search_id);

    let next_generation = seed_index
        .as_ref()
        .map(|index| {
            crate::search::vector::traits::VectorSearch::generation(index).saturating_add(1)
        })
        .unwrap_or(1);
    let generation = crate::search::indexing::generation::GenerationManager::stage_generation(
        &state.index_dir,
        next_generation,
        crate::search::vector::manifest::GRANITE_MODEL_REVISION,
        snapshot.epoch,
        &records,
    )?;

    Ok(PreparedSearchRebuild::Staged {
        epoch: snapshot.epoch,
        changed: pending.len(),
        generation,
    })
}

/// Publish a prepared generation only if no import/delete happened after its
/// SQLite snapshot. Returns `None` when stale work was safely discarded.
pub(crate) fn commit_search_rebuild(
    conn: &Connection,
    state: &SearchIndexState,
    prepared: PreparedSearchRebuild,
) -> DbResult<Option<usize>> {
    match prepared {
        PreparedSearchRebuild::Noop { epoch } => {
            if state.mutation_epoch() == epoch {
                Ok(Some(0))
            } else {
                Ok(None)
            }
        }
        PreparedSearchRebuild::Staged {
            epoch,
            changed,
            generation,
        } => {
            if state.mutation_epoch() != epoch {
                crate::search::indexing::generation::GenerationManager::discard_staged_generation(
                    generation,
                );
                return Ok(None);
            }
            crate::search::indexing::generation::GenerationManager::activate_staged_generation(
                generation,
            )?;
            conn.execute("DELETE FROM search_index_jobs", [])
                .stringify_err()?;
            invalidate_search_index(state)?;
            Ok(Some(changed))
        }
    }
}

/// Synchronous convenience wrapper used by focused tests. Production uses the
/// split capture/prepare/commit flow so the shared DB mutex is not retained.
#[cfg(test)]
pub fn rebuild_search_index(conn: &Connection, state: &SearchIndexState) -> DbResult<usize> {
    let snapshot = capture_search_rebuild(conn, state)?;
    let prepared = prepare_search_rebuild(snapshot, state)?;
    Ok(commit_search_rebuild(conn, state, prepared)?.unwrap_or(0))
}

/// Return the broad UPSC category used by semantic search for each question.
pub fn question_main_tags(
    conn: &Connection,
    _state: &SearchIndexState,
    questions: &[Question],
) -> DbResult<HashMap<String, String>> {
    if questions.is_empty() {
        return Ok(HashMap::new());
    }

    let qids: Vec<&str> = questions.iter().map(|q| q.id.as_str()).collect();
    let placeholders: Vec<String> = qids.iter().map(|_| "?".to_string()).collect();
    let sql = format!(
        "SELECT question_id, main_tag FROM question_taxonomy WHERE question_id IN ({})",
        placeholders.join(",")
    );

    let mut stmt = conn.prepare(&sql).stringify_err()?;
    let param_refs: Vec<&dyn rusqlite::ToSql> =
        qids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

    let mut map = HashMap::new();
    let mut rows = stmt.query(param_refs.as_slice()).stringify_err()?;
    while let Some(row) = rows.next().stringify_err()? {
        let qid: String = row.get(0).stringify_err()?;
        let main_tag: String = row.get(1).stringify_err()?;
        map.insert(qid, main_tag);
    }
    Ok(map)
}

/// Return the broad taxonomy category and subtags for each question.
pub fn question_taxonomy_tags(
    conn: &Connection,
    _state: &SearchIndexState,
    questions: &[Question],
) -> DbResult<HashMap<String, Vec<String>>> {
    if questions.is_empty() {
        return Ok(HashMap::new());
    }

    let qids: Vec<&str> = questions.iter().map(|q| q.id.as_str()).collect();
    let placeholders: Vec<String> = qids.iter().map(|_| "?".to_string()).collect();
    let sql = format!(
        "SELECT question_id, main_tag, subtags_json FROM question_taxonomy WHERE question_id IN ({})",
        placeholders.join(",")
    );

    let mut stmt = conn.prepare(&sql).stringify_err()?;
    let param_refs: Vec<&dyn rusqlite::ToSql> =
        qids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

    let mut map = HashMap::new();
    let mut rows = stmt.query(param_refs.as_slice()).stringify_err()?;
    while let Some(row) = rows.next().stringify_err()? {
        let qid: String = row.get(0).stringify_err()?;
        let main_tag: String = row.get(1).stringify_err()?;
        let subtags_json: String = row.get(2).stringify_err()?;
        let mut tags = Vec::new();
        if !main_tag.is_empty() {
            tags.push(main_tag);
        }
        if let Ok(subtags) = serde_json::from_str::<Vec<String>>(&subtags_json) {
            tags.extend(subtags);
        }
        map.insert(qid, tags);
    }
    Ok(map)
}

#[cfg(test)]
pub fn semantic_index_freshness(conn: &Connection) -> DbResult<(usize, usize)> {
    let total: usize = conn
        .query_row("SELECT COUNT(*) FROM search_documents", [], |r| {
            r.get::<_, i64>(0).map(|c| c as usize)
        })
        .stringify_err()?;
    let with_fp: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM search_documents WHERE content_fingerprint IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0).map(|c| c as usize),
        )
        .stringify_err()?;
    Ok((with_fp, total))
}

#[cfg(test)]
pub fn semantic_tag_coverage(conn: &Connection) -> DbResult<(usize, usize)> {
    let total: usize = conn
        .query_row("SELECT COUNT(*) FROM search_documents", [], |r| {
            r.get::<_, i64>(0).map(|c| c as usize)
        })
        .stringify_err()?;
    let tagged: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM search_documents WHERE main_tag IS NOT NULL AND main_tag != ''",
            [],
            |r| r.get::<_, i64>(0).map(|c| c as usize),
        )
        .stringify_err()?;
    Ok((tagged, total))
}

#[cfg(test)]
mod rebuild_tests {
    use super::*;
    use crate::backend::db::schema::run_migrations;
    use crate::search::vector::format::{VectorRecord, VECTOR_DIMS};
    use crate::search::vector::traits::VectorSearch;

    fn pending_jobs(conn: &Connection) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM search_index_jobs WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn database_match_rejects_duplicate_vector_search_ids() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO question_banks
             (id, name, exam, metadata, total_questions, difficulty, default_duration, imported_at)
             VALUES ('b', 'Bank', 'UPSC', '{}', 2, 'medium', 60, 1)",
            [],
        )
        .unwrap();
        for (question_id, question) in [("q1", "First?"), ("q2", "Second?")] {
            conn.execute(
                "INSERT INTO questions (id, bank_id, type, question, correct_answers, marks)
                 VALUES (?1, 'b', 'single', ?2, '[]', 1)",
                rusqlite::params![question_id, question],
            )
            .unwrap();
            let fingerprint = crate::search::indexing::fingerprint::content_fingerprint(question);
            conn.execute(
                "INSERT INTO search_documents
                 (question_id, question, bank_id, bank_name, content_fingerprint)
                 VALUES (?1, ?2, 'b', 'Bank', ?3)",
                rusqlite::params![question_id, question, &fingerprint.to_le_bytes()[..]],
            )
            .unwrap();
        }

        let (search_id, fingerprint): (u64, u64) = conn
            .query_row(
                "SELECT search_id, content_fingerprint FROM search_documents WHERE question_id = 'q1'",
                [],
                |row| {
                    let search_id = row.get::<_, i64>(0)? as u64;
                    let bytes = row.get::<_, Vec<u8>>(1)?;
                    Ok((
                        search_id,
                        u64::from_le_bytes(bytes.as_slice().try_into().unwrap()),
                    ))
                },
            )
            .unwrap();
        let vector = vec![0.0; VECTOR_DIMS];
        let duplicate = VectorRecord::from_embedding(search_id, fingerprint, 0, &vector).unwrap();
        let path = std::env::temp_dir().join(format!(
            "preploop_duplicate_ids_{}.bin",
            uuid::Uuid::new_v4()
        ));
        let index = crate::search::vector::flat::FlatExactVectorIndex::write_new(
            &path,
            1,
            crate::search::vector::manifest::GRANITE_MODEL_REVISION,
            &[duplicate.clone(), duplicate],
        )
        .unwrap();

        assert!(!vector_index_matches_database(&conn, &index));
        drop(index);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn generation_rebuild_embeds_changes_and_reuses_matching_records() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO question_banks
             (id, name, exam, metadata, total_questions, difficulty, default_duration, imported_at)
             VALUES ('b', 'Bank', 'UPSC', '{}', 1, 'medium', 60, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO questions (id, bank_id, type, question, correct_answers, marks)
             VALUES ('q', 'b', 'single', 'How does inflation affect purchasing power?', '[]', 1)",
            [],
        )
        .unwrap();
        let text = "How does inflation affect purchasing power?";
        let fingerprint = crate::search::indexing::fingerprint::content_fingerprint(text);
        conn.execute(
            "INSERT INTO search_documents
             (question_id, question, bank_id, bank_name, content_fingerprint)
             VALUES ('q', ?1, 'b', 'Bank', ?2)",
            rusqlite::params![text, &fingerprint.to_le_bytes()[..]],
        )
        .unwrap();

        let temp_dir =
            std::env::temp_dir().join(format!("preploop_rebuild_{}", uuid::Uuid::new_v4()));
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let state = SearchIndexState::new(
            Some(manifest_dir.join("models/granite-r2-q8_0.gguf")),
            temp_dir.clone(),
            None,
        );

        assert_eq!(rebuild_search_index(&conn, &state).unwrap(), 1);
        let index =
            crate::search::indexing::generation::GenerationManager::load_active_index(&temp_dir)
                .unwrap()
                .unwrap();
        assert_eq!(index.count(), 1);
        assert_eq!(pending_jobs(&conn), 0);
        assert_eq!(rebuild_search_index(&conn, &state).unwrap(), 0);

        let changed_text = "How does inflation reduce real purchasing power?";
        let changed_fingerprint =
            crate::search::indexing::fingerprint::content_fingerprint(changed_text);
        conn.execute(
            "UPDATE search_documents
             SET question = ?1, content_fingerprint = ?2
             WHERE question_id = 'q'",
            rusqlite::params![changed_text, &changed_fingerprint.to_le_bytes()[..]],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO search_index_jobs
             (question_id, operation, status, attempts, created_at, updated_at)
             VALUES ('q', 'embed', 'pending', 0, 1, 1)",
            [],
        )
        .unwrap();
        assert!(state.note_search_mutation());
        let snapshot = capture_search_rebuild(&conn, &state).unwrap();
        let prepared = prepare_search_rebuild(snapshot, &state).unwrap();
        assert!(!state.note_search_mutation());
        assert_eq!(
            commit_search_rebuild(&conn, &state, prepared).unwrap(),
            None
        );
        assert_eq!(pending_jobs(&conn), 1);
        assert_eq!(
            crate::search::indexing::generation::GenerationManager::get_active_generation(
                &temp_dir
            ),
            Some("generation-001".to_string())
        );

        state.finish_rebuild();
        assert_eq!(rebuild_search_index(&conn, &state).unwrap(), 1);
        assert_eq!(pending_jobs(&conn), 0);

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn generation_rebuild_prefers_a_matching_bundled_index_over_stale_active_data() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO question_banks
             (id, name, exam, metadata, total_questions, difficulty, default_duration, imported_at)
             VALUES ('b', 'Bank', 'UPSC', '{}', 1, 'medium', 60, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO questions (id, bank_id, type, question, correct_answers, marks)
             VALUES ('q', 'b', 'single', 'Current question text', '[]', 1)",
            [],
        )
        .unwrap();
        let text = "Current question text";
        let fingerprint = crate::search::indexing::fingerprint::content_fingerprint(text);
        conn.execute(
            "INSERT INTO search_documents
             (question_id, question, bank_id, bank_name, content_fingerprint)
             VALUES ('q', ?1, 'b', 'Bank', ?2)",
            rusqlite::params![text, &fingerprint.to_le_bytes()[..]],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO search_index_jobs
             (question_id, operation, status, created_at, updated_at)
             VALUES ('q', 'embed', 'pending', 1, 1)",
            [],
        )
        .unwrap();

        let temp_dir =
            std::env::temp_dir().join(format!("preploop_rebuild_{}", uuid::Uuid::new_v4()));
        crate::search::indexing::generation::GenerationManager::build_and_swap_generation(
            &temp_dir,
            1,
            "2ab6fa8ea2d674564defd37171ae19079b864b33",
            1,
            &[],
        )
        .unwrap();

        let bundled_path = temp_dir.join("bundled.bin");
        let mut embedding = vec![0.0; crate::search::vector::format::VECTOR_DIMS];
        embedding[0] = 1.0;
        let bundled_record = crate::search::vector::format::VectorRecord::from_embedding(
            1,
            fingerprint,
            0,
            &embedding,
        )
        .unwrap();
        crate::search::vector::flat::FlatExactVectorIndex::write_new(
            &bundled_path,
            1,
            "2ab6fa8ea2d674564defd37171ae19079b864b33",
            &[bundled_record],
        )
        .unwrap();
        crate::search::vector::manifest::VectorManifest::new_granite_q8(1, 1)
            .save(temp_dir.join("manifest.json"))
            .unwrap();

        let state = SearchIndexState::new(None, temp_dir.clone(), Some(bundled_path));
        assert_eq!(rebuild_search_index(&conn, &state).unwrap(), 0);
        let active =
            crate::search::indexing::generation::GenerationManager::load_active_index(&temp_dir)
                .unwrap()
                .unwrap();
        assert_eq!(active.count(), 1);
        assert_eq!(pending_jobs(&conn), 0);

        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
