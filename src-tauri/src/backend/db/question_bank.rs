//! Question bank and question CRUD.
//!
//! A **question bank** is an imported JSON file containing exam metadata
//! and a list of questions.  Banks are immutable after import (questions
//! are not editable). Deleting a bank cascades to its questions and attempts.

use std::collections::HashSet;

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::{json, Value as JsonValue};

use super::helpers::{now_ms, parse_json, to_sql_error};
use super::{DbResult, QuestionRow};
use crate::backend::error::LoopError;
use crate::backend::error::ResultExt;
use crate::backend::types::{
    Difficulty, Question, QuestionBank, QuestionBankWithQuestions, QuestionMarkBreakdown,
    QuestionOption, QuestionType, StoredQuestionBank, ValidationError,
};

fn resolved_taxonomy(question: &Question) -> DbResult<(String, Vec<String>)> {
    if let Some(taxonomy) = &question.taxonomy {
        let resolved = taxonomy.resolve()?;
        Ok((
            resolved.main_tag.to_string(),
            resolved.subtags.into_iter().map(str::to_string).collect(),
        ))
    } else {
        // Preserve the existing convention for third-party banks that have
        // not adopted the typed taxonomy field yet.
        let main_tag = question.tags.first().cloned().unwrap_or_default();
        let subtags = question.tags.get(1..).unwrap_or_default().to_vec();
        Ok((main_tag, subtags))
    }
}

fn merge_mark_breakdown_taxonomy(
    stored: &mut [QuestionMarkBreakdown],
    incoming: &[QuestionMarkBreakdown],
) -> DbResult<()> {
    if stored.len() != incoming.len() {
        return Err(LoopError::invalid_input(
            "Taxonomy refresh mark-breakdown structure does not match the stored question",
        ));
    }
    for (stored_part, incoming_part) in stored.iter_mut().zip(incoming) {
        if stored_part.label != incoming_part.label
            || (stored_part.marks - incoming_part.marks).abs() > f64::EPSILON
        {
            return Err(LoopError::invalid_input(
                "Taxonomy refresh mark-breakdown content does not match the stored question",
            ));
        }
        stored_part.main_tag = incoming_part.main_tag;
        stored_part.subtags.clone_from(&incoming_part.subtags);
        merge_mark_breakdown_taxonomy(&mut stored_part.subparts, &incoming_part.subparts)?;
    }
    Ok(())
}

/// Fetch all stored question banks, newest first.
pub fn fetch_question_banks(conn: &Connection) -> DbResult<Vec<StoredQuestionBank>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, exam, metadata, total_questions, difficulty, default_duration, imported_at
             FROM question_banks
             ORDER BY imported_at DESC",
        )
        .stringify_err()?;
    let mut rows = stmt.query([]).stringify_err()?;
    let mut banks = Vec::new();

    while let Some(row) = rows.next().stringify_err()? {
        let difficulty: String = row.get("difficulty").stringify_err()?;

        let metadata: String = row.get("metadata").stringify_err()?;
        if bundled_metadata_is_archived(&metadata) {
            continue;
        }

        banks.push(StoredQuestionBank {
            id: row.get("id").stringify_err()?,
            name: row.get("name").stringify_err()?,
            exam: row.get("exam").stringify_err()?,
            metadata,
            total_questions: row.get("total_questions").stringify_err()?,
            difficulty: Difficulty::try_from(difficulty.as_str())?,
            default_duration: row.get("default_duration").stringify_err()?,
            imported_at: row.get("imported_at").stringify_err()?,
        });
    }

    Ok(banks)
}

/// Fetch a single question bank by ID.
pub fn fetch_question_bank(
    conn: &Connection,
    bank_id: &str,
) -> DbResult<Option<StoredQuestionBank>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, exam, metadata, total_questions, difficulty, default_duration, imported_at
             FROM question_banks
             WHERE id = ?1",
        )
        .stringify_err()?;

    let bank = stmt
        .query_row(params![bank_id], |row| {
            let difficulty: String = row.get("difficulty")?;

            Ok(StoredQuestionBank {
                id: row.get("id")?,
                name: row.get("name")?,
                exam: row.get("exam")?,
                metadata: row.get("metadata")?,
                total_questions: row.get("total_questions")?,
                difficulty: Difficulty::try_from(difficulty.as_str()).map_err(to_sql_error)?,
                default_duration: row.get("default_duration")?,
                imported_at: row.get("imported_at")?,
            })
        })
        .optional()
        .stringify_err()?;

    Ok(bank)
}

/// Fetch all questions belonging to a question bank, in insertion order.
pub fn fetch_questions_by_bank_id(conn: &Connection, bank_id: &str) -> DbResult<Vec<Question>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, type, question, options, correct_answers, explanation, marks, negative_marks,
                    negative_marks_unanswered, time_estimate, difficulty, tags, mark_breakdown,
                    qt.main_tag, qt.subtags_json
             FROM questions q
             LEFT JOIN question_taxonomy qt ON qt.question_id = q.id
             WHERE q.bank_id = ?1
             ORDER BY q.rowid",
        )
        .stringify_err()?;
    let mut rows = stmt.query(params![bank_id]).stringify_err()?;
    let mut questions = Vec::new();

    while let Some(row) = rows.next().stringify_err()? {
        questions.push(question_from_row(QuestionRow {
            id: row.get("id").stringify_err()?,
            question_type: row.get("type").stringify_err()?,
            question: row.get("question").stringify_err()?,
            options: row.get("options").stringify_err()?,
            correct_answers: row.get("correct_answers").stringify_err()?,
            explanation: row.get("explanation").stringify_err()?,
            marks: row.get("marks").stringify_err()?,
            negative_marks: row.get("negative_marks").stringify_err()?,
            negative_marks_unanswered: row.get("negative_marks_unanswered").stringify_err()?,
            time_estimate: row.get("time_estimate").stringify_err()?,
            difficulty: row.get("difficulty").stringify_err()?,
            tags: row.get("tags").stringify_err()?,
            mark_breakdown: row.get("mark_breakdown").stringify_err()?,
            taxonomy_main_tag: row.get("main_tag").stringify_err()?,
            taxonomy_subtags: row.get("subtags_json").stringify_err()?,
        })?);
    }

    Ok(questions)
}

