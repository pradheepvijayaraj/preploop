//! Tauri command handlers.
//!
//! Each function is annotated with `#[tauri::command]` and registered in
//! `lib.rs`.  Commands receive a shared `DbState` via Tauri's managed-state
//! mechanism (#13 / #21) instead of opening a fresh connection per call.

use rusqlite::TransactionBehavior;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use tauri::State;

use super::db;
use super::db::{DbState, SearchIndexState};
use super::error::{LoopError, LoopResult, ResultExt};
use super::scoring;
use super::session;
use super::types::{
    ArchiveMissingBundledQuestionBanksArgs, AttemptIdArgs, BankIdArgs,
    BundledQuestionBankSyncResult, CreateTestAttemptArgs, ImportQuestionBankArgs, ImportResult,
    LoadedSessionPayload, PracticeQuestionFeedback, PracticeQuestionFeedbackArgs, Question,
    QuestionBankWithQuestions, RefreshQuestionBankTaxonomyArgs, ResponseState, SaveAnswerArgs,
    SaveSettingsArgs, SearchQuestionsArgs, Settings, StoredQuestionBank, SubmitResult,
    SubmitTestArgs, SyncBundledQuestionBankArgs, TestAttempt, TestAttemptHistoryEntry, TestResult,
    TestStatus, ToggleFlagArgs, UpdateTimeArgs, ValidationError,
};
use super::validation;

const MAX_IMPORT_JSON_BYTES: usize = 16 * 1024 * 1024;
const MAX_SEARCH_QUERY_CHARS: usize = 512;
const MAX_SEARCH_SECTIONS: usize = 32;
const MAX_SEARCH_SECTION_CHARS: usize = 128;
const MAX_IDENTIFIER_CHARS: usize = 256;
const MAX_ANSWER_JSON_BYTES: usize = 1024 * 1024;
const MAX_BUNDLED_CATALOG_KEYS: usize = 4096;

// ── Helper: acquire the connection from managed state ───────────────────

/// Acquire a lock on the shared database connection.
///
/// Takes `&DbState` (not `&State<DbState>`) to avoid lifetime ambiguity.
/// This works because `State<'_, T>` implements `Deref<Target = T>`, so
/// call sites pass `conn(&db)` and Rust auto-deref-coerces the reference.
///
/// Returns a user-friendly error if the mutex is poisoned (should never
/// happen in practice — panics inside a MutexGuard are the only cause,
/// and we never panic in DB code).
fn conn(db: &DbState) -> LoopResult<std::sync::MutexGuard<'_, rusqlite::Connection>> {
    db.0.lock()
        .map_err(|_| LoopError::internal("Database mutex was poisoned"))
}

/// Debounce database mutations and rebuild one immutable vector generation in
/// the background. Repeated bundled-paper imports collapse into one rebuild.
pub(crate) fn schedule_search_rebuild(db: DbState, search_index: SearchIndexState) {
    if !search_index.note_search_mutation() {
        return;
    }
    tauri::async_runtime::spawn_blocking(move || loop {
        let quiet_epoch = search_index.mutation_epoch();
        std::thread::sleep(std::time::Duration::from_millis(400));
        if search_index.mutation_epoch() != quiet_epoch {
            continue;
        }

        let result = (|| {
            // SQLite is held only long enough to copy canonical rows.
            let snapshot = {
                let connection = conn(&db)?;
                db::capture_search_rebuild(&connection, &search_index)?
            };

            // Model inference and generation-file writes run with no DB guard.
            let prepared = db::prepare_search_rebuild(snapshot, &search_index)?;

            // Reacquire briefly to validate the mutation epoch and publish.
            let committed = {
                let connection = conn(&db)?;
                db::commit_search_rebuild(&connection, &search_index, prepared)?
            };
            if let Some(count) = committed {
                log::info!("Search generation rebuilt; embedded {count} changed questions");
            } else {
                log::info!("Discarded a stale search generation; scheduling the newer snapshot");
            }
            Ok::<(), LoopError>(())
        })();
        if let Err(error) = result {
            log::warn!("Background semantic indexing deferred: {error}");
        }

        search_index.finish_rebuild();
        if search_index.mutation_epoch() == quiet_epoch || !search_index.try_begin_rebuild() {
            break;
        }
    });
}

struct SubmissionContext {
    attempt: TestAttempt,
    questions: Vec<Question>,
    main_tags: HashMap<String, String>,
    analysis: scoring::SubmissionAnalysis,
}

