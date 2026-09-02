//! Test attempt, responses, flag/save/submit operations.
//!
//! Response rows are sparse: a row exists only when a question is answered or
//! flagged. The `(attempt_id, question_id)` primary key is the natural key.
//! - Functions that perform multi-statement writes take `&mut Connection`
//!   to signal they need a transaction.  Read-only functions take `&Connection`.

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::Value as JsonValue;

use super::helpers::{now_ms, parse_json};
use super::question_bank::fetch_question_bank;
use super::{AttemptRow, DbResult, ResponseRow};
use crate::backend::error::LoopError;
use crate::backend::error::ResultExt;
use crate::backend::types::{
    PracticeQuestionFeedback, ResponseState, TestAttempt, TestAttemptHistoryEntry, TestMode,
    TestStatus,
};

/// Create a new test attempt. Responses are inserted lazily.
pub fn create_test_attempt(
    conn: &mut Connection,
    bank_id: &str,
    mode: TestMode,
    duration_override: Option<i64>,
) -> DbResult<String> {
    let Some(bank) = fetch_question_bank(conn, bank_id)? else {
        return Err(LoopError::not_found("Question bank not found"));
    };
    if duration_override.is_some_and(|duration| duration <= 0) {
        return Err(LoopError::invalid_input(
            "Duration override must be positive",
        ));
    }

    let attempt_id = uuid::Uuid::new_v4().to_string();
    let duration = duration_override.unwrap_or(bank.default_duration);
    let time_remaining = if matches!(mode, TestMode::Test) {
        duration
    } else {
        0
    };
    let started_at = now_ms();

    conn.execute(
        "INSERT INTO test_attempts (
            id, bank_id, mode, status, duration, time_remaining, started_at, completed_at, score, max_score
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, NULL)",
        params![
            attempt_id,
            bank_id,
            mode.as_str(),
            TestStatus::InProgress.as_str(),
            duration,
            time_remaining,
            started_at,
        ],
        )
    .stringify_err()?;

    Ok(attempt_id)
}

/// Save (or clear) the user's answer for a question.
///
/// Rejects writes for attempts that are no longer in progress.
///
/// Fix (#14): The status check and upsert are wrapped in an IMMEDIATE
/// transaction so no other writer can change the attempt status between
/// the SELECT and the UPDATE.
pub fn save_answer(
    conn: &mut Connection,
    attempt_id: &str,
    question_id: &str,
    answer: Option<&JsonValue>,
) -> DbResult<()> {
    // CONCURRENCY (#14): status check + upsert in a single transaction.
    // Without this, a concurrent `submit_test` could mark the attempt
    // as 'completed' between our SELECT and our UPDATE, allowing a
    // stale answer to be saved after scoring.
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .stringify_err()?;

    // Check the attempt is still in progress inside the transaction.
    let attempt = {
        let mut stmt = tx
            .prepare(
                "SELECT id, bank_id, mode, status, duration, time_remaining, started_at, completed_at, score, max_score
                 FROM test_attempts WHERE id = ?1",
            )
            .stringify_err()?;
        let row = stmt
            .query_row(params![attempt_id], |row| {
                Ok(AttemptRow {
                    id: row.get("id")?,
                    bank_id: row.get("bank_id")?,
                    mode: row.get("mode")?,
                    status: row.get("status")?,
                    duration: row.get("duration")?,
                    time_remaining: row.get("time_remaining")?,
                    started_at: row.get("started_at")?,
                    completed_at: row.get("completed_at")?,
                    score: row.get("score")?,
                    max_score: row.get("max_score")?,
                })
            })
            .optional()
            .stringify_err()?;

        match row {
            Some(r) => test_attempt_from_row(r)?,
            None => return Err(LoopError::not_found("Test attempt not found")),
        }
    };

    if !matches!(attempt.status, TestStatus::InProgress) {
        return Err(LoopError::invalid_state(
            "Cannot save answer: test attempt is not in progress",
        ));
    }

    let answer_json = answer
        .filter(|value| !is_empty_answer(value))
        .map(serde_json::to_string)
        .transpose()
        .stringify_err()?;

    let changed = tx
        .execute(
            "INSERT INTO question_responses (attempt_id, question_id, answer, is_flagged)
         SELECT ?1, ?2, ?3, 0
         WHERE EXISTS (
            SELECT 1 FROM test_attempts a
            JOIN questions q ON q.bank_id = a.bank_id
            WHERE a.id = ?1 AND q.id = ?2
         )
         ON CONFLICT(attempt_id, question_id) DO UPDATE SET answer = excluded.answer",
            params![attempt_id, question_id, answer_json],
        )
        .stringify_err()?;

    if changed == 0 {
        return Err(LoopError::not_found("Question not found in test attempt"));
    }

    tx.execute(
        "DELETE FROM question_responses
         WHERE attempt_id = ?1 AND question_id = ?2
           AND answer IS NULL AND is_flagged = 0",
        params![attempt_id, question_id],
    )
    .stringify_err()?;

    tx.commit().stringify_err()?;
    Ok(())
}