/// Fetch a question bank together with its questions.
pub fn fetch_question_bank_with_questions(
    conn: &Connection,
    bank_id: &str,
) -> DbResult<Option<QuestionBankWithQuestions>> {
    let Some(bank) = fetch_question_bank(conn, bank_id)? else {
        return Ok(None);
    };

    Ok(Some(QuestionBankWithQuestions {
        id: bank.id,
        name: bank.name,
        exam: bank.exam,
        metadata: bank.metadata,
        total_questions: bank.total_questions,
        difficulty: bank.difficulty,
        default_duration: bank.default_duration,
        imported_at: bank.imported_at,
        questions: fetch_questions_by_bank_id(conn, bank_id)?,
    }))
}

/// Import a validated question bank into the database (transactional).
pub fn import_question_bank(conn: &mut Connection, bank: &QuestionBank) -> DbResult<String> {
    if let Some(field) = reserved_bundled_metadata_fields(bank).into_iter().next() {
        return Err(LoopError::invalid_input(format!(
            "Metadata field '{field}' is reserved for bundled catalog synchronization"
        )));
    }
    let bank_id = uuid::Uuid::new_v4().to_string();
    let imported_at = now_ms();
    let tx = conn.transaction().stringify_err()?;

    insert_question_bank_rows(&tx, bank, &bank_id, imported_at)?;
    tx.commit().stringify_err()?;
    Ok(bank_id)
}

/// Ownership fields are private to the bundled synchronization path. Ordinary
/// imports must not be able to hide themselves or opt into lifecycle cleanup.
pub fn reserved_bundled_metadata_fields(bank: &QuestionBank) -> Vec<String> {
    bank.metadata
        .extra
        .keys()
        // Reserve the namespace, not only today's fields. Otherwise a future
        // ownership field could already have been forged by an ordinary
        // import before the application learns its exact name.
        .filter(|key| key.to_ascii_lowercase().starts_with("bundled"))
        .cloned()
        .collect()
}

fn insert_question_bank_rows(
    tx: &Transaction<'_>,
    bank: &QuestionBank,
    bank_id: &str,
    imported_at: i64,
) -> DbResult<()> {
    let metadata = serde_json::to_string(&bank.metadata).stringify_err()?;

    tx.execute(
        "INSERT INTO question_banks (
            id, name, exam, metadata, total_questions, difficulty, default_duration, imported_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            bank_id,
            bank.metadata.name,
            bank.metadata.exam,
            metadata,
            bank.metadata.total_questions,
            bank.metadata.difficulty.as_str(),
            bank.metadata.default_duration,
            imported_at,
        ],
    )
    .stringify_err()?;

    {
        let mut insert_question = tx
            .prepare(
                "INSERT INTO questions (
                    id, bank_id, type, question, options, correct_answers, explanation, marks,
                    negative_marks, negative_marks_unanswered, time_estimate, difficulty, tags,
                    mark_breakdown
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            )
            .stringify_err()?;

        let mut insert_search_doc = tx
            .prepare(
                "INSERT OR REPLACE INTO search_documents (
                    question_id, question, options_text, main_tag, subtags_text,
                    bank_id, bank_name, year, stage, paper, section, content_fingerprint
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )
            .stringify_err()?;

        let mut insert_taxonomy = tx
            .prepare(
                "INSERT OR REPLACE INTO question_taxonomy (
                    question_id, main_tag, subtags_json, taxonomy_source, taxonomy_version
                 ) VALUES (?1, ?2, ?3, 'imported', ?4)",
            )
            .stringify_err()?;

        let mut insert_job = tx
            .prepare(
                "INSERT OR REPLACE INTO search_index_jobs (
                    question_id, operation, status, created_at, updated_at
                 ) VALUES (?1, 'embed', 'pending', ?2, ?2)",
            )
            .stringify_err()?;

        let year = bank.metadata.extra.get("year").and_then(|v| v.as_i64());
        let stage = bank
            .metadata
            .extra
            .get("stage")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let paper = bank
            .metadata
            .extra
            .get("paper")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let section = bank
            .metadata
            .extra
            .get("section")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        for question in &bank.questions {
            let options = question
                .options
                .as_ref()
                .filter(|options| !options.is_empty())
                .map(serde_json::to_string)
                .transpose()
                .stringify_err()?;
            let correct_answers =
                serde_json::to_string(&question.correct_answers).stringify_err()?;
            let tags = (!question.tags.is_empty())
                .then(|| serde_json::to_string(&question.tags))
                .transpose()
                .stringify_err()?;
            let mark_breakdown = serde_json::to_string(&question.mark_breakdown).stringify_err()?;

            insert_question
                .execute(params![
                    question.id,
                    bank_id,
                    question.question_type.as_str(),
                    question.question,
                    options,
                    correct_answers,
                    question.explanation,
                    question.marks,
                    question.negative_marks,
                    question.negative_marks_unanswered,
                    question.time_estimate,
                    question.difficulty.map(|difficulty| difficulty.as_str()),
                    tags,
                    mark_breakdown,
                ])
                .stringify_err()?;

            let mut opt_parts = Vec::new();
            if let Some(opts) = &question.options {
                for opt in opts {
                    opt_parts.push(format!("({}) {}", opt.id, opt.text.trim()));
                }
            }
            let options_text = opt_parts.join(" ");

            let (main_tag, subtags) = resolved_taxonomy(question)?;
            let subtags_text = subtags.join(" ");
            let subtags_json = serde_json::to_string(&subtags).unwrap_or_else(|_| "[]".to_string());

            let canonical = if options_text.is_empty() {
                question.question.clone()
            } else {
                format!("{}\n{}", question.question, options_text)
            };
            let fp = crate::search::indexing::fingerprint::content_fingerprint(&canonical);
            let fp_bytes = fp.to_le_bytes();

            insert_search_doc
                .execute(params![
                    question.id,
                    question.question,
                    options_text,
                    main_tag,
                    subtags_text,
                    bank_id,
                    bank.metadata.name,
                    year,
                    stage,
                    paper,
                    section,
                    &fp_bytes[..],
                ])
                .stringify_err()?;

            if !main_tag.is_empty() {
                insert_taxonomy
                    .execute(params![
                        question.id,
                        main_tag,
                        subtags_json,
                        crate::taxonomy::TAXONOMY_VERSION
                    ])
                    .stringify_err()?;
            }

            insert_job
                .execute(params![question.id, imported_at])
                .stringify_err()?;
        }
    }

    Ok(())
}

