//! Answer evaluation and test result computation.
//!
//! The scoring engine is intentionally stateless: it takes questions +
//! responses and returns evaluation/analysis structs.  The caller
//! (commands.rs) is responsible for persisting the results.
//!
//! SCORING RULES:
//! - Correct answer:   +question.marks
//! - Wrong answer:     -question.negative_marks
//! - Unanswered:       -question.negative_marks_unanswered
//! - Category breakdown uses the same broad UPSC taxonomy as semantic search.

use std::collections::{BTreeMap, HashMap};

use serde_json::Value as JsonValue;

use super::types::{
    CategoryScore, Question, QuestionReviewItem, QuestionType, ResponseState, TestAttempt,
    TestMode, TestResult,
};

/// Per-question evaluation result produced by `analyze_submission`.
///
/// Kept in memory so result and review views can be derived without storing
/// duplicate correctness data for every response.
#[derive(Debug, Clone)]
pub struct QuestionEvaluation {
    pub question_id: String,
    pub user_answer: Option<JsonValue>,
    pub is_correct: Option<bool>, // None = unanswered
    pub is_flagged: bool,
    pub marks_obtained: f64, // positive for correct, negative for wrong/unanswered
}

/// Aggregated analysis of an entire test submission.
#[derive(Debug, Clone)]
pub struct SubmissionAnalysis {
    pub correct: usize,
    pub wrong: usize,
    pub unanswered: usize,
    pub flagged: usize,
    pub score: f64,     // sum of all marks_obtained (can be negative)
    pub max_score: f64, // sum of all question.marks
    pub category_breakdown: Option<Vec<CategoryScore>>,
    pub evaluations: Vec<QuestionEvaluation>,
}

/// Evaluate every question against the user's responses.
///
/// Builds a `SubmissionAnalysis` containing per-question evaluations,
/// aggregate counts, and an optional category breakdown.
pub fn analyze_submission(
    questions: &[Question],
    responses: &[ResponseState],
    main_tags: &HashMap<String, String>,
) -> SubmissionAnalysis {
    let response_map: HashMap<&str, &ResponseState> = responses
        .iter()
        .map(|response| (response.question_id.as_str(), response))
        .collect();

    let mut evaluations = Vec::with_capacity(questions.len());
    let mut correct = 0;
    let mut wrong = 0;
    let mut unanswered = 0;
    let mut flagged = 0;
    let mut score = 0.0;
    let mut max_score = 0.0;
    let mut category_stats: BTreeMap<String, (f64, f64)> = BTreeMap::new();

    for question in questions {
        max_score += question.marks;
        let response = response_map.get(question.id.as_str()).copied();
        let user_answer = response
            .and_then(|item| item.answer.clone())
            .filter(|answer| !is_empty_answer(answer));
        let is_flagged = response.map(|item| item.is_flagged).unwrap_or(false);

        if is_flagged {
            flagged += 1;
        }

        let (is_correct, marks_obtained) = evaluate_question(question, user_answer.as_ref());
        score += marks_obtained;

        match is_correct {
            Some(true) => correct += 1,
            Some(false) => wrong += 1,
            None => unanswered += 1,
        }

        let category = main_tags
            .get(&question.id)
            .map(|item| item.trim())
            .filter(|item| !item.is_empty())
            .unwrap_or("Other");
        let entry = category_stats.entry(category.to_string()).or_default();

        if marks_obtained >= 0.0 {
            entry.0 += marks_obtained;
        } else {
            entry.1 += marks_obtained.abs();
        }

        evaluations.push(QuestionEvaluation {
            question_id: question.id.clone(),
            user_answer,
            is_correct,
            is_flagged,
            marks_obtained,
        });
    }

    SubmissionAnalysis {
        correct,
        wrong,
        unanswered,
        flagged,
        score,
        max_score,
        category_breakdown: if category_stats.is_empty() {
            None
        } else {
            Some(
                category_stats
                    .into_iter()
                    .map(
                        |(category, (positive_marks, negative_marks))| CategoryScore {
                            category,
                            positive_marks,
                            negative_marks,
                        },
                    )
                    .collect(),
            )
        },
        evaluations,
    }
}

