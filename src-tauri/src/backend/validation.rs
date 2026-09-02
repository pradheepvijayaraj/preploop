//! Question bank JSON validation.
//!
//! Validation is split into two phases:
//! 1. **Structural** (`deserialize_question_bank`): JSON parsing via serde.
//!    Reports the first structural error (wrong types, missing fields).
//! 2. **Semantic** (`validate_question_bank`): Business rule checks on the
//!    parsed struct.  Collects all errors so the user sees everything at once.
//!
//! Both phases run before any data touches the database.

use super::types::{QuestionBank, QuestionMarkBreakdown, ValidationError};

fn mark_breakdown_has_taxonomy(parts: &[QuestionMarkBreakdown]) -> bool {
    parts.iter().any(|part| {
        part.main_tag.is_some()
            || !part.subtags.is_empty()
            || mark_breakdown_has_taxonomy(&part.subparts)
    })
}

fn validate_mark_breakdown_taxonomy(
    parts: &[QuestionMarkBreakdown],
    path: &str,
    max_subtags: usize,
    errors: &mut Vec<ValidationError>,
) {
    for (index, part) in parts.iter().enumerate() {
        let part_path = format!("{path}[{index}]");
        match part.main_tag {
            Some(main_tag) => {
                let subtags = part
                    .subtags
                    .iter()
                    .copied()
                    .map(crate::taxonomy::Subtag::try_from)
                    .collect::<Result<Vec<_>, _>>();
                match subtags {
                    Ok(subtags) => {
                        if subtags.len() > max_subtags {
                            errors.push(ValidationError::new(
                                format!("{part_path}.subtags"),
                                format!("Taxonomy has more than {max_subtags} subtags"),
                            ));
                        }
                        let taxonomy = crate::taxonomy::QuestionTaxonomy { main_tag, subtags };
                        if let Err(error) = taxonomy.resolve() {
                            errors
                                .push(ValidationError::new(format!("{part_path}.taxonomy"), error));
                        }
                    }
                    Err(error) => {
                        errors.push(ValidationError::new(format!("{part_path}.subtags"), error))
                    }
                }
            }
            None if !part.subtags.is_empty() => errors.push(ValidationError::new(
                format!("{part_path}.mainTag"),
                "A main tag is required when taxonomy subtags are present",
            )),
            None => {}
        }
        validate_mark_breakdown_taxonomy(
            &part.subparts,
            &format!("{part_path}.subparts"),
            max_subtags,
            errors,
        );
    }
}

const MARK_SUM_TOLERANCE: f64 = 1e-9;

/// Parse and validate a question bank JSON string.
///
/// Returns `Ok(QuestionBank)` if valid, or `Err(Vec<String>)` with
/// all validation errors.  Used by `import_question_bank` in commands.rs.
pub fn parse_question_bank_json(json_content: &str) -> Result<QuestionBank, Vec<ValidationError>> {
    let bank = deserialize_question_bank(json_content)?;
    let errors = validate_question_bank(&bank);

    if errors.is_empty() {
        Ok(bank)
    } else {
        Err(errors)
    }
}

/// Deserialise the raw JSON into a `QuestionBank`.
///
/// NOTE (#8): `serde_path_to_error` reports only the **first** structural
/// error (e.g. wrong type for a field).  This is a limitation of serde's
/// fail-fast parsing; collecting multiple structural errors would require
/// a custom deserialiser or a crate like `serde_valid`.  Semantic errors
/// (missing IDs, wrong option counts, etc.) are fully aggregated by
/// `validate_question_bank` below.
fn deserialize_question_bank(json_content: &str) -> Result<QuestionBank, Vec<ValidationError>> {
    let mut deserializer = serde_json::Deserializer::from_str(json_content);

    serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
        let path = error.path().to_string();
        let message = error.inner().to_string();

        vec![ValidationError::new(path, message)]
    })
}