const BUNDLED_ACTIVE_FIELD: &str = "bundledActive";
const BUNDLED_ARCHIVED_AT_FIELD: &str = "bundledArchivedAt";
const BUNDLED_CATALOG_KEY_FIELD: &str = "bundledCatalogKey";
const BUNDLED_CATALOG_VERSION_FIELD: &str = "bundledCatalogVersion";
const BUNDLED_CONTENT_HASH_FIELD: &str = "bundledContentHash";
const BUNDLED_QUESTION_NAMESPACE_FIELD: &str = "bundledQuestionIdNamespace";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundledQuestionBankSyncOutcome {
    pub bank_id: String,
    pub imported: bool,
    pub search_changed: bool,
}

#[derive(Debug)]
struct StoredBundledRevision {
    id: String,
    metadata: JsonValue,
    catalog_key: Option<String>,
    content_hash: Option<String>,
    active: bool,
    managed: bool,
}

fn bundled_metadata_is_archived(metadata_json: &str) -> bool {
    serde_json::from_str::<JsonValue>(metadata_json)
        .ok()
        .and_then(|metadata| {
            metadata
                .get(BUNDLED_ACTIVE_FIELD)
                .and_then(JsonValue::as_bool)
        })
        == Some(false)
}

fn catalog_key_from_metadata(metadata: &JsonValue) -> Option<String> {
    if let Some(key) = metadata
        .get(BUNDLED_CATALOG_KEY_FIELD)
        .and_then(JsonValue::as_str)
        .filter(|key| !key.trim().is_empty())
    {
        return Some(key.to_string());
    }

    source_catalog_key_from_metadata(metadata)
}

fn source_catalog_key_from_metadata(metadata: &JsonValue) -> Option<String> {
    let section = metadata.get("section")?.as_str()?;
    let year = metadata.get("year")?.as_i64()?;
    let paper = metadata.get("paper")?.as_str()?;
    Some(format!("{section}:{year}:{paper}"))
}

fn bundled_source_id_matches(metadata: &JsonValue) -> bool {
    let Some(source_id) = metadata.get("sourceId").and_then(JsonValue::as_str) else {
        return false;
    };
    let Some(year) = metadata.get("year").and_then(JsonValue::as_i64) else {
        return false;
    };
    let Some(stage) = metadata.get("stage").and_then(JsonValue::as_str) else {
        return false;
    };
    let Some(section) = metadata.get("section").and_then(JsonValue::as_str) else {
        return false;
    };
    let (stage_segment, section_name) = if stage.eq_ignore_ascii_case("mains") {
        ("mains_", section.strip_prefix("mains-").unwrap_or(section))
    } else if stage.eq_ignore_ascii_case("prelims") {
        ("", section.strip_prefix("prelims-").unwrap_or(section))
    } else {
        return false;
    };
    source_id == format!("upsc_{year}_{stage_segment}{section_name}")
}