/// Build the test result summary shown on the results page.
///
/// `time_taken` is only meaningful for completed attempts. Callers gate result
/// access on completion; returning zero for an incomplete value keeps this
/// pure helper safe if it is reused elsewhere.
pub fn build_test_result(attempt: &TestAttempt, analysis: &SubmissionAnalysis) -> TestResult {
    let time_taken = match (attempt.mode, attempt.completed_at) {
        (TestMode::Test, Some(_)) => {
            attempt.duration - attempt.time_remaining.clamp(0, attempt.duration)
        }
        (TestMode::Practice, Some(completed_at)) => {
            ((completed_at - attempt.started_at) / 1000).max(0)
        }
        (_, None) => 0,
    };

    TestResult {
        attempt_id: attempt.id.clone(),
        total_questions: analysis.evaluations.len(),
        correct: analysis.correct,
        wrong: analysis.wrong,
        unanswered: analysis.unanswered,
        flagged: analysis.flagged,
        score: analysis.score,
        max_score: analysis.max_score,
        time_taken,
        category_breakdown: analysis.category_breakdown.clone(),
    }
}

/// Build per-question review items for the review page.
///
/// Each item pairs a question with its evaluation so the UI can display
/// the correct answer, the user's answer, and whether they got it right.
pub fn build_review_items(
    questions: &[Question],
    analysis: &SubmissionAnalysis,
    main_tags: &HashMap<String, String>,
) -> Vec<QuestionReviewItem> {
    let evaluation_map: HashMap<&str, &QuestionEvaluation> = analysis
        .evaluations
        .iter()
        .map(|evaluation| (evaluation.question_id.as_str(), evaluation))
        .collect();

    questions
        .iter()
        .map(|question| {
            let evaluation = evaluation_map.get(question.id.as_str()).copied();
            let mut question = question.clone();
            if let Some(main_tag) = main_tags.get(&question.id) {
                if !question.tags.iter().any(|tag| tag == main_tag) {
                    question.tags.insert(0, main_tag.clone());
                }
            }

            QuestionReviewItem {
                question,
                user_answer: evaluation.and_then(|item| item.user_answer.clone()),
                is_correct: evaluation.and_then(|item| item.is_correct).unwrap_or(false),
                is_flagged: evaluation.map(|item| item.is_flagged).unwrap_or(false),
                marks_obtained: evaluation.map(|item| item.marks_obtained).unwrap_or(0.0),
            }
        })
        .collect()
}

/// Evaluate a single question: returns (is_correct, marks_obtained).
///
/// - `None` for unanswered (applies negative_marks_unanswered penalty).
/// - `Some(true/false)` for answered (applies marks or negative_marks).
fn evaluate_question(question: &Question, user_answer: Option<&JsonValue>) -> (Option<bool>, f64) {
    match user_answer.filter(|answer| !is_empty_answer(answer)) {
        None => (None, -question.negative_marks_unanswered),
        Some(answer) => {
            let is_correct = is_answer_correct(question, answer);
            let marks_obtained = if is_correct {
                question.marks
            } else {
                -question.negative_marks
            };

            (Some(is_correct), marks_obtained)
        }
    }
}

/// Check whether a user answer matches the correct answer(s).
///
/// Dispatches to type-specific comparison logic based on `question_type`.
fn is_answer_correct(question: &Question, answer: &JsonValue) -> bool {
    match question.question_type {
        QuestionType::MultipleChoice => {
            let Some(selected) = as_string_array(answer) else {
                return false;
            };

            // Set-equality check: same length + mutual containment.
            selected.len() == question.correct_answers.len()
                && selected
                    .iter()
                    .all(|value| question.correct_answers.iter().any(|ca| ca == value))
                && question
                    .correct_answers
                    .iter()
                    .all(|value| selected.iter().any(|s| s == value))
        }
        // Tolerance note (#10): numerical answers are compared with a
        // 1e-9 absolute tolerance.  This is sufficient for exam-style
        // integer or low-precision decimal answers.  If sub-nano
        // precision is ever required (unlikely for an exam app), switch
        // to a relative-error comparison or fixed-point representation.
        QuestionType::Numerical => numeric_value(answer)
            .map(|left| {
                question.correct_answers.iter().any(|expected| {
                    expected
                        .trim()
                        .parse::<f64>()
                        .map(|right| (left - right).abs() < 1e-9)
                        .unwrap_or(false)
                })
            })
            .unwrap_or_else(|| {
                normalized_string(answer)
                    .map(|left| {
                        question
                            .correct_answers
                            .iter()
                            .any(|right| left.trim() == right.trim())
                    })
                    .unwrap_or(false)
            }),
        QuestionType::FillBlank => normalized_string(answer)
            .map(|left| {
                question
                    .correct_answers
                    .iter()
                    .any(|right| left.trim().eq_ignore_ascii_case(right.trim()))
            })
            .unwrap_or(false),
        _ => normalized_string(answer)
            .map(|value| question.correct_answers.iter().any(|ca| ca == &value))
            .unwrap_or(false),
    }
}

