//! Session utilities: payload assembly for frontend hydration and
//! keyboard shortcut answer computation.
//!
//! These functions are stateless helpers called from `commands.rs`.
//! They don't touch the database — they transform already-fetched data
//! into the shapes the frontend expects.

use super::types::{AnswerEntry, LoadedSessionPayload, Question, ResponseState, TestAttempt};

/// Assemble the full payload needed to resume a test session.
///
/// The frontend needs the attempt metadata, the ordered question list,
/// the user's saved answers (as `[{ questionId, answer }]`), and the
/// list of flagged question IDs.  This function extracts those from
/// the raw response rows.
pub fn build_loaded_session_payload(
    attempt: TestAttempt,
    questions: Vec<Question>,
    responses: Vec<ResponseState>,
) -> LoadedSessionPayload {
    let answers = responses
        .iter()
        .filter_map(|response| {
            response.answer.clone().map(|answer| AnswerEntry {
                question_id: response.question_id.clone(),
                answer,
            })
        })
        .collect();

    let flags = responses
        .into_iter()
        .filter(|response| response.is_flagged)
        .map(|response| response.question_id)
        .collect();

    LoadedSessionPayload {
        attempt,
        questions,
        answers,
        flags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::types::{Difficulty, QuestionType, TestMode, TestStatus};

    fn attempt() -> TestAttempt {
        TestAttempt {
            id: "attempt".to_string(),
            bank_id: "bank".to_string(),
            mode: TestMode::Test,
            status: TestStatus::InProgress,
            duration: 60,
            time_remaining: 45,
            started_at: 1,
            completed_at: None,
            score: None,
            max_score: None,
        }
    }

    fn question() -> Question {
        Question {
            id: "q".to_string(),
            question_type: QuestionType::FillBlank,
            question: "Question?".to_string(),
            options: None,
            correct_answers: vec!["answer".to_string()],
            explanation: "Explanation".to_string(),
            is_open_ended: false,
            marks: 1.0,
            mark_breakdown: Vec::new(),
            negative_marks: 0.0,
            negative_marks_unanswered: 0.0,
            time_estimate: None,
            difficulty: Some(Difficulty::Medium),
            tags: Vec::new(),
            taxonomy: None,
        }
    }

    #[test]
    fn payload_separates_answers_and_flags_from_sparse_rows() {
        let payload = build_loaded_session_payload(
            attempt(),
            vec![question()],
            vec![
                ResponseState {
                    question_id: "q".to_string(),
                    answer: Some(serde_json::json!("saved")),
                    is_flagged: true,
                },
                ResponseState {
                    question_id: "flag-only".to_string(),
                    answer: None,
                    is_flagged: true,
                },
            ],
        );

        assert_eq!(payload.answers.len(), 1);
        assert_eq!(payload.answers[0].question_id, "q");
        assert_eq!(payload.flags, vec!["q", "flag-only"]);
    }

    #[test]
    fn redacted_questions_omit_grading_fields_from_json() {
        let mut item = question();
        item.redact_answer_key();
        let value = serde_json::to_value(item).unwrap();
        assert!(value.get("correctAnswers").is_none());
        assert!(value.get("explanation").is_none());
    }

    #[test]
    fn redaction_preserves_only_the_open_ended_presentation_hint() {
        let mut item = question();
        item.correct_answers = vec!["__open__".to_string()];
        item.redact_answer_key();
        let value = serde_json::to_value(item).unwrap();
        assert_eq!(value["isOpenEnded"], true);
        assert!(value.get("correctAnswers").is_none());
    }
}