fn load_bundled_revisions(conn: &Connection) -> DbResult<Vec<StoredBundledRevision>> {
    let mut statement = conn
        .prepare("SELECT id, metadata FROM question_banks WHERE exam = 'UPSC CSE'")
        .stringify_err()?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .stringify_err()?;

    let mut revisions = Vec::new();
    for row in rows {
        let (id, raw_metadata) = row.stringify_err()?;
        let Ok(metadata) = serde_json::from_str::<JsonValue>(&raw_metadata) else {
            continue;
        };
        let catalog_key = catalog_key_from_metadata(&metadata);
        let content_hash = metadata
            .get(BUNDLED_CONTENT_HASH_FIELD)
            .and_then(JsonValue::as_str)
            .map(str::to_string);
        let active = metadata
            .get(BUNDLED_ACTIVE_FIELD)
            .and_then(JsonValue::as_bool)
            .unwrap_or(true);
        // Existing bundled installs predate the explicit marker. Their
        // section/year/paper/contentVersion tuple is the migration signal.
        let managed = metadata.get(BUNDLED_CATALOG_KEY_FIELD).is_some()
            || metadata
                .get("sourceId")
                .and_then(JsonValue::as_str)
                .is_some_and(|source_id| source_id.starts_with("upsc_"));
        revisions.push(StoredBundledRevision {
            id,
            metadata,
            catalog_key,
            content_hash,
            active,
            managed,
        });
    }
    Ok(revisions)
}

fn metadata_object(metadata: &mut JsonValue) -> DbResult<&mut serde_json::Map<String, JsonValue>> {
    metadata
        .as_object_mut()
        .ok_or_else(|| LoopError::internal("Question bank metadata is not an object"))
}

fn archive_bundled_revision(
    tx: &Transaction<'_>,
    revision: &StoredBundledRevision,
    archived_at: i64,
) -> DbResult<bool> {
    if !revision.active {
        return Ok(false);
    }

    let mut metadata = revision.metadata.clone();
    let object = metadata_object(&mut metadata)?;
    object.insert(BUNDLED_ACTIVE_FIELD.to_string(), json!(false));
    object.insert(BUNDLED_ARCHIVED_AT_FIELD.to_string(), json!(archived_at));
    if let Some(key) = &revision.catalog_key {
        object.insert(BUNDLED_CATALOG_KEY_FIELD.to_string(), json!(key));
    }
    let metadata_json = serde_json::to_string(&metadata).stringify_err()?;

    tx.execute(
        "UPDATE question_banks SET metadata = ?1 WHERE id = ?2",
        params![metadata_json, revision.id],
    )
    .stringify_err()?;
    tx.execute(
        "INSERT INTO search_index_jobs (
            question_id, operation, status, attempts, created_at, updated_at
         )
         SELECT id, 'delete', 'pending', 0, ?2, ?2
         FROM questions WHERE bank_id = ?1
         ON CONFLICT(question_id, operation) DO UPDATE SET
            status = 'pending', attempts = 0, last_error = NULL, updated_at = ?2",
        params![revision.id, archived_at],
    )
    .stringify_err()?;
    // Historical question/taxonomy rows remain available to attempts. Only
    // their active-catalog search projection is retired.
    tx.execute(
        "DELETE FROM search_documents WHERE bank_id = ?1",
        params![revision.id],
    )
    .stringify_err()?;
    Ok(true)
}

fn stamp_active_bundled_metadata(
    bank: &mut QuestionBank,
    catalog_key: &str,
    content_hash: &str,
    catalog_version: i64,
    content_version: i64,
) {
    bank.metadata
        .extra
        .insert(BUNDLED_CATALOG_KEY_FIELD.to_string(), json!(catalog_key));
    bank.metadata
        .extra
        .insert(BUNDLED_CONTENT_HASH_FIELD.to_string(), json!(content_hash));
    bank.metadata.extra.insert(
        BUNDLED_CATALOG_VERSION_FIELD.to_string(),
        json!(catalog_version),
    );
    bank.metadata
        .extra
        .insert("contentVersion".to_string(), json!(content_version));
    bank.metadata
        .extra
        .insert(BUNDLED_ACTIVE_FIELD.to_string(), json!(true));
    bank.metadata.extra.remove(BUNDLED_ARCHIVED_AT_FIELD);
}

fn namespace_question_ids_if_needed(
    tx: &Transaction<'_>,
    bank: &mut QuestionBank,
    content_hash: &str,
    bank_id: &str,
) -> DbResult<()> {
    let mut statement = tx
        .prepare("SELECT 1 FROM questions WHERE id = ?1 LIMIT 1")
        .stringify_err()?;
    let mut has_conflict = false;
    for question in &bank.questions {
        if statement
            .query_row(params![question.id], |_| Ok(()))
            .optional()
            .stringify_err()?
            .is_some()
        {
            has_conflict = true;
            break;
        }
    }
    drop(statement);

    if !has_conflict {
        return Ok(());
    }

    let hash_prefix = content_hash.get(..16).unwrap_or(content_hash);
    let bank_prefix = bank_id.get(..8).unwrap_or(bank_id);
    let namespace = format!("{hash_prefix}-{bank_prefix}");
    for question in &mut bank.questions {
        question.id = format!("{}::revision:{namespace}", question.id);
    }
    bank.metadata.extra.insert(
        BUNDLED_QUESTION_NAMESPACE_FIELD.to_string(),
        json!(namespace),
    );
    Ok(())
}