/// Narrow data-access boundary for submission analysis. Command handlers use
/// the SQLite implementation, while scoring orchestration can be unit-tested
/// with an in-memory fake without constructing a database.
trait SubmissionRepository {
    fn attempt(&self, attempt_id: &str) -> LoopResult<Option<TestAttempt>>;
    fn questions(&self, bank_id: &str) -> LoopResult<Vec<Question>>;
    fn responses(&self, attempt_id: &str) -> LoopResult<Vec<ResponseState>>;
    fn main_tags(
        &self,
        search_index: &SearchIndexState,
        questions: &[Question],
    ) -> LoopResult<HashMap<String, String>>;
}

impl SubmissionRepository for rusqlite::Connection {
    fn attempt(&self, attempt_id: &str) -> LoopResult<Option<TestAttempt>> {
        db::fetch_test_attempt(self, attempt_id)
    }

    fn questions(&self, bank_id: &str) -> LoopResult<Vec<Question>> {
        db::fetch_questions_by_bank_id(self, bank_id)
    }

    fn responses(&self, attempt_id: &str) -> LoopResult<Vec<ResponseState>> {
        db::fetch_responses_by_attempt_id(self, attempt_id)
    }

    fn main_tags(
        &self,
        search_index: &SearchIndexState,
        questions: &[Question],
    ) -> LoopResult<HashMap<String, String>> {
        db::question_main_tags(self, search_index, questions)
    }
}

impl SubmissionRepository for rusqlite::Transaction<'_> {
    fn attempt(&self, attempt_id: &str) -> LoopResult<Option<TestAttempt>> {
        db::fetch_test_attempt(self, attempt_id)
    }

    fn questions(&self, bank_id: &str) -> LoopResult<Vec<Question>> {
        db::fetch_questions_by_bank_id(self, bank_id)
    }

    fn responses(&self, attempt_id: &str) -> LoopResult<Vec<ResponseState>> {
        db::fetch_responses_by_attempt_id(self, attempt_id)
    }

    fn main_tags(
        &self,
        search_index: &SearchIndexState,
        questions: &[Question],
    ) -> LoopResult<HashMap<String, String>> {
        db::question_main_tags(self, search_index, questions)
    }
}

fn load_submission_context(
    repository: &impl SubmissionRepository,
    search_index: &SearchIndexState,
    attempt_id: &str,
) -> LoopResult<SubmissionContext> {
    let Some(attempt) = repository.attempt(attempt_id)? else {
        return Err(LoopError::not_found("Test attempt not found"));
    };
    let questions = repository.questions(&attempt.bank_id)?;
    let responses = repository.responses(attempt_id)?;
    let main_tags = repository.main_tags(search_index, &questions)?;
    let analysis = scoring::analyze_submission(&questions, &responses, &main_tags);
    Ok(SubmissionContext {
        attempt,
        questions,
        main_tags,
        analysis,
    })
}

fn prepend_missing_tags(question: &mut Question, tags: &[String]) {
    for tag in tags.iter().rev() {
        if !question.tags.iter().any(|existing| existing == tag) {
            question.tags.insert(0, tag.clone());
        }
    }
}

fn require_completed(attempt: &TestAttempt) -> LoopResult<()> {
    if attempt.status == TestStatus::Completed {
        Ok(())
    } else {
        Err(LoopError::invalid_state(
            "Results are only available after the test attempt is completed",
        ))
    }
}

fn validate_search_args(args: &SearchQuestionsArgs) -> LoopResult<()> {
    if args.query.chars().count() > MAX_SEARCH_QUERY_CHARS {
        return Err(LoopError::invalid_input(format!(
            "Search query must be at most {MAX_SEARCH_QUERY_CHARS} characters"
        )));
    }
    if let Some(sections) = &args.sections {
        if sections.len() > MAX_SEARCH_SECTIONS {
            return Err(LoopError::invalid_input(format!(
                "Search can include at most {MAX_SEARCH_SECTIONS} sections"
            )));
        }
        if sections
            .iter()
            .any(|section| section.chars().count() > MAX_SEARCH_SECTION_CHARS)
        {
            return Err(LoopError::invalid_input(format!(
                "Each search section must be at most {MAX_SEARCH_SECTION_CHARS} characters"
            )));
        }
    }
    Ok(())
}

fn validate_identifier(label: &str, value: &str) -> LoopResult<()> {
    if value.trim().is_empty() {
        return Err(LoopError::invalid_input(format!(
            "{label} must not be empty"
        )));
    }
    if value.chars().count() > MAX_IDENTIFIER_CHARS {
        return Err(LoopError::invalid_input(format!(
            "{label} must be at most {MAX_IDENTIFIER_CHARS} characters"
        )));
    }
    Ok(())
}