fn is_empty_answer(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => true,
        JsonValue::String(text) => text.trim().is_empty(),
        JsonValue::Array(values) => {
            values.is_empty()
                || values.iter().all(|value| {
                    matches!(value, JsonValue::Null)
                        || matches!(value, JsonValue::String(text) if text.trim().is_empty())
                })
        }
        _ => false,
    }
}

/// Toggle the flag on a question response. Returns the new flag state.
///
/// Flags are review/bookmark metadata rather than grading input, so they may
/// be changed while an attempt is active or after it is completed. Paused
/// attempts remain immutable until explicitly resumed.
pub fn toggle_flag(conn: &Connection, attempt_id: &str, question_id: &str) -> DbResult<bool> {
    // ATOMICITY: The ON CONFLICT trick makes this a single SQL statement,
    // so no transaction is needed.  The SELECT that follows is guaranteed
    // to see the value we just wrote because SQLite is serialised.
    let changed = conn
        .execute(
            "INSERT INTO question_responses (attempt_id, question_id, answer, is_flagged)
         SELECT ?1, ?2, NULL, 1
         WHERE EXISTS (
            SELECT 1 FROM test_attempts a
            JOIN questions q ON q.bank_id = a.bank_id
            WHERE a.id = ?1 AND q.id = ?2 AND a.status IN (?3, ?4)
         )
         ON CONFLICT(attempt_id, question_id) DO UPDATE SET is_flagged = NOT is_flagged",
            params![
                attempt_id,
                question_id,
                TestStatus::InProgress.as_str(),
                TestStatus::Completed.as_str()
            ],
        )
        .stringify_err()?;

    if changed == 0 {
        return Err(LoopError::invalid_state(
            "Cannot toggle flag: attempt is paused or question was not found",
        ));
    }

    let new_flag: Option<i64> = conn
        .query_row(
            "SELECT is_flagged FROM question_responses
             WHERE attempt_id = ?1 AND question_id = ?2",
            params![attempt_id, question_id],
            |row| row.get(0),
        )
        .optional()
        .stringify_err()?;

    let Some(new_flag) = new_flag else {
        return Err(LoopError::invalid_state(
            "Cannot toggle flag: attempt is paused or question was not found",
        ));
    };

    if new_flag == 0 {
        conn.execute(
            "DELETE FROM question_responses
             WHERE attempt_id = ?1 AND question_id = ?2
               AND answer IS NULL AND is_flagged = 0",
            params![attempt_id, question_id],
        )
        .stringify_err()?;
    }

    Ok(new_flag == 1)
}

/// Persist the current `time_remaining` for a test attempt.
pub fn update_time_remaining(
    conn: &Connection,
    attempt_id: &str,
    time_remaining: i64,
) -> DbResult<()> {
    if time_remaining < 0 {
        return Err(LoopError::invalid_input(
            "Time remaining cannot be negative",
        ));
    }
    let changed = conn
        .execute(
            "UPDATE test_attempts
             SET time_remaining = MIN(time_remaining, duration, ?1)
             WHERE id = ?2 AND status IN (?3, ?4)",
            params![
                time_remaining,
                attempt_id,
                TestStatus::InProgress.as_str(),
                TestStatus::Paused.as_str()
            ],
        )
        .stringify_err()?;
    if changed == 0 {
        return Err(LoopError::invalid_state(
            "Cannot update timer: attempt is completed or was not found",
        ));
    }
    Ok(())
}

