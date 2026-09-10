use super::*;
use crate::backend::db::schema::run_migrations;
use crate::search::embedding::engine::{Embedding, EmbeddingEngine, EmbeddingError};
use crate::search::indexing::fingerprint::content_fingerprint;
use crate::search::indexing::generation::GenerationManager;
use crate::search::response::SemanticStatus;
use crate::search::vector::flat::FlatExactVectorIndex;
use crate::search::vector::format::{VectorRecord, VECTOR_DIMS};
use crate::search::vector::manifest::{VectorManifest, GRANITE_MODEL_REVISION};
use std::sync::Mutex;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("preploop_lifecycle_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[derive(Default)]
struct RecordingEngine {
    documents: Mutex<Vec<String>>,
}

fn unit_vector() -> Embedding {
    let mut vector = vec![0.0; VECTOR_DIMS];
    vector[0] = 1.0;
    vector
}

impl EmbeddingEngine for RecordingEngine {
    fn dimensions(&self) -> usize {
        VECTOR_DIMS
    }

    fn embed_query(&self, _: &str) -> Result<Embedding, EmbeddingError> {
        Ok(unit_vector())
    }

    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Embedding>, EmbeddingError> {
        self.documents.lock().unwrap().extend_from_slice(texts);
        Ok(texts.iter().map(|_| unit_vector()).collect())
    }
}

fn record(id: u64, text: &str) -> VectorRecord {
    VectorRecord::from_embedding(id, content_fingerprint(text), 0, &unit_vector()).unwrap()
}

fn write_bundle(dir: &TestDirectory, records: &[VectorRecord]) -> PathBuf {
    let path = dir.0.join("bundled.bin");
    FlatExactVectorIndex::write_new(&path, 1, GRANITE_MODEL_REVISION, records).unwrap();
    VectorManifest::new_granite_q8(records.len() as u64, 1)
        .save(dir.0.join("manifest.json"))
        .unwrap();
    path
}

fn database() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn).unwrap();
    conn.execute(
        "INSERT INTO question_banks
         (id, name, exam, metadata, total_questions, difficulty, default_duration, imported_at)
         VALUES ('bank', 'Bank', 'UPSC', '{}', 3, 'medium', 60, 1)",
        [],
    )
    .unwrap();
    conn
}

fn insert_document(conn: &Connection, id: u64, text: &str) {
    let question_id = format!("q{id}");
    conn.execute(
        "INSERT INTO questions (id, bank_id, type, question, correct_answers, marks)
         VALUES (?1, 'bank', 'single', ?2, '[]', 1)",
        rusqlite::params![question_id, text],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO search_documents
         (search_id, question_id, question, bank_id, bank_name, section, content_fingerprint)
         VALUES (?1, ?2, ?3, 'bank', 'Bank', 'prelims-gs1', ?4)",
        rusqlite::params![
            id as i64,
            question_id,
            text,
            &content_fingerprint(text).to_le_bytes()[..]
        ],
    )
    .unwrap();
}

#[test]
fn related_search_reuses_shifted_ids_before_rebuild_with_an_unindexed_import() {
    let dir = TestDirectory::new();
    let conn = database();
    insert_document(&conn, 101, "Water conservation and irrigation");
    insert_document(&conn, 102, "Water conservation and irrigation");
    insert_document(&conn, 103, "A newly imported question");
    let bundle = write_bundle(&dir, &[record(1, "Water conservation and irrigation")]);
    let state = SearchIndexState::new(None, dir.0.join("runtime"), Some(bundle));
    let engine = Arc::new(RecordingEngine::default());
    *state.embedding_engine.write().unwrap() = Some(engine.clone());

    let response = search_questions_cached(&conn, &state, "water", None).unwrap();
    assert_eq!(response.semantic_status, SemanticStatus::Available);
    for id in ["q101", "q102"] {
        assert!(response
            .results
            .iter()
            .any(|hit| hit.question_id == id && hit.semantic_match));
    }
    assert!(engine.documents.lock().unwrap().is_empty());
    assert!(
        !state.index_dir.exists(),
        "search should use the bundle without writing a runtime index"
    );
}

#[test]
fn update_combines_bundled_and_custom_vectors_and_only_embeds_new_content() {
    let dir = TestDirectory::new();
    let conn = database();
    insert_document(&conn, 101, "Water conservation and irrigation");
    insert_document(&conn, 201, "My custom water question");
    insert_document(&conn, 301, "A genuinely new question");
    let bundle = write_bundle(&dir, &[record(1, "Water conservation and irrigation")]);
    let runtime = dir.0.join("runtime");
    let previous = GenerationManager::build_and_swap_generation(
        &runtime,
        5,
        GRANITE_MODEL_REVISION,
        1,
        &[record(201, "My custom water question")],
    )
    .unwrap();
    let state = SearchIndexState::new(None, runtime.clone(), Some(bundle));
    let engine = Arc::new(RecordingEngine::default());
    *state.embedding_engine.write().unwrap() = Some(engine.clone());

    assert_eq!(rebuild_search_index(&conn, &state).unwrap(), 1);
    assert_eq!(
        *engine.documents.lock().unwrap(),
        ["A genuinely new question"]
    );
    let active = GenerationManager::load_active_index(&runtime)
        .unwrap()
        .unwrap();
    assert!(vector_index_matches_database(&conn, &active));
    assert_eq!(active.generation(), 6);
    assert_eq!(
        previous.get_record(0).unwrap(),
        record(201, "My custom water question")
    );
    assert_eq!(rebuild_search_index(&conn, &state).unwrap(), 0);
}