fn ordinary_import_ownership_errors(bank: &super::types::QuestionBank) -> Vec<ValidationError> {
    let mut errors: Vec<ValidationError> = db::reserved_bundled_metadata_fields(bank)
        .into_iter()
        .map(|field| {
            ValidationError::new(
                format!("metadata.{field}"),
                "Bundled ownership metadata is reserved for the built-in catalog",
            )
        })
        .collect();
    if bank
        .metadata
        .extra
        .get("sourceId")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|source_id| source_id.starts_with("upsc_"))
    {
        errors.push(ValidationError::new(
            "metadata.sourceId",
            "The 'upsc_' source ID prefix is reserved for bundled papers",
        ));
    }
    errors
}

fn protect_session_answer_key(question: &mut Question) {
    question.redact_answer_key();
}

// ── Commands ────────────────────────────────────────────────────────────

/// Load the user's persisted settings.
#[tauri::command]
pub fn load_settings(db: State<'_, DbState>) -> LoopResult<Settings> {
    let c = conn(&db)?;
    db::load_settings(&c)
}

/// Persist a partial settings patch.
#[tauri::command]
pub fn save_settings(db: State<'_, DbState>, args: SaveSettingsArgs) -> LoopResult<()> {
    let mut c = conn(&db)?;
    db::save_settings(&mut c, args.settings)
}

/// Import a question bank from JSON into the database.
///
/// Validates the JSON first; returns validation errors on failure without
/// touching the DB.
#[tauri::command]
pub async fn import_question_bank(
    db: State<'_, DbState>,
    search_index: State<'_, SearchIndexState>,
    args: ImportQuestionBankArgs,
) -> LoopResult<ImportResult> {
    if args.json_content.len() > MAX_IMPORT_JSON_BYTES {
        return Ok(ImportResult {
            success: false,
            bank_id: None,
            error: Some("Validation failed".to_string()),
            validation_errors: Some(vec![ValidationError::new(
                "jsonContent",
                format!(
                    "Question bank JSON must not exceed {} MiB",
                    MAX_IMPORT_JSON_BYTES / (1024 * 1024)
                ),
            )]),
        });
    }
    let bank = match validation::parse_question_bank_json(&args.json_content) {
        Ok(bank) => bank,
        Err(errors) => {
            return Ok(ImportResult {
                success: false,
                bank_id: None,
                error: Some("Validation failed".to_string()),
                validation_errors: Some(errors),
            })
        }
    };
    let ownership_errors = ordinary_import_ownership_errors(&bank);
    if !ownership_errors.is_empty() {
        return Ok(ImportResult {
            success: false,
            bank_id: None,
            error: Some("Validation failed".to_string()),
            validation_errors: Some(ownership_errors),
        });
    }

    let db = db.inner().clone();
    let search_index = search_index.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut c = conn(&db)?;
        let conflicts = db::question_id_conflicts(&c, &bank)?;
        if !conflicts.is_empty() {
            return Ok(ImportResult {
                success: false,
                bank_id: None,
                error: Some("Validation failed".to_string()),
                validation_errors: Some(conflicts),
            });
        }
        let bank_id = match db::import_question_bank(&mut c, &bank) {
            Ok(bank_id) => bank_id,
            Err(error) => {
                log::error!("Question bank import failed: {error}");
                return Ok(ImportResult {
                    success: false,
                    bank_id: None,
                    error: Some("Import failed".to_string()),
                    validation_errors: None,
                });
            }
        };
        // All search paths acquire locks in DB -> cache order, so invalidating
        // before releasing the DB guard closes the stale-read window safely.
        db::invalidate_search_index(&search_index)?;
        schedule_search_rebuild(db.clone(), search_index.clone());
        Ok(ImportResult {
            success: true,
            bank_id: Some(bank_id),
            error: None,
            validation_errors: None,
        })
    })
    .await
    .map_err(LoopError::internal)?
}