/// Semantic validation of a parsed question bank struct.
///
/// Checks business rules that serde can't enforce:
/// - Required fields non-empty
/// - Positive marks/duration
/// - Unique question IDs
/// - Option consistency for choice-based questions
/// - Correct answer IDs match declared options
fn validate_question_bank(bank: &QuestionBank) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    let taxonomy_version = bank
        .metadata
        .extra
        .get("taxonomyVersion")
        .and_then(serde_json::Value::as_u64);
    let has_typed_taxonomy = bank.questions.iter().any(|question| {
        question.taxonomy.is_some() || mark_breakdown_has_taxonomy(&question.mark_breakdown)
    });
    if has_typed_taxonomy && taxonomy_version != Some(u64::from(crate::taxonomy::TAXONOMY_VERSION))
    {
        errors.push(ValidationError::new(
            "metadata.taxonomyVersion",
            format!(
                "Typed taxonomy requires taxonomy version {}",
                crate::taxonomy::TAXONOMY_VERSION
            ),
        ));
    } else if bank.metadata.extra.contains_key("taxonomyVersion")
        && taxonomy_version != Some(u64::from(crate::taxonomy::TAXONOMY_VERSION))
    {
        errors.push(ValidationError::new(
            "metadata.taxonomyVersion",
            format!(
                "Unsupported taxonomy version; expected {}",
                crate::taxonomy::TAXONOMY_VERSION
            ),
        ));
    }
    let is_versioned_taxonomy =
        taxonomy_version == Some(u64::from(crate::taxonomy::TAXONOMY_VERSION));
    let is_essay = bank
        .metadata
        .extra
        .get("section")
        .and_then(serde_json::Value::as_str)
        == Some("mains-essay");
    let max_subtags = if is_essay {
        2
    } else {
        crate::taxonomy::MAX_SUBTAGS
    };

    if bank.metadata.name.trim().is_empty() {
        errors.push(ValidationError::new(
            "metadata.name",
            "Question bank name is required",
        ));
    }
    if bank.metadata.exam.trim().is_empty() {
        errors.push(ValidationError::new(
            "metadata.exam",
            "Exam type is required",
        ));
    }
    if bank.metadata.total_questions <= 0 {
        errors.push(ValidationError::new(
            "metadata.totalQuestions",
            "Total questions must be positive",
        ));
    }
    if bank.metadata.default_duration <= 0 {
        errors.push(ValidationError::new(
            "metadata.defaultDuration",
            "Default duration must be positive (in seconds)",
        ));
    }
    if bank.questions.is_empty() {
        errors.push(ValidationError::new(
            "questions",
            "At least one question is required",
        ));
    } else if bank.metadata.total_questions > 0
        && bank.metadata.total_questions as usize != bank.questions.len()
    {
        errors.push(ValidationError::new(
            "metadata.totalQuestions",
            "Total questions in metadata does not match actual question count",
        ));
    }

    let mut seen_question_ids = std::collections::HashSet::new();

    for (index, question) in bank.questions.iter().enumerate() {
        let base = format!("questions[{index}]");

        if question.id.trim().is_empty() {
            errors.push(ValidationError::new(
                format!("{base}.id"),
                "Question ID is required",
            ));
        }
        if !seen_question_ids.insert(question.id.clone()) {
            errors.push(ValidationError::new(
                format!("{base}.id"),
                format!("Duplicate question ID '{}'", question.id),
            ));
        }
        if question.question.trim().is_empty() {
            errors.push(ValidationError::new(
                format!("{base}.question"),
                "Question text is required",
            ));
        }
        if question.correct_answers.is_empty() {
            errors.push(ValidationError::new(
                format!("{base}.correctAnswers"),
                "At least one correct answer is required",
            ));
        } else if question
            .correct_answers
            .iter()
            .any(|answer| answer.trim().is_empty())
        {
            errors.push(ValidationError::new(
                format!("{base}.correctAnswers"),
                "Correct answers cannot be empty",
            ));
        }
        if question.marks <= 0.0 {
            errors.push(ValidationError::new(
                format!("{base}.marks"),
                "Marks must be positive",
            ));
        }
        if !question.mark_breakdown.is_empty() {
            validate_mark_breakdown(
                &question.mark_breakdown,
                question.marks,
                &format!("{base}.markBreakdown"),
                &mut errors,
            );
        }
        if question.negative_marks < 0.0 {
            errors.push(ValidationError::new(
                format!("{base}.negativeMarks"),
                "Negative marks cannot be negative",
            ));
        }
        if question.negative_marks_unanswered < 0.0 {
            errors.push(ValidationError::new(
                format!("{base}.negativeMarksUnanswered"),
                "Negative marks for unanswered cannot be negative",
            ));
        }
        if let Some(time_estimate) = question.time_estimate {
            if time_estimate <= 0 {
                errors.push(ValidationError::new(
                    format!("{base}.timeEstimate"),
                    "Time estimate must be positive",
                ));
            }
        }
        if is_versioned_taxonomy && question.taxonomy.is_none() {
            errors.push(ValidationError::new(
                format!("{base}.taxonomy"),
                "A versioned taxonomy requires exactly one main tag per question",
            ));
        }
        if let Some(taxonomy) = &question.taxonomy {
            if taxonomy.subtags.len() > max_subtags {
                errors.push(ValidationError::new(
                    format!("{base}.taxonomy.subtags"),
                    format!("Taxonomy has more than {max_subtags} subtags"),
                ));
            }
            if let Err(error) = taxonomy.resolve() {
                errors.push(ValidationError::new(format!("{base}.taxonomy"), error));
            }
        }
        validate_mark_breakdown_taxonomy(
            &question.mark_breakdown,
            &format!("{base}.markBreakdown"),
            max_subtags,
            &mut errors,
        );

        let is_choice_question = matches!(
            question.question_type,
            super::types::QuestionType::SingleChoice
                | super::types::QuestionType::MultipleChoice
                | super::types::QuestionType::TrueFalse
        );

        if is_choice_question {
            let Some(options) = question.options.as_ref() else {
                errors.push(ValidationError::new(
                    format!("{base}.options"),
                    "Choice-based questions must have at least 2 options",
                ));
                continue;
            };

            if options.len() < 2 {
                errors.push(ValidationError::new(
                    format!("{base}.options"),
                    "Choice-based questions must have at least 2 options",
                ));
            }

            let mut seen_option_ids = std::collections::HashSet::new();
            for (option_index, option) in options.iter().enumerate() {
                if option.id.trim().is_empty() {
                    errors.push(ValidationError::new(
                        format!("{base}.options[{option_index}].id"),
                        "Option ID is required",
                    ));
                } else if !seen_option_ids.insert(option.id.as_str()) {
                    errors.push(ValidationError::new(
                        format!("{base}.options[{option_index}].id"),
                        format!("Duplicate option ID '{}'", option.id),
                    ));
                }
                if option.text.trim().is_empty() {
                    errors.push(ValidationError::new(
                        format!("{base}.options[{option_index}].text"),
                        "Option text is required",
                    ));
                }
            }

            let option_ids: std::collections::HashSet<&str> =
                options.iter().map(|option| option.id.as_str()).collect();
            if !question
                .correct_answers
                .iter()
                .all(|answer| option_ids.contains(answer.as_str()))
            {
                errors.push(ValidationError::new(
                    format!("{base}.correctAnswers"),
                    "Correct answers must match option IDs",
                ));
            }
        }

        if matches!(
            question.question_type,
            super::types::QuestionType::SingleChoice
        ) && question.correct_answers.len() != 1
        {
            errors.push(ValidationError::new(
                format!("{base}.correctAnswers"),
                "Single-choice questions must have exactly one correct answer",
            ));
        }

        if matches!(
            question.question_type,
            super::types::QuestionType::TrueFalse
        ) && question
            .options
            .as_ref()
            .map(|options| options.len())
            .unwrap_or_default()
            != 2
        {
            errors.push(ValidationError::new(
                format!("{base}.options"),
                "True/False questions must have exactly 2 options",
            ));
        }

        let unique_answers = question
            .correct_answers
            .iter()
            .collect::<std::collections::HashSet<_>>();
        if unique_answers.len() != question.correct_answers.len() {
            errors.push(ValidationError::new(
                format!("{base}.correctAnswers"),
                "Correct answers must not contain duplicates",
            ));
        }
    }

    errors
}