#[test]
fn bundled_reuse_never_overwrites_an_existing_generation() {
    let dir = TestDirectory::new();
    let conn = database();
    insert_document(&conn, 1, "Current question");
    let runtime = dir.0.join("runtime");
    let previous = GenerationManager::build_and_swap_generation(
        &runtime,
        2,
        GRANITE_MODEL_REVISION,
        1,
        &[record(1, "Old question")],
    )
    .unwrap();
    // An interrupted older build may also have left a generation on disk.
    let orphan =
        GenerationManager::stage_generation(&runtime, 3, GRANITE_MODEL_REVISION, 1, &[]).unwrap();
    let bundle = write_bundle(&dir, &[record(1, "Current question")]);
    let state = SearchIndexState::new(None, runtime.clone(), Some(bundle));
    assert_eq!(rebuild_search_index(&conn, &state).unwrap(), 0);
    let active = GenerationManager::load_active_index(&runtime)
        .unwrap()
        .unwrap();
    assert_eq!(active.generation(), 4);
    let old_on_disk = FlatExactVectorIndex::open(previous.path()).unwrap();
    assert_eq!(
        old_on_disk.get_record(0).unwrap(),
        record(1, "Old question")
    );
    GenerationManager::discard_staged_generation(orphan);
}

#[test]
fn full_bundled_corpus_reuses_vectors_after_update_and_restart_without_a_model() {
    use crate::backend::db::question_bank::{import_question_bank, sync_bundled_question_bank};
    use crate::backend::types::{QuestionBank, TestMode};
    use sha2::{Digest, Sha256};

    let dir = TestDirectory::new();
    let db_path = dir.0.join("loop.db");
    let mut conn = Connection::open(&db_path).unwrap();
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    run_migrations(&conn).unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let corpus = root.join("../static/upsc");
    let catalog: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(corpus.join("catalog.json")).unwrap())
            .unwrap();
    let papers = catalog["papers"].as_array().unwrap();
    let first = &papers[0];
    let old_bank: QuestionBank = serde_json::from_str(
        &std::fs::read_to_string(corpus.join(first["path"].as_str().unwrap())).unwrap(),
    )
    .unwrap();
    let old_question_id = old_bank.questions[0].id.clone();
    let old_bank_id = import_question_bank(&mut conn, &old_bank).unwrap();
    let attempt = crate::backend::db::attempt::create_test_attempt(
        &mut conn,
        &old_bank_id,
        TestMode::Test,
        None,
    )
    .unwrap();
    crate::backend::db::attempt::save_answer(
        &mut conn,
        &attempt,
        &old_question_id,
        Some(&serde_json::json!("a")),
    )
    .unwrap();
    crate::backend::db::attempt::finalize_submission(&conn, &attempt, 0.0, 1.0, 1, None).unwrap();

    // Real bundled sync retains the historical revision and creates new row
    // and question IDs. Reverse import order also differs from index order.
    for paper in papers.iter().rev() {
        let json = std::fs::read_to_string(corpus.join(paper["path"].as_str().unwrap())).unwrap();
        let bank: QuestionBank = serde_json::from_str(&json).unwrap();
        let key = format!(
            "{}:{}:{}",
            paper["section"].as_str().unwrap(),
            paper["year"],
            paper["paper"].as_str().unwrap()
        );
        sync_bundled_question_bank(
            &mut conn,
            &key,
            &format!("{:x}", Sha256::digest(json.as_bytes())),
            catalog["contentVersion"].as_i64().unwrap(),
            paper["contentVersion"]
                .as_i64()
                .unwrap_or(catalog["contentVersion"].as_i64().unwrap()),
            bank,
        )
        .unwrap();
    }
    let bundle = root.join("models/search-index/generation-001/vectors.bin");
    let runtime = dir.0.join("runtime");
    let state = SearchIndexState::new(None, runtime.clone(), Some(bundle.clone()));
    assert_eq!(
        rebuild_search_index(&conn, &state).unwrap(),
        0,
        "a corpus update must reuse bundled vectors even with different local IDs"
    );
    let active = GenerationManager::load_active_index(&runtime)
        .unwrap()
        .unwrap();
    let bundled = FlatExactVectorIndex::open(&bundle).unwrap();
    assert_eq!(active.count(), bundled.count());
    assert!(vector_index_matches_database(&conn, &active));
    let generation = active.generation();
    drop((active, bundled, conn, state));

    let conn = Connection::open(&db_path).unwrap();
    let state = SearchIndexState::new(None, runtime.clone(), Some(bundle));
    assert_eq!(rebuild_search_index(&conn, &state).unwrap(), 0);
    let active = GenerationManager::load_active_index(&runtime)
        .unwrap()
        .unwrap();
    assert_eq!(active.generation(), generation);
    let history = crate::backend::db::attempt::list_test_attempt_history(&conn).unwrap();
    assert_eq!(history.len(), 1);
    let answers =
        crate::backend::db::attempt::fetch_responses_by_attempt_id(&conn, &attempt).unwrap();
    assert_eq!(answers.len(), 1);
}