/// Refresh only the taxonomy projection of a bundled bank. This deliberately
/// avoids deletion/re-import so historical attempts retain their bank and
/// question foreign keys.
#[tauri::command]
pub async fn refresh_question_bank_taxonomy(
    db: State<'_, DbState>,
    search_index: State<'_, SearchIndexState>,
    args: RefreshQuestionBankTaxonomyArgs,
) -> LoopResult<ImportResult> {
    validate_identifier("Bank ID", &args.bank_id)?;
    if args.json_content.len() > MAX_IMPORT_JSON_BYTES {
        return Ok(ImportResult {
            success: false,
            bank_id: None,
            error: Some("Validation failed".to_string()),
            validation_errors: Some(vec![ValidationError::new(
                "jsonContent",
                format!(
                    "Question bank JSON must not exceed {} MiB",
                    MAX_IMPORT_JSON_BYTES / (1024 * 1024)
                ),
            )]),
        });
    }
    let bank = match validation::parse_question_bank_json(&args.json_content) {
        Ok(bank) => bank,
        Err(errors) => {
            return Ok(ImportResult {
                success: false,
                bank_id: None,
                error: Some("Validation failed".to_string()),
                validation_errors: Some(errors),
            })
        }
    };

    let bank_id = args.bank_id;
    let db = db.inner().clone();
    let search_index = search_index.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut c = conn(&db)?;
        db::refresh_question_bank_taxonomy(&mut c, &bank_id, &bank)?;
        db::invalidate_search_index(&search_index)?;
        Ok(ImportResult {
            success: true,
            bank_id: Some(bank_id),
            error: None,
            validation_errors: None,
        })
    })
    .await
    .map_err(LoopError::internal)?
}

/// Import a bundled UPSC paper as an immutable revision and atomically make
/// it current. Existing revisions remain available to historical attempts.
#[tauri::command]
pub async fn sync_bundled_question_bank(
    db: State<'_, DbState>,
    search_index: State<'_, SearchIndexState>,
    args: SyncBundledQuestionBankArgs,
) -> LoopResult<BundledQuestionBankSyncResult> {
    if args.json_content.len() > MAX_IMPORT_JSON_BYTES {
        return Ok(BundledQuestionBankSyncResult {
            success: false,
            imported: false,
            bank_id: None,
            error: Some("Validation failed".to_string()),
            validation_errors: Some(vec![ValidationError::new(
                "jsonContent",
                format!(
                    "Question bank JSON must not exceed {} MiB",
                    MAX_IMPORT_JSON_BYTES / (1024 * 1024)
                ),
            )]),
        });
    }
    validate_identifier("Catalog key", &args.catalog_key)?;
    if args.catalog_version <= 0 || args.content_version <= 0 {
        return Err(LoopError::invalid_input(
            "Bundled catalog and content versions must be positive",
        ));
    }
    let content_hash = args.content_hash.to_ascii_lowercase();
    if content_hash.len() != 64 || !content_hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(LoopError::invalid_input(
            "Bundled content hash must be a SHA-256 hex digest",
        ));
    }
    let actual_hash = format!("{:x}", Sha256::digest(args.json_content.as_bytes()));
    if content_hash != actual_hash {
        return Err(LoopError::invalid_input(
            "Bundled content hash does not match the paper JSON",
        ));
    }
    let bank = match validation::parse_question_bank_json(&args.json_content) {
        Ok(bank) => bank,
        Err(errors) => {
            return Ok(BundledQuestionBankSyncResult {
                success: false,
                imported: false,
                bank_id: None,
                error: Some("Validation failed".to_string()),
                validation_errors: Some(errors),
            })
        }
    };
    if bank.metadata.exam != "UPSC CSE" {
        return Err(LoopError::invalid_input(
            "Bundled synchronization only accepts UPSC CSE papers",
        ));
    }

    let db = db.inner().clone();
    let search_index = search_index.inner().clone();
    let catalog_key = args.catalog_key;
    let catalog_version = args.catalog_version;
    let content_version = args.content_version;
    tauri::async_runtime::spawn_blocking(move || {
        let mut connection = conn(&db)?;
        let outcome = db::sync_bundled_question_bank(
            &mut connection,
            &catalog_key,
            &content_hash,
            catalog_version,
            content_version,
            bank,
        )?;
        if outcome.search_changed {
            db::invalidate_search_index(&search_index)?;
            schedule_search_rebuild(db.clone(), search_index.clone());
        }
        Ok(BundledQuestionBankSyncResult {
            success: true,
            imported: outcome.imported,
            bank_id: Some(outcome.bank_id),
            error: None,
            validation_errors: None,
        })
    })
    .await
    .map_err(LoopError::internal)?
}