/// Import one bundled paper as a new immutable revision, then atomically make
/// it current. Prior revisions remain addressable by historical attempts.
pub fn sync_bundled_question_bank(
    conn: &mut Connection,
    catalog_key: &str,
    content_hash: &str,
    catalog_version: i64,
    content_version: i64,
    mut bank: QuestionBank,
) -> DbResult<BundledQuestionBankSyncOutcome> {
    let supplied_metadata = serde_json::to_value(&bank.metadata).stringify_err()?;
    if let Some(field) = reserved_bundled_metadata_fields(&bank).into_iter().next() {
        return Err(LoopError::invalid_input(format!(
            "Bundled paper JSON contains reserved metadata field '{field}'"
        )));
    }
    if source_catalog_key_from_metadata(&supplied_metadata).as_deref() != Some(catalog_key) {
        return Err(LoopError::invalid_input(
            "Catalog key does not match the bundled paper metadata",
        ));
    }
    if !bundled_source_id_matches(&supplied_metadata) {
        return Err(LoopError::invalid_input(
            "Bundled source ID does not match the paper metadata",
        ));
    }

    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .stringify_err()?;
    let revisions = load_bundled_revisions(&tx)?;
    let matching_active = revisions.iter().find(|revision| {
        revision.managed
            && revision.active
            && revision.catalog_key.as_deref() == Some(catalog_key)
            && revision.content_hash.as_deref() == Some(content_hash)
    });
    let activated_at = now_ms();

    if let Some(current) = matching_active {
        let mut metadata = current.metadata.clone();
        let object = metadata_object(&mut metadata)?;
        object.insert(BUNDLED_ACTIVE_FIELD.to_string(), json!(true));
        object.insert(BUNDLED_CATALOG_KEY_FIELD.to_string(), json!(catalog_key));
        object.insert(BUNDLED_CONTENT_HASH_FIELD.to_string(), json!(content_hash));
        object.insert(
            BUNDLED_CATALOG_VERSION_FIELD.to_string(),
            json!(catalog_version),
        );
        object.insert("contentVersion".to_string(), json!(content_version));
        object.remove(BUNDLED_ARCHIVED_AT_FIELD);
        tx.execute(
            "UPDATE question_banks SET metadata = ?1 WHERE id = ?2",
            params![
                serde_json::to_string(&metadata).stringify_err()?,
                current.id
            ],
        )
        .stringify_err()?;

        let mut search_changed = false;
        for duplicate in revisions.iter().filter(|revision| {
            revision.id != current.id
                && revision.managed
                && revision.active
                && revision.catalog_key.as_deref() == Some(catalog_key)
        }) {
            search_changed |= archive_bundled_revision(&tx, duplicate, activated_at)?;
        }
        let bank_id = current.id.clone();
        tx.commit().stringify_err()?;
        return Ok(BundledQuestionBankSyncOutcome {
            bank_id,
            imported: false,
            search_changed,
        });
    }

    let bank_id = uuid::Uuid::new_v4().to_string();
    stamp_active_bundled_metadata(
        &mut bank,
        catalog_key,
        content_hash,
        catalog_version,
        content_version,
    );
    namespace_question_ids_if_needed(&tx, &mut bank, content_hash, &bank_id)?;

    // Insertion and every possible validation/constraint failure happen before
    // the old revision is retired. The transaction publishes both changes or
    // neither of them.
    insert_question_bank_rows(&tx, &bank, &bank_id, activated_at)?;
    for current in revisions.iter().filter(|revision| {
        revision.managed && revision.active && revision.catalog_key.as_deref() == Some(catalog_key)
    }) {
        archive_bundled_revision(&tx, current, activated_at)?;
    }
    tx.commit().stringify_err()?;

    Ok(BundledQuestionBankSyncOutcome {
        bank_id,
        imported: true,
        search_changed: true,
    })
}

/// Retire papers removed from the bundled catalog without deleting the bank,
/// its questions, attempts, responses, or taxonomy needed by history views.
pub fn archive_missing_bundled_question_banks(
    conn: &mut Connection,
    active_catalog_keys: &HashSet<String>,
) -> DbResult<usize> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .stringify_err()?;
    let revisions = load_bundled_revisions(&tx)?;
    let archived_at = now_ms();
    let mut archived = 0;
    for revision in revisions.iter().filter(|revision| {
        revision.active
            && revision.managed
            && revision
                .catalog_key
                .as_ref()
                .is_some_and(|key| !active_catalog_keys.contains(key))
    }) {
        if archive_bundled_revision(&tx, revision, archived_at)? {
            archived += 1;
        }
    }
    tx.commit().stringify_err()?;
    Ok(archived)
}