/// Mark a test attempt as paused and store the remaining time.
pub fn pause_test(conn: &Connection, attempt_id: &str, time_remaining: i64) -> DbResult<()> {
    if time_remaining < 0 {
        return Err(LoopError::invalid_input(
            "Time remaining cannot be negative",
        ));
    }
    let changed = conn
        .execute(
            "UPDATE test_attempts
             SET status = ?1, time_remaining = MIN(time_remaining, duration, ?2)
             WHERE id = ?3 AND status = ?4",
            params![
                TestStatus::Paused.as_str(),
                time_remaining,
                attempt_id,
                TestStatus::InProgress.as_str()
            ],
        )
        .stringify_err()?;
    if changed == 0 {
        return Err(LoopError::invalid_state(
            "Cannot pause: attempt is not in progress",
        ));
    }
    Ok(())
}

/// Resume a paused test attempt.
pub fn resume_test(conn: &Connection, attempt_id: &str) -> DbResult<()> {
    let changed = conn
        .execute(
            "UPDATE test_attempts SET status = ?1 WHERE id = ?2 AND status = ?3",
            params![
                TestStatus::InProgress.as_str(),
                attempt_id,
                TestStatus::Paused.as_str()
            ],
        )
        .stringify_err()?;
    if changed == 0 {
        return Err(LoopError::invalid_state(
            "Cannot resume: attempt is not paused",
        ));
    }
    Ok(())
}

/// Fetch a single test attempt by ID.
pub fn fetch_test_attempt(conn: &Connection, attempt_id: &str) -> DbResult<Option<TestAttempt>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, bank_id, mode, status, duration, time_remaining, started_at, completed_at, score, max_score
             FROM test_attempts
             WHERE id = ?1",
        )
        .stringify_err()?;

    let attempt = stmt
        .query_row(params![attempt_id], |row| {
            Ok(AttemptRow {
                id: row.get("id")?,
                bank_id: row.get("bank_id")?,
                mode: row.get("mode")?,
                status: row.get("status")?,
                duration: row.get("duration")?,
                time_remaining: row.get("time_remaining")?,
                started_at: row.get("started_at")?,
                completed_at: row.get("completed_at")?,
                score: row.get("score")?,
                max_score: row.get("max_score")?,
            })
        })
        .optional()
        .stringify_err()?;

    attempt.map(test_attempt_from_row).transpose()
}

pub fn list_test_attempt_history(conn: &Connection) -> DbResult<Vec<TestAttemptHistoryEntry>> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.completed_at, b.name AS paper,
                    COALESCE(a.score, 0.0) AS score,
                    COALESCE(a.max_score, 0.0) AS max_score
             FROM test_attempts a
             JOIN question_banks b ON b.id = a.bank_id
             WHERE a.mode = ?1 AND a.status = ?2 AND a.completed_at IS NOT NULL
             ORDER BY a.completed_at DESC, a.started_at DESC",
        )
        .stringify_err()?;

    let mut rows = stmt
        .query(params![
            TestMode::Test.as_str(),
            TestStatus::Completed.as_str()
        ])
        .stringify_err()?;
    let mut entries = Vec::new();

    while let Some(row) = rows.next().stringify_err()? {
        entries.push(TestAttemptHistoryEntry {
            id: row.get("id").stringify_err()?,
            completed_at: row.get("completed_at").stringify_err()?,
            paper: row.get("paper").stringify_err()?,
            score: row.get("score").stringify_err()?,
            max_score: row.get("max_score").stringify_err()?,
        });
    }

    Ok(entries)
}