/// Retire papers no longer present in the bundled catalog while preserving
/// every bank revision referenced by test history.
#[tauri::command]
pub async fn archive_missing_bundled_question_banks(
    db: State<'_, DbState>,
    search_index: State<'_, SearchIndexState>,
    args: ArchiveMissingBundledQuestionBanksArgs,
) -> LoopResult<usize> {
    if args.active_catalog_keys.is_empty() {
        return Err(LoopError::invalid_input(
            "Bundled catalog must contain at least one paper",
        ));
    }
    if args.active_catalog_keys.len() > MAX_BUNDLED_CATALOG_KEYS {
        return Err(LoopError::invalid_input(format!(
            "Bundled catalog may contain at most {MAX_BUNDLED_CATALOG_KEYS} papers"
        )));
    }
    let mut active_catalog_keys = HashSet::with_capacity(args.active_catalog_keys.len());
    for key in args.active_catalog_keys {
        validate_identifier("Catalog key", &key)?;
        active_catalog_keys.insert(key);
    }

    let db = db.inner().clone();
    let search_index = search_index.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut connection = conn(&db)?;
        let archived =
            db::archive_missing_bundled_question_banks(&mut connection, &active_catalog_keys)?;
        if archived > 0 {
            db::invalidate_search_index(&search_index)?;
            schedule_search_rebuild(db.clone(), search_index.clone());
        }
        Ok(archived)
    })
    .await
    .map_err(LoopError::internal)?
}

/// List all stored question banks.
#[tauri::command]
pub fn get_question_banks(db: State<'_, DbState>) -> LoopResult<Vec<StoredQuestionBank>> {
    let c = conn(&db)?;
    db::fetch_question_banks(&c)
}

/// Fetch a single question bank by ID.
#[tauri::command]
pub fn get_question_bank(
    db: State<'_, DbState>,
    args: BankIdArgs,
) -> LoopResult<Option<StoredQuestionBank>> {
    validate_identifier("Bank ID", &args.bank_id)?;
    let c = conn(&db)?;
    db::fetch_question_bank(&c, &args.bank_id)
}

/// Fetch a question bank together with its questions.
#[tauri::command]
pub fn get_question_bank_with_questions(
    db: State<'_, DbState>,
    search_index: State<'_, SearchIndexState>,
    args: BankIdArgs,
) -> LoopResult<Option<QuestionBankWithQuestions>> {
    validate_identifier("Bank ID", &args.bank_id)?;
    let c = conn(&db)?;
    let Some(mut bank) = db::fetch_question_bank_with_questions(&c, &args.bank_id)? else {
        return Ok(None);
    };
    let taxonomy_tags = db::question_taxonomy_tags(&c, &search_index, &bank.questions)?;
    for question in &mut bank.questions {
        if let Some(tags) = taxonomy_tags.get(&question.id) {
            prepend_missing_tags(question, tags);
        }
        question.redact_answer_key();
    }
    Ok(Some(bank))
}

/// Search every stored question and return the strongest semantic-aware hits.
#[tauri::command]
pub async fn search_questions(
    db: State<'_, DbState>,
    search_index: State<'_, SearchIndexState>,
    args: SearchQuestionsArgs,
) -> LoopResult<super::types::QuestionSearchResponse> {
    validate_search_args(&args)?;
    let db = db.inner().clone();
    let search_index = search_index.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let c = conn(&db)?;
        db::search_questions_cached(&c, &search_index, &args.query, args.sections.as_deref())
    })
    .await
    .map_err(LoopError::internal)?
}

/// Initialize the validated search index and perform one real model inference.
#[tauri::command]
pub async fn warm_question_search(
    db: State<'_, DbState>,
    search_index: State<'_, SearchIndexState>,
) -> LoopResult<()> {
    let db = db.inner().clone();
    let search_index = search_index.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let service = {
            let c = conn(&db)?;
            db::prepare_question_search(&c, &search_index)?
        };
        service.warm_embedding().map_err(|error| {
            log::warn!("Search warm-up failed: {error}");
            LoopError::unavailable("Semantic search is unavailable")
        })
    })
    .await
    .map_err(LoopError::internal)?
}

/// Delete a question bank (cascading).
#[tauri::command]
pub async fn delete_question_bank(
    db: State<'_, DbState>,
    search_index: State<'_, SearchIndexState>,
    args: BankIdArgs,
) -> LoopResult<()> {
    validate_identifier("Bank ID", &args.bank_id)?;
    let db = db.inner().clone();
    let search_index = search_index.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut c = conn(&db)?;
        db::delete_question_bank(&mut c, &args.bank_id)?;
        db::invalidate_search_index(&search_index)?;
        schedule_search_rebuild(db.clone(), search_index.clone());
        Ok(())
    })
    .await
    .map_err(LoopError::internal)?
}

/// Create a new test attempt.
#[tauri::command]
pub fn create_test_attempt(
    db: State<'_, DbState>,
    args: CreateTestAttemptArgs,
) -> LoopResult<String> {
    validate_identifier("Bank ID", &args.bank_id)?;
    let mut c = conn(&db)?;
    db::create_test_attempt(&mut c, &args.bank_id, args.mode, args.duration_override)
}