/// Treat renderer representations of a cleared answer as unanswered.
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

/// Extract a comparable string representation from a JSON value.
///
/// Returns owned `String` because `Number` and `Bool` variants require
/// allocation.  For `String` values the inner text is cloned (#12).
fn normalized_string(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(text) => Some(text.clone()),
        JsonValue::Number(number) => Some(number.to_string()),
        JsonValue::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

/// Try to interpret a JSON value as `f64`.
///
/// Accepts both JSON numbers and stringified numbers.
fn numeric_value(value: &JsonValue) -> Option<f64> {
    match value {
        JsonValue::Number(number) => number.as_f64(),
        JsonValue::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// Convert a JSON array of primitives into a `Vec<String>`.
///
/// Returns `None` if the value is not an array or if any element cannot
/// be normalised to a string.
fn as_string_array(value: &JsonValue) -> Option<Vec<String>> {
    match value {
        JsonValue::Array(values) => values
            .iter()
            .map(normalized_string)
            .collect::<Option<Vec<String>>>(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::types::{Difficulty, QuestionOption, TestMode, TestStatus};

    fn question(id: &str) -> Question {
        Question {
            id: id.to_string(),
            question_type: QuestionType::SingleChoice,
            question: "Question?".to_string(),
            options: Some(vec![QuestionOption {
                id: "a".to_string(),
                text: "A".to_string(),
            }]),
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

    fn response(id: &str, answer: Option<JsonValue>, is_flagged: bool) -> ResponseState {
        ResponseState {
            question_id: id.to_string(),
            answer,
            is_flagged,
        }
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected}, received {actual}"
        );
    }

    #[test]
    fn category_breakdown_separates_positive_and_negative_marks() {
        let questions = vec![question("correct"), question("wrong")];
        let responses = vec![
            ResponseState {
                question_id: "correct".to_string(),
                answer: Some(serde_json::json!("a")),
                is_flagged: false,
            },
            ResponseState {
                question_id: "wrong".to_string(),
                answer: Some(serde_json::json!("b")),
                is_flagged: false,
            },
        ];
        let tags = HashMap::from([
            ("correct".to_string(), "Polity".to_string()),
            ("wrong".to_string(), "Polity".to_string()),
        ]);

        let analysis = analyze_submission(&questions, &responses, &tags);
        let category = &analysis.category_breakdown.unwrap()[0];
        assert_eq!(category.positive_marks, 2.0);
        assert_eq!(category.negative_marks, 0.5);
    }

    #[test]
    fn aggregate_counts_flags_and_unanswered_penalties_are_exact() {
        let mut unanswered = question("unanswered");
        unanswered.negative_marks_unanswered = 0.25;
        let questions = vec![question("correct"), question("wrong"), unanswered];
        let responses = vec![
            response("correct", Some(serde_json::json!("a")), true),
            response("wrong", Some(serde_json::json!("b")), false),
        ];

        let analysis = analyze_submission(&questions, &responses, &HashMap::new());
        assert_eq!(
            (analysis.correct, analysis.wrong, analysis.unanswered),
            (1, 1, 1)
        );
        assert_eq!(analysis.flagged, 1);
        assert_close(analysis.score, 1.25);
        assert_close(analysis.max_score, 6.0);
        assert_close(analysis.evaluations[2].marks_obtained, -0.25);
        assert_eq!(analysis.evaluations[2].is_correct, None);
    }

    #[test]
    fn multiple_choice_requires_set_equality() {
        let mut item = question("multi");
        item.question_type = QuestionType::MultipleChoice;
        item.correct_answers = vec!["a".to_string(), "c".to_string()];

        assert_eq!(
            evaluate_question(&item, Some(&serde_json::json!(["c", "a"]))),
            (Some(true), 2.0)
        );
        assert_eq!(
            evaluate_question(&item, Some(&serde_json::json!(["a"]))),
            (Some(false), -0.5)
        );
        assert_eq!(
            evaluate_question(&item, Some(&serde_json::json!(["a", "a"]))),
            (Some(false), -0.5)
        );
    }

    #[test]
    fn numerical_fill_blank_and_true_false_answers_are_normalized() {
        let mut numerical = question("number");
        numerical.question_type = QuestionType::Numerical;
        numerical.correct_answers = vec!["10".to_string(), "12.5".to_string()];
        assert!(is_answer_correct(&numerical, &serde_json::json!(12.5)));
        assert!(is_answer_correct(&numerical, &serde_json::json!(" 12.5 ")));
        assert!(!is_answer_correct(&numerical, &serde_json::json!(12.5001)));

        let mut blank = question("blank");
        blank.question_type = QuestionType::FillBlank;
        blank.correct_answers = vec!["Lok Sabha".to_string(), "Parliament".to_string()];
        assert!(is_answer_correct(
            &blank,
            &serde_json::json!(" parliament ")
        ));

        let mut boolean = question("boolean");
        boolean.question_type = QuestionType::TrueFalse;
        boolean.correct_answers = vec!["true".to_string()];
        assert!(is_answer_correct(&boolean, &serde_json::json!(true)));
    }

    #[test]
    fn timed_result_uses_consumed_countdown_and_practice_uses_wall_time() {
        let analysis = analyze_submission(&[question("q")], &[], &HashMap::new());
        let mut attempt = TestAttempt {
            id: "attempt".to_string(),
            bank_id: "bank".to_string(),
            mode: TestMode::Test,
            status: TestStatus::Completed,
            duration: 600,
            time_remaining: 450,
            started_at: 1_000,
            completed_at: Some(126_000),
            score: None,
            max_score: None,
        };

        assert_eq!(build_test_result(&attempt, &analysis).time_taken, 150);

        attempt.mode = TestMode::Practice;
        assert_eq!(build_test_result(&attempt, &analysis).time_taken, 125);

        attempt.completed_at = None;
        assert_eq!(build_test_result(&attempt, &analysis).time_taken, 0);
    }

    #[test]
    fn cleared_answer_shapes_are_scored_as_unanswered() {
        let mut item = question("cleared");
        item.negative_marks_unanswered = 0.25;

        for answer in [
            serde_json::Value::Null,
            serde_json::json!("  "),
            serde_json::json!([]),
            serde_json::json!(["", " "]),
        ] {
            assert_eq!(evaluate_question(&item, Some(&answer)), (None, -0.25));
        }

        let analysis = analyze_submission(
            std::slice::from_ref(&item),
            &[response("cleared", Some(serde_json::json!([])), false)],
            &HashMap::new(),
        );
        assert_eq!(
            (analysis.correct, analysis.wrong, analysis.unanswered),
            (0, 0, 1)
        );
        assert_eq!(analysis.evaluations[0].user_answer, None);
    }

    #[test]
    fn review_items_preserve_answers_marks_flags_and_taxonomy_tag() {
        let questions = vec![question("q")];
        let responses = vec![response("q", Some(serde_json::json!("b")), true)];
        let tags = HashMap::from([("q".to_string(), "Polity".to_string())]);
        let analysis = analyze_submission(&questions, &responses, &tags);

        let review = build_review_items(&questions, &analysis, &tags);
        assert_eq!(review.len(), 1);
        assert_eq!(review[0].user_answer, Some(serde_json::json!("b")));
        assert!(!review[0].is_correct);
        assert!(review[0].is_flagged);
        assert_close(review[0].marks_obtained, -0.5);
        assert_eq!(review[0].question.tags, vec!["Polity"]);
    }
}