/// Fetch all responses for a test attempt.
pub fn fetch_responses_by_attempt_id(
    conn: &Connection,
    attempt_id: &str,
) -> DbResult<Vec<ResponseState>> {
    let mut stmt = conn
        .prepare(
            "SELECT question_id, answer, is_flagged
             FROM question_responses
             WHERE attempt_id = ?1",
        )
        .stringify_err()?;
    let mut rows = stmt.query(params![attempt_id]).stringify_err()?;
    let mut responses = Vec::new();

    while let Some(row) = rows.next().stringify_err()? {
        responses.push(response_state_from_row(ResponseRow {
            question_id: row.get("question_id").stringify_err()?,
            answer: row.get("answer").stringify_err()?,
            is_flagged: row.get("is_flagged").stringify_err()?,
        })?);
    }

    Ok(responses)
}

/// Reveal feedback for one practice question only after its answer is saved.
/// Active session payloads never contain grading fields for other questions.
pub fn fetch_practice_question_feedback(
    conn: &Connection,
    attempt_id: &str,
    question_id: &str,
) -> DbResult<Option<PracticeQuestionFeedback>> {
    let row = conn
        .query_row(
            "SELECT q.correct_answers, q.explanation
             FROM test_attempts a
             JOIN questions q ON q.bank_id = a.bank_id
             JOIN question_responses r
               ON r.attempt_id = a.id AND r.question_id = q.id
             WHERE a.id = ?1 AND q.id = ?2
               AND a.mode = ?3 AND a.status = ?4
               AND r.answer IS NOT NULL",
            params![
                attempt_id,
                question_id,
                TestMode::Practice.as_str(),
                TestStatus::InProgress.as_str()
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .stringify_err()?;

    row.map(|(correct_answers, explanation)| {
        Ok(PracticeQuestionFeedback {
            question_id: question_id.to_string(),
            correct_answers: parse_json(&correct_answers, "correct answers")?,
            explanation,
        })
    })
    .transpose()
}

/// Finalise a submission. Per-question correctness is derived on demand.
pub fn finalize_submission(
    conn: &Connection,
    attempt_id: &str,
    score: f64,
    max_score: f64,
    completed_at: i64,
    time_remaining: Option<i64>,
) -> DbResult<()> {
    if time_remaining.is_some_and(|seconds| seconds < 0) {
        return Err(LoopError::invalid_input(
            "Time remaining cannot be negative",
        ));
    }
    let changed = conn
        .execute(
            "UPDATE test_attempts
         SET status = ?1,
             completed_at = ?2,
             score = ?3,
             max_score = ?4,
             time_remaining = CASE
                 WHEN ?5 IS NULL THEN time_remaining
                 ELSE MIN(time_remaining, duration, ?5)
             END
         WHERE id = ?6 AND status = ?7",
            params![
                TestStatus::Completed.as_str(),
                completed_at,
                score,
                max_score,
                time_remaining,
                attempt_id,
                TestStatus::InProgress.as_str()
            ],
        )
        .stringify_err()?;
    if changed == 0 {
        return Err(LoopError::invalid_state(
            "Cannot submit: attempt is not in progress",
        ));
    }
    Ok(())
}

// ── Internal helpers ────────────────────────────────────────────────────

fn test_attempt_from_row(row: AttemptRow) -> DbResult<TestAttempt> {
    Ok(TestAttempt {
        id: row.id,
        bank_id: row.bank_id,
        mode: TestMode::try_from(row.mode.as_str())?,
        status: TestStatus::try_from(row.status.as_str())?,
        duration: row.duration,
        time_remaining: row.time_remaining,
        started_at: row.started_at,
        completed_at: row.completed_at,
        score: row.score,
        max_score: row.max_score,
    })
}

fn response_state_from_row(row: ResponseRow) -> DbResult<ResponseState> {
    let answer = row
        .answer
        .as_deref()
        .map(|value| parse_json::<JsonValue>(value, "saved answer"))
        .transpose()?;

    Ok(ResponseState {
        question_id: row.question_id,
        answer,
        is_flagged: row.is_flagged == 1,
    })
}