#[tauri::command]
pub fn list_test_attempt_history(
    db: State<'_, DbState>,
) -> LoopResult<Vec<TestAttemptHistoryEntry>> {
    let c = conn(&db)?;
    db::list_test_attempt_history(&c)
}

/// Save (or clear) an answer for a question.
#[tauri::command]
pub fn save_answer(db: State<'_, DbState>, args: SaveAnswerArgs) -> LoopResult<()> {
    validate_identifier("Attempt ID", &args.attempt_id)?;
    validate_identifier("Question ID", &args.question_id)?;
    if args
        .answer
        .as_ref()
        .is_some_and(|answer| answer.to_string().len() > MAX_ANSWER_JSON_BYTES)
    {
        return Err(LoopError::invalid_input(format!(
            "Answer must not exceed {} MiB",
            MAX_ANSWER_JSON_BYTES / (1024 * 1024)
        )));
    }
    let mut c = conn(&db)?;
    db::save_answer(
        &mut c,
        &args.attempt_id,
        &args.question_id,
        args.answer.as_ref(),
    )
}

/// Reveal feedback for an answered question in an active practice session.
#[tauri::command]
pub fn get_practice_question_feedback(
    db: State<'_, DbState>,
    args: PracticeQuestionFeedbackArgs,
) -> LoopResult<PracticeQuestionFeedback> {
    validate_identifier("Attempt ID", &args.attempt_id)?;
    validate_identifier("Question ID", &args.question_id)?;
    let c = conn(&db)?;
    db::fetch_practice_question_feedback(&c, &args.attempt_id, &args.question_id)?.ok_or_else(
        || {
            LoopError::invalid_state(
                "Practice feedback is available only after the answer has been saved",
            )
        },
    )
}

/// Toggle the flag on a question.
#[tauri::command]
pub fn toggle_flag(db: State<'_, DbState>, args: ToggleFlagArgs) -> LoopResult<bool> {
    validate_identifier("Attempt ID", &args.attempt_id)?;
    validate_identifier("Question ID", &args.question_id)?;
    let c = conn(&db)?;
    db::toggle_flag(&c, &args.attempt_id, &args.question_id)
}

/// Persist the current timer value.
#[tauri::command]
pub fn update_time_remaining(db: State<'_, DbState>, args: UpdateTimeArgs) -> LoopResult<()> {
    validate_identifier("Attempt ID", &args.attempt_id)?;
    let c = conn(&db)?;
    db::update_time_remaining(&c, &args.attempt_id, args.time_remaining)
}

/// Pause a test attempt.
#[tauri::command]
pub fn pause_test(db: State<'_, DbState>, args: UpdateTimeArgs) -> LoopResult<()> {
    validate_identifier("Attempt ID", &args.attempt_id)?;
    let c = conn(&db)?;
    db::pause_test(&c, &args.attempt_id, args.time_remaining)
}

/// Resume a paused test attempt.
#[tauri::command]
pub fn resume_test(db: State<'_, DbState>, args: AttemptIdArgs) -> LoopResult<()> {
    validate_identifier("Attempt ID", &args.attempt_id)?;
    let c = conn(&db)?;
    db::resume_test(&c, &args.attempt_id)
}

/// Submit a test and compute the score.
///
/// DATA FLOW:
/// 1. Fetch the attempt, its questions, and the user's responses.
/// 2. Run `scoring::analyze_submission` to evaluate each answer.
/// 3. Persist the aggregate score and completion state.
/// 4. Return the score to the frontend for immediate display.
#[tauri::command]
pub fn submit_test(
    db: State<'_, DbState>,
    search_index: State<'_, SearchIndexState>,
    args: SubmitTestArgs,
) -> LoopResult<SubmitResult> {
    validate_identifier("Attempt ID", &args.attempt_id)?;
    if args.time_remaining.is_some_and(|seconds| seconds < 0) {
        return Err(LoopError::invalid_input(
            "Time remaining cannot be negative",
        ));
    }
    let mut c = conn(&db)?;
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .stringify_err()?;
    let context = load_submission_context(&tx, &search_index, &args.attempt_id)?;
    let completed_at = db::now_ms();

    db::finalize_submission(
        &tx,
        &args.attempt_id,
        context.analysis.score,
        context.analysis.max_score,
        completed_at,
        args.time_remaining,
    )?;
    tx.commit().stringify_err()?;

    Ok(SubmitResult {
        score: context.analysis.score,
        max_score: context.analysis.max_score,
    })
}