/// Replace only the taxonomy projection of an existing bank. Question and
/// bank IDs remain stable, so attempts, responses, flags, and vector records
/// continue to refer to the same rows.
pub fn refresh_question_bank_taxonomy(
    conn: &mut Connection,
    bank_id: &str,
    bank: &QuestionBank,
) -> DbResult<()> {
    let Some(stored_bank) = fetch_question_bank(conn, bank_id)? else {
        return Err(LoopError::not_found("Question bank not found"));
    };
    let mut stored_metadata: serde_json::Value =
        serde_json::from_str(&stored_bank.metadata).stringify_err()?;
    let incoming_metadata = serde_json::to_value(&bank.metadata).stringify_err()?;
    for key in ["section", "year", "paper"] {
        if stored_metadata.get(key) != incoming_metadata.get(key) {
            return Err(LoopError::invalid_input(format!(
                "Taxonomy refresh metadata does not match the stored bank ({key})"
            )));
        }
    }

    let existing_question_ids = {
        let mut statement = conn
            .prepare("SELECT id FROM questions WHERE bank_id = ?1 ORDER BY rowid")
            .stringify_err()?;
        let rows = statement
            .query_map(params![bank_id], |row| row.get::<_, String>(0))
            .stringify_err()?;
        rows.collect::<Result<Vec<_>, _>>().stringify_err()?
    };
    let incoming_question_ids = bank
        .questions
        .iter()
        .map(|question| question.id.as_str())
        .collect::<Vec<_>>();
    if existing_question_ids
        .iter()
        .map(String::as_str)
        .ne(incoming_question_ids.iter().copied())
    {
        return Err(LoopError::invalid_input(
            "Taxonomy refresh question IDs do not match the stored bank",
        ));
    }

    let Some(metadata_object) = stored_metadata.as_object_mut() else {
        return Err(LoopError::internal(
            "Stored question-bank metadata is not an object",
        ));
    };
    metadata_object.insert(
        "taxonomyVersion".to_string(),
        serde_json::json!(crate::taxonomy::TAXONOMY_VERSION),
    );
    let metadata_json = serde_json::to_string(&stored_metadata).stringify_err()?;

    let tx = conn.transaction().stringify_err()?;
    tx.execute(
        "UPDATE question_banks SET metadata = ?2 WHERE id = ?1",
        params![bank_id, metadata_json],
    )
    .stringify_err()?;

    for question in &bank.questions {
        let (main_tag, subtags) = resolved_taxonomy(question)?;
        if main_tag.is_empty() {
            return Err(LoopError::invalid_input(
                "Taxonomy refresh requires a main tag for every question",
            ));
        }
        let subtags_json = serde_json::to_string(&subtags).stringify_err()?;
        let subtags_text = subtags.join(" ");
        let stored_mark_breakdown: Option<String> = tx
            .query_row(
                "SELECT mark_breakdown FROM questions WHERE id = ?1 AND bank_id = ?2",
                params![question.id, bank_id],
                |row| row.get(0),
            )
            .optional()
            .stringify_err()?;
        let mut merged_mark_breakdown: Vec<QuestionMarkBreakdown> = parse_json(
            stored_mark_breakdown.as_deref().unwrap_or("[]"),
            "question mark breakdown",
        )?;
        merge_mark_breakdown_taxonomy(&mut merged_mark_breakdown, &question.mark_breakdown)?;
        let mark_breakdown = serde_json::to_string(&merged_mark_breakdown).stringify_err()?;

        let updated_question = tx
            .execute(
                "UPDATE questions SET mark_breakdown = ?3 WHERE id = ?1 AND bank_id = ?2",
                params![question.id, bank_id, mark_breakdown],
            )
            .stringify_err()?;
        let updated_search = tx
            .execute(
                "UPDATE search_documents
                 SET main_tag = ?2, subtags_text = ?3
                 WHERE question_id = ?1 AND bank_id = ?4",
                params![question.id, main_tag, subtags_text, bank_id],
            )
            .stringify_err()?;
        if updated_question != 1 || updated_search != 1 {
            return Err(LoopError::internal(
                "Taxonomy refresh could not match an existing question",
            ));
        }
        tx.execute(
            "INSERT INTO question_taxonomy (
                question_id, main_tag, subtags_json, taxonomy_source, taxonomy_version
             ) VALUES (?1, ?2, ?3, 'imported', ?4)
             ON CONFLICT(question_id) DO UPDATE SET
                main_tag = excluded.main_tag,
                subtags_json = excluded.subtags_json,
                taxonomy_source = excluded.taxonomy_source,
                taxonomy_version = excluded.taxonomy_version",
            params![
                question.id,
                main_tag,
                subtags_json,
                crate::taxonomy::TAXONOMY_VERSION
            ],
        )
        .stringify_err()?;
    }

    tx.commit().stringify_err()
}

/// Return user-facing validation errors for IDs already owned by another bank.
pub fn question_id_conflicts(
    conn: &Connection,
    bank: &QuestionBank,
) -> DbResult<Vec<ValidationError>> {
    let mut statement = conn
        .prepare("SELECT bank_id FROM questions WHERE id = ?1")
        .stringify_err()?;
    let mut conflicts = Vec::new();
    for (index, question) in bank.questions.iter().enumerate() {
        let owner: Option<String> = statement
            .query_row(params![question.id], |row| row.get(0))
            .optional()
            .stringify_err()?;
        if owner.is_some() {
            conflicts.push(ValidationError::new(
                format!("questions[{index}].id"),
                format!(
                    "Question ID '{}' is already used by another bank",
                    question.id
                ),
            ));
        }
    }
    Ok(conflicts)
}

/// Delete a question bank and all associated data (cascading).
pub fn delete_question_bank(conn: &mut Connection, bank_id: &str) -> DbResult<()> {
    if fetch_question_bank(conn, bank_id)?.is_none() {
        return Err(LoopError::not_found("Question bank not found"));
    }

    let tx = conn.transaction().stringify_err()?;
    let queued_at = now_ms();
    tx.execute(
        "INSERT INTO search_index_jobs (
            question_id, operation, status, attempts, created_at, updated_at
         )
         SELECT id, 'delete', 'pending', 0, ?2, ?2
         FROM questions WHERE bank_id = ?1
         ON CONFLICT(question_id, operation) DO UPDATE SET
            status = 'pending', attempts = 0, last_error = NULL, updated_at = ?2",
        params![bank_id, queued_at],
    )
    .stringify_err()?;
    tx.execute("DELETE FROM question_banks WHERE id = ?1", params![bank_id])
        .stringify_err()?;

    tx.commit().stringify_err()
}

// ── Internal helpers ────────────────────────────────────────────────────