fn validate_mark_breakdown(
    parts: &[QuestionMarkBreakdown],
    expected_total: f64,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    let breakdown_total: f64 = parts.iter().map(|part| part.marks).sum();
    if (breakdown_total - expected_total).abs() > MARK_SUM_TOLERANCE {
        errors.push(ValidationError::new(
            path,
            "Subquestion marks must sum to their parent total",
        ));
    }

    let mut labels = std::collections::HashSet::new();
    for (index, part) in parts.iter().enumerate() {
        let part_path = format!("{path}[{index}]");
        let label = part.label.trim();
        if label.is_empty() {
            errors.push(ValidationError::new(
                format!("{part_path}.label"),
                "Mark-breakdown labels cannot be empty",
            ));
        } else if !labels.insert(label) {
            errors.push(ValidationError::new(
                format!("{part_path}.label"),
                format!("Duplicate mark-breakdown label '{}'", part.label),
            ));
        }
        if part.marks <= 0.0 {
            errors.push(ValidationError::new(
                format!("{part_path}.marks"),
                "Mark-breakdown values must be positive",
            ));
        }
        if !part.subparts.is_empty() {
            validate_mark_breakdown(
                &part.subparts,
                part.marks,
                &format!("{part_path}.subparts"),
                errors,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_bank_json() -> serde_json::Value {
        json!({
            "metadata": {
                "name": "Sample",
                "exam": "UPSC",
                "totalQuestions": 1,
                "difficulty": "medium",
                "defaultDuration": 60
            },
            "questions": [{
                "id": "q1",
                "type": "single-choice",
                "question": "Question?",
                "options": [
                    { "id": "a", "text": "A" },
                    { "id": "b", "text": "B" }
                ],
                "correctAnswers": ["a"],
                "marks": 2,
                "negativeMarks": 0.667
            }]
        })
    }

    #[test]
    fn structural_errors_keep_machine_readable_paths() {
        let mut value = valid_bank_json();
        value["questions"][0]["marks"] = json!("two");
        let errors = parse_question_bank_json(&value.to_string()).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].path.contains("questions[0].marks"));
        assert!(!errors[0].message.is_empty());
    }

    #[test]
    fn semantic_validation_aggregates_independent_failures() {
        let mut value = valid_bank_json();
        value["metadata"]["name"] = json!(" ");
        value["metadata"]["defaultDuration"] = json!(0);
        value["questions"][0]["question"] = json!("");
        value["questions"][0]["negativeMarks"] = json!(-1);

        let errors = parse_question_bank_json(&value.to_string()).unwrap_err();
        let paths = errors
            .iter()
            .map(|error| error.path.as_str())
            .collect::<Vec<_>>();
        assert!(paths.contains(&"metadata.name"));
        assert!(paths.contains(&"metadata.defaultDuration"));
        assert!(paths.contains(&"questions[0].question"));
        assert!(paths.contains(&"questions[0].negativeMarks"));
    }

    #[test]
    fn choice_rules_reject_duplicate_ids_and_unknown_answers() {
        let mut value = valid_bank_json();
        value["questions"][0]["options"][1]["id"] = json!("a");
        value["questions"][0]["correctAnswers"] = json!(["missing"]);

        let errors = parse_question_bank_json(&value.to_string()).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.message.contains("Duplicate option ID")));
        assert!(errors
            .iter()
            .any(|error| error.message == "Correct answers must match option IDs"));
    }

    #[test]
    fn numerical_and_fill_blank_allow_multiple_acceptable_answers() {
        let mut value = valid_bank_json();
        value["metadata"]["totalQuestions"] = json!(2);
        value["questions"] = json!([
            {
                "id": "n",
                "type": "numerical",
                "question": "Value?",
                "correctAnswers": ["42", "42.0"],
                "marks": 1,
                "negativeMarks": 0
            },
            {
                "id": "f",
                "type": "fill-blank",
                "question": "Capital?",
                "correctAnswers": ["Delhi", "New Delhi"],
                "marks": 1,
                "negativeMarks": 0
            }
        ]);

        assert!(parse_question_bank_json(&value.to_string()).is_ok());
    }

    #[test]
    fn typed_taxonomy_requires_the_current_explicit_version() {
        let mut value = valid_bank_json();
        value["questions"][0]["taxonomy"] = json!({ "mainTag": 7, "subtags": [] });

        let errors = parse_question_bank_json(&value.to_string()).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.path == "metadata.taxonomyVersion"));

        value["metadata"]["taxonomyVersion"] = json!(crate::taxonomy::TAXONOMY_VERSION);
        assert!(parse_question_bank_json(&value.to_string()).is_ok());
    }

    #[test]
    fn essay_taxonomy_rejects_more_than_two_subtags() {
        let mut value = valid_bank_json();
        value["metadata"]["section"] = json!("mains-essay");
        value["metadata"]["taxonomyVersion"] = json!(crate::taxonomy::TAXONOMY_VERSION);
        value["questions"][0]["taxonomy"] = json!({ "mainTag": 604, "subtags": [450, 464, 465] });

        let errors = parse_question_bank_json(&value.to_string()).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.path == "questions[0].taxonomy.subtags"));
    }

    #[test]
    fn bundled_corpus_uses_the_current_valid_taxonomy_contract() {
        let corpus_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../static/upsc");
        let catalog: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(corpus_root.join("catalog.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            catalog["taxonomyVersion"].as_u64(),
            Some(u64::from(crate::taxonomy::TAXONOMY_VERSION))
        );

        let forbidden_subject_placeholders = [
            "Mathematics",
            "Anthropology",
            "History",
            "Geography",
            "Political Science",
            "Sociology",
            "Philosophy",
            "GS1",
            "GS2",
            "GS3",
            "GS4",
            "General Studies",
            "Paper I",
            "Paper II",
            "Optional",
        ];
        let mut essay_themes = std::collections::HashSet::new();
        let mut question_count = 0usize;
        let mut map_question_count = 0usize;
        for paper in catalog["papers"].as_array().unwrap() {
            let relative_path = paper["path"].as_str().unwrap();
            let section = paper["section"].as_str().unwrap();
            let source = std::fs::read_to_string(corpus_root.join(relative_path)).unwrap();
            let bank = parse_question_bank_json(&source)
                .unwrap_or_else(|errors| panic!("{relative_path}: {errors:?}"));
            for question in bank.questions {
                question_count += 1;
                let question_text = question.question.to_ascii_lowercase();
                let is_map_question = question_text.contains("outline map")
                    || question_text.contains("marked on the map supplied");
                let question_id = question.id.clone();
                let resolved = question.taxonomy.unwrap().resolve().unwrap();
                if is_map_question {
                    map_question_count += 1;
                    assert!(
                        resolved.subtags.is_empty(),
                        "{relative_path}: {question_id} map question has taxonomy subtags"
                    );
                }
                if section == "mains-essay" {
                    essay_themes.insert(resolved.main_tag.to_string());
                    assert!(resolved.subtags.len() <= 2);
                } else {
                    assert!(
                        !forbidden_subject_placeholders.contains(&resolved.main_tag),
                        "{relative_path}: {} uses subject identity as taxonomy",
                        resolved.main_tag
                    );
                    assert!(resolved.subtags.len() <= crate::taxonomy::MAX_SUBTAGS);
                }
            }
        }
        assert_eq!(question_count, 6_200);
        assert_eq!(map_question_count, 25);
        assert_eq!(essay_themes.len(), 20);
    }
    #[test]
    fn mark_breakdown_validation_recurses_with_precise_paths() {
        let mut value = valid_bank_json();
        value["questions"][0]["markBreakdown"] = json!([
            {
                "label": "a",
                "marks": 2,
                "subparts": [
                    { "label": "i", "marks": 1.5 },
                    { "label": "i", "marks": 1 },
                    { "label": " ", "marks": 0 }
                ]
            }
        ]);

        let errors = parse_question_bank_json(&value.to_string()).unwrap_err();

        assert!(errors.iter().any(|error| {
            error.path == "questions[0].markBreakdown[0].subparts"
                && error.message.contains("parent total")
        }));
        assert!(errors.iter().any(|error| {
            error.path == "questions[0].markBreakdown[0].subparts[1].label"
                && error.message.contains("Duplicate")
        }));
        assert!(errors.iter().any(|error| {
            error.path == "questions[0].markBreakdown[0].subparts[2].label"
                && error.message.contains("cannot be empty")
        }));
        assert!(errors.iter().any(|error| {
            error.path == "questions[0].markBreakdown[0].subparts[2].marks"
                && error.message.contains("positive")
        }));
    }

    #[test]
    fn valid_nested_mark_breakdown_is_accepted() {
        let mut value = valid_bank_json();
        value["questions"][0]["markBreakdown"] = json!([
            {
                "label": "a",
                "marks": 2,
                "subparts": [
                    { "label": "i", "marks": 0.75 },
                    { "label": "ii", "marks": 1.25 }
                ]
            }
        ]);

        assert!(parse_question_bank_json(&value.to_string()).is_ok());
    }
}