/// Fetch a single test attempt.
#[tauri::command]
pub fn get_test_attempt(
    db: State<'_, DbState>,
    args: AttemptIdArgs,
) -> LoopResult<Option<TestAttempt>> {
    validate_identifier("Attempt ID", &args.attempt_id)?;
    let c = conn(&db)?;
    db::fetch_test_attempt(&c, &args.attempt_id)
}

/// Calculate the test result for a completed attempt.
#[tauri::command]
pub fn calculate_test_result(
    db: State<'_, DbState>,
    search_index: State<'_, SearchIndexState>,
    args: AttemptIdArgs,
) -> LoopResult<TestResult> {
    validate_identifier("Attempt ID", &args.attempt_id)?;
    let mut c = conn(&db)?;
    let tx = c.transaction().stringify_err()?;
    let context = load_submission_context(&tx, &search_index, &args.attempt_id)?;
    require_completed(&context.attempt)?;
    let result = scoring::build_test_result(&context.attempt, &context.analysis);
    tx.commit().stringify_err()?;
    Ok(result)
}

/// Fetch per-question review data for a completed attempt.
#[tauri::command]
pub fn get_question_review(
    db: State<'_, DbState>,
    search_index: State<'_, SearchIndexState>,
    args: AttemptIdArgs,
) -> LoopResult<Vec<super::types::QuestionReviewItem>> {
    validate_identifier("Attempt ID", &args.attempt_id)?;
    let mut c = conn(&db)?;
    let tx = c.transaction().stringify_err()?;
    let context = load_submission_context(&tx, &search_index, &args.attempt_id)?;
    require_completed(&context.attempt)?;
    let review =
        scoring::build_review_items(&context.questions, &context.analysis, &context.main_tags);
    tx.commit().stringify_err()?;
    Ok(review)
}