/// Question_from_row: maps raw SQLite columns → the domain `Question` type.
///
/// JSON columns (`options`, `correct_answers`, `tags`) are deserialized
/// here.  An empty options array is normalised to `None` to match the
/// type from the original import JSON (options are only meaningful for
/// choice-based question types).
pub(crate) fn question_from_row(row: QuestionRow) -> DbResult<Question> {
    let options: Vec<QuestionOption> =
        parse_json(row.options.as_deref().unwrap_or("[]"), "question options")?;
    let correct_answers = parse_json(&row.correct_answers, "correct answers")?;
    let is_open_ended = correct_answers == ["__open__"];
    let difficulty = match row.difficulty.as_deref().map(str::trim) {
        Some("") | None => None,
        Some(value) => Some(Difficulty::try_from(value)?),
    };
    let mark_breakdown: Vec<QuestionMarkBreakdown> = parse_json(
        row.mark_breakdown.as_deref().unwrap_or("[]"),
        "question mark breakdown",
    )?;
    let taxonomy = row
        .taxonomy_main_tag
        .as_deref()
        .filter(|label| !label.trim().is_empty())
        .and_then(|main_tag| {
            let subtags: Vec<String> = parse_json(
                row.taxonomy_subtags.as_deref().unwrap_or("[]"),
                "question taxonomy subtags",
            )
            .ok()?;
            crate::taxonomy::QuestionTaxonomy::from_labels(main_tag, &subtags).ok()
        });

    Ok(Question {
        id: row.id,
        question_type: QuestionType::try_from(row.question_type.as_str())?,
        question: row.question,
        options: if options.is_empty() {
            None
        } else {
            Some(options)
        },
        correct_answers,
        explanation: row.explanation,
        is_open_ended,
        marks: row.marks,
        mark_breakdown,
        negative_marks: row.negative_marks,
        negative_marks_unanswered: row.negative_marks_unanswered,
        time_estimate: row.time_estimate.filter(|value| *value > 0),
        difficulty,
        tags: parse_json(row.tags.as_deref().unwrap_or("[]"), "question tags")?,
        taxonomy,
    })
}

#[cfg(test)]
mod bundled_sync_tests {
    use std::collections::{BTreeMap, HashSet};
    use std::path::PathBuf;

    use rusqlite::Connection;

    use super::*;
    use crate::backend::db::schema::run_migrations;
    use crate::backend::types::{QuestionBankMetadata, QuestionOption, TestMode};

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    fn bundled_bank(question_text: &str) -> QuestionBank {
        let mut extra = BTreeMap::new();
        extra.insert("year".to_string(), json!(2024));
        extra.insert("stage".to_string(), json!("Prelims"));
        extra.insert("paper".to_string(), json!("GS1"));
        extra.insert("section".to_string(), json!("prelims-gs1"));
        extra.insert("sourceId".to_string(), json!("upsc_2024_gs1"));
        QuestionBank {
            metadata: QuestionBankMetadata {
                name: "Prelims GS Paper I · 2024".to_string(),
                exam: "UPSC CSE".to_string(),
                total_questions: 1,
                difficulty: Difficulty::Hard,
                default_duration: 7200,
                extra,
            },
            questions: vec![Question {
                id: "upsc-2024-gs1-q1".to_string(),
                question_type: QuestionType::SingleChoice,
                question: question_text.to_string(),
                options: Some(vec![
                    QuestionOption {
                        id: "a".to_string(),
                        text: "A".to_string(),
                    },
                    QuestionOption {
                        id: "b".to_string(),
                        text: "B".to_string(),
                    },
                ]),
                correct_answers: vec!["a".to_string()],
                explanation: "Because".to_string(),
                is_open_ended: false,
                marks: 2.0,
                mark_breakdown: Vec::new(),
                negative_marks: 0.667,
                negative_marks_unanswered: 0.0,
                time_estimate: Some(60),
                difficulty: Some(Difficulty::Hard),
                tags: vec!["Polity".to_string()],
                taxonomy: None,
            }],
        }
    }