/// Load the full session payload for resuming a test attempt.
#[tauri::command]
pub fn get_session_payload(
    db: State<'_, DbState>,
    search_index: State<'_, SearchIndexState>,
    args: AttemptIdArgs,
) -> LoopResult<Option<LoadedSessionPayload>> {
    validate_identifier("Attempt ID", &args.attempt_id)?;
    let c = conn(&db)?;
    let Some(attempt) = db::fetch_test_attempt(&c, &args.attempt_id)? else {
        return Ok(None);
    };
    if attempt.status == TestStatus::Completed {
        return Ok(Some(session::build_loaded_session_payload(
            attempt,
            Vec::new(),
            Vec::new(),
        )));
    }
    let Some(bank) = db::fetch_question_bank_with_questions(&c, &attempt.bank_id)? else {
        return Err(LoopError::not_found("Question bank not found"));
    };
    let responses = db::fetch_responses_by_attempt_id(&c, &args.attempt_id)?;

    let mut questions = bank.questions;
    let main_tags = db::question_main_tags(&c, &search_index, &questions)?;
    for question in &mut questions {
        if let Some(main_tag) = main_tags.get(&question.id) {
            prepend_missing_tags(question, std::slice::from_ref(main_tag));
        }
        protect_session_answer_key(question);
    }

    Ok(Some(session::build_loaded_session_payload(
        attempt, questions, responses,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::types::{
        Difficulty, QuestionBank, QuestionBankMetadata, QuestionType, TestMode,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    struct FakeSubmissionRepository {
        attempt: Option<TestAttempt>,
        questions: Vec<Question>,
        responses: Vec<ResponseState>,
        tags: HashMap<String, String>,
    }

    impl SubmissionRepository for FakeSubmissionRepository {
        fn attempt(&self, _attempt_id: &str) -> LoopResult<Option<TestAttempt>> {
            Ok(self.attempt.clone())
        }

        fn questions(&self, _bank_id: &str) -> LoopResult<Vec<Question>> {
            Ok(self.questions.clone())
        }

        fn responses(&self, _attempt_id: &str) -> LoopResult<Vec<ResponseState>> {
            Ok(self.responses.clone())
        }

        fn main_tags(
            &self,
            _search_index: &SearchIndexState,
            _questions: &[Question],
        ) -> LoopResult<HashMap<String, String>> {
            Ok(self.tags.clone())
        }
    }

    fn attempt(status: TestStatus) -> TestAttempt {
        TestAttempt {
            id: "attempt".to_string(),
            bank_id: "bank".to_string(),
            mode: TestMode::Test,
            status,
            duration: 60,
            time_remaining: 30,
            started_at: 1_000,
            completed_at: None,
            score: None,
            max_score: None,
        }
    }

    fn question() -> Question {
        Question {
            id: "q".to_string(),
            question_type: QuestionType::SingleChoice,
            question: "Question?".to_string(),
            options: None,
            correct_answers: vec!["a".to_string()],
            explanation: String::new(),
            is_open_ended: false,
            marks: 2.0,
            mark_breakdown: Vec::new(),
            negative_marks: 0.5,
            negative_marks_unanswered: 0.0,
            time_estimate: None,
            difficulty: Some(Difficulty::Medium),
            tags: Vec::new(),
            taxonomy: None,
        }
    }

    #[test]
    fn fake_submission_repository_exercises_scoring_orchestration() {
        let repository = FakeSubmissionRepository {
            attempt: Some(attempt(TestStatus::InProgress)),
            questions: vec![question()],
            responses: vec![ResponseState {
                question_id: "q".to_string(),
                answer: Some(serde_json::json!("a")),
                is_flagged: true,
            }],
            tags: HashMap::from([("q".to_string(), "Polity".to_string())]),
        };
        let state = SearchIndexState::new(None, PathBuf::new(), None);

        let context = load_submission_context(&repository, &state, "attempt").unwrap();
        assert_eq!((context.analysis.correct, context.analysis.wrong), (1, 0));
        assert_eq!(context.analysis.flagged, 1);
        assert_eq!(context.analysis.score, 2.0);
        assert_eq!(
            context.main_tags.get("q").map(String::as_str),
            Some("Polity")
        );
    }

    #[test]
    fn completed_gate_rejects_active_and_paused_attempts() {
        assert!(require_completed(&attempt(TestStatus::InProgress)).is_err());
        assert!(require_completed(&attempt(TestStatus::Paused)).is_err());
        assert!(require_completed(&attempt(TestStatus::Completed)).is_ok());
    }

    #[test]
    fn search_argument_limits_reject_oversized_inputs() {
        let long_query = SearchQuestionsArgs {
            query: "x".repeat(MAX_SEARCH_QUERY_CHARS + 1),
            sections: None,
        };
        assert!(validate_search_args(&long_query).is_err());

        let too_many_sections = SearchQuestionsArgs {
            query: "polity".to_string(),
            sections: Some(vec!["section".to_string(); MAX_SEARCH_SECTIONS + 1]),
        };
        assert!(validate_search_args(&too_many_sections).is_err());

        assert!(validate_search_args(&SearchQuestionsArgs {
            query: "polity".to_string(),
            sections: Some(vec!["prelims-gs1".to_string()]),
        })
        .is_ok());
    }

    #[test]
    fn identifier_limits_reject_empty_and_oversized_values() {
        assert!(validate_identifier("Attempt ID", " ").is_err());
        assert!(validate_identifier("Attempt ID", &"x".repeat(MAX_IDENTIFIER_CHARS + 1)).is_err());
        assert!(validate_identifier("Attempt ID", "valid-id").is_ok());
    }

    #[test]
    fn every_active_session_payload_is_redacted() {
        let mut timed_question = question();
        protect_session_answer_key(&mut timed_question);
        assert!(timed_question.correct_answers.is_empty());

        let mut practice_question = question();
        protect_session_answer_key(&mut practice_question);
        assert!(practice_question.correct_answers.is_empty());
        assert!(practice_question.explanation.is_empty());
    }

    #[test]
    fn ordinary_import_reports_reserved_bundled_ownership_metadata() {
        let bank = QuestionBank {
            metadata: QuestionBankMetadata {
                name: "Custom".to_string(),
                exam: "UPSC CSE".to_string(),
                total_questions: 1,
                difficulty: Difficulty::Medium,
                default_duration: 60,
                extra: BTreeMap::from([
                    ("bundledActive".to_string(), serde_json::json!(false)),
                    ("BundledFutureOwner".to_string(), serde_json::json!(true)),
                    (
                        "bundledCatalogKey".to_string(),
                        serde_json::json!("prelims-gs1:2024:GS1"),
                    ),
                    ("sourceId".to_string(), serde_json::json!("upsc_custom_gs1")),
                ]),
            },
            questions: vec![question()],
        };

        let errors = ordinary_import_ownership_errors(&bank);

        assert_eq!(errors.len(), 4);
        assert!(errors
            .iter()
            .any(|error| error.path == "metadata.bundledActive"));
        assert!(errors
            .iter()
            .any(|error| error.path == "metadata.bundledCatalogKey"));
        assert!(errors
            .iter()
            .any(|error| error.path == "metadata.BundledFutureOwner"));
        assert!(errors.iter().any(|error| error.path == "metadata.sourceId"));
    }
}