    #[test]
    fn replacement_preserves_completed_attempt_and_retires_only_active_search_rows() {
        let mut conn = setup_conn();
        let key = "prelims-gs1:2024:GS1";
        let old = sync_bundled_question_bank(
            &mut conn,
            key,
            &"1".repeat(64),
            54,
            47,
            bundled_bank("Original question?"),
        )
        .unwrap();
        let attempt_id = crate::backend::db::attempt::create_test_attempt(
            &mut conn,
            &old.bank_id,
            TestMode::Test,
            None,
        )
        .unwrap();
        crate::backend::db::attempt::save_answer(
            &mut conn,
            &attempt_id,
            "upsc-2024-gs1-q1",
            Some(&json!("a")),
        )
        .unwrap();
        crate::backend::db::attempt::finalize_submission(
            &conn,
            &attempt_id,
            2.0,
            2.0,
            now_ms(),
            Some(7100),
        )
        .unwrap();

        let replacement = sync_bundled_question_bank(
            &mut conn,
            key,
            &"2".repeat(64),
            54,
            47,
            bundled_bank("Corrected question? ![map](/upsc/assets/map.png)"),
        )
        .unwrap();

        let active = fetch_question_banks(&conn).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, replacement.bank_id);
        assert_ne!(replacement.bank_id, old.bank_id);
        let historical = fetch_question_bank_with_questions(&conn, &old.bank_id)
            .unwrap()
            .unwrap();
        assert_eq!(historical.questions[0].question, "Original question?");
        let responses =
            crate::backend::db::attempt::fetch_responses_by_attempt_id(&conn, &attempt_id).unwrap();
        assert_eq!(responses.len(), 1);
        assert_eq!(
            crate::backend::db::attempt::list_test_attempt_history(&conn)
                .unwrap()
                .len(),
            1
        );
        let old_search_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM search_documents WHERE bank_id = ?1",
                params![old.bank_id],
                |row| row.get(0),
            )
            .unwrap();
        let old_taxonomy_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM question_taxonomy qt
                 JOIN questions q ON q.id = qt.question_id WHERE q.bank_id = ?1",
                params![historical.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(old_search_rows, 0);
        assert_eq!(old_taxonomy_rows, 1);
        let search_state = crate::backend::db::SearchIndexState::new(None, PathBuf::new(), None);
        let historical_tags =
            crate::backend::db::question_main_tags(&conn, &search_state, &historical.questions)
                .unwrap();
        let analysis = crate::backend::scoring::analyze_submission(
            &historical.questions,
            &responses,
            &historical_tags,
        );
        let review = crate::backend::scoring::build_review_items(
            &historical.questions,
            &analysis,
            &historical_tags,
        );
        assert_eq!(analysis.correct, 1);
        assert_eq!(review[0].question.explanation, "Because");
        assert_eq!(replacement.bank_id, active[0].id);
        let current_questions = fetch_questions_by_bank_id(&conn, &replacement.bank_id).unwrap();
        assert_ne!(current_questions[0].id, "upsc-2024-gs1-q1");
        assert!(current_questions[0]
            .question
            .contains("/upsc/assets/map.png"));
    }

    #[test]
    fn failed_replacement_leaves_the_previous_revision_active() {
        let mut conn = setup_conn();
        let key = "prelims-gs1:2024:GS1";
        let old = sync_bundled_question_bank(
            &mut conn,
            key,
            &"1".repeat(64),
            54,
            47,
            bundled_bank("Original question?"),
        )
        .unwrap();
        let mut invalid = bundled_bank("Invalid replacement?");
        invalid.questions.push(invalid.questions[0].clone());
        invalid.metadata.total_questions = 2;

        assert!(
            sync_bundled_question_bank(&mut conn, key, &"2".repeat(64), 54, 47, invalid,).is_err()
        );

        let active = fetch_question_banks(&conn).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, old.bank_id);
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM question_banks", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn mismatched_catalog_key_cannot_retire_another_paper() {
        let mut conn = setup_conn();
        let error = sync_bundled_question_bank(
            &mut conn,
            "prelims-gs1:2023:GS1",
            &"1".repeat(64),
            54,
            47,
            bundled_bank("Question?"),
        )
        .unwrap_err();

        assert_eq!(
            error.message(),
            "Catalog key does not match the bundled paper metadata"
        );
        assert!(fetch_question_banks(&conn).unwrap().is_empty());
    }

    #[test]
    fn forged_bundled_key_cannot_override_the_raw_paper_identity() {
        let mut conn = setup_conn();
        let mut forged = bundled_bank("Forged identity?");
        forged.metadata.extra.insert(
            "bundledCatalogKey".to_string(),
            json!("prelims-gs1:2023:GS1"),
        );

        let error = sync_bundled_question_bank(
            &mut conn,
            "prelims-gs1:2023:GS1",
            &"1".repeat(64),
            54,
            47,
            forged,
        )
        .unwrap_err();

        assert!(error.message().contains("reserved metadata field"));
        assert!(fetch_question_banks(&conn).unwrap().is_empty());
    }

    #[test]
    fn archive_missing_does_not_claim_similar_user_imported_banks() {
        let mut conn = setup_conn();
        let mut custom = bundled_bank("Custom question?");
        custom
            .metadata
            .extra
            .insert("sourceId".to_string(), json!("custom_2024_gs1"));
        custom
            .metadata
            .extra
            .insert("contentVersion".to_string(), json!(47));
        let custom_id = import_question_bank(&mut conn, &custom).unwrap();

        assert_eq!(
            archive_missing_bundled_question_banks(&mut conn, &HashSet::new()).unwrap(),
            0
        );
        let visible = fetch_question_banks(&conn).unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, custom_id);

        let bundled = sync_bundled_question_bank(
            &mut conn,
            "prelims-gs1:2024:GS1",
            &"2".repeat(64),
            54,
            47,
            bundled_bank("Bundled question?"),
        )
        .unwrap();
        assert_eq!(
            archive_missing_bundled_question_banks(&mut conn, &HashSet::new()).unwrap(),
            1
        );
        assert!(fetch_question_bank(&conn, &bundled.bank_id)
            .unwrap()
            .is_some());
        assert_eq!(fetch_question_banks(&conn).unwrap().len(), 1);
    }

    #[test]
    fn ordinary_import_rejects_forged_bundled_ownership_fields() {
        let mut conn = setup_conn();
        let mut forged = bundled_bank("Forged ownership?");
        forged
            .metadata
            .extra
            .insert("bundledActive".to_string(), json!(false));
        forged.metadata.extra.insert(
            "bundledCatalogKey".to_string(),
            json!("prelims-gs1:2024:GS1"),
        );

        let error = import_question_bank(&mut conn, &forged).unwrap_err();

        assert!(error
            .message()
            .contains("reserved for bundled catalog synchronization"));
        assert!(fetch_question_banks(&conn).unwrap().is_empty());
    }
}
