//! User settings CRUD.
//!
//! Settings are stored as key-value pairs in the `settings` table rather
//! than as columns on a single row.  This makes adding new settings a
//! code-only change (no schema migration needed).  The trade-off is that
//! we must manually map between DB keys (camelCase strings) and the
//! `Settings` struct fields.

use rusqlite::{params, Connection};

use super::helpers::upsert_setting;
use super::DbResult;
use crate::backend::error::LoopError;
use crate::backend::error::ResultExt;
use crate::backend::types::{Settings, SettingsPatch};

/// Load all settings from the database, applying defaults for any
/// missing keys.
pub fn load_settings(conn: &Connection) -> DbResult<Settings> {
    let mut stmt = conn
        .prepare("SELECT key, value FROM settings")
        .stringify_err()?;
    let mut rows = stmt.query([]).stringify_err()?;
    let mut settings = Settings::default();

    while let Some(row) = rows.next().stringify_err()? {
        let key: String = row.get("key").stringify_err()?;
        let value: String = row.get("value").stringify_err()?;

        match key.as_str() {
            "theme" => {
                if matches!(value.as_str(), "system" | "light" | "dark") {
                    settings.theme = value;
                }
            }
            "navigatorExpanded" => settings.navigator_expanded = value == "true",
            "lastLibrarySelectionId" => {
                if !value.is_empty() {
                    settings.last_library_selection_id = Some(value);
                }
            }
            "practiceShowImmediateFeedback" => {
                settings.practice_show_immediate_feedback = value == "true"
            }
            "autoSubmitOnTimerEnd" => settings.auto_submit_on_timer_end = value == "true",
            "optionalSubjectIds" => {
                settings.optional_subject_ids = serde_json::from_str(&value).unwrap_or_default()
            }
            "showOptionalResults" => settings.show_optional_results = value == "true",
            "hasCompletedOnboarding" => settings.has_completed_onboarding = value == "true",
            _ => {}
        }
    }

    Ok(settings)
}

/// Persist a partial settings patch (only provided fields are updated).
///
/// Uses a transaction so that all changed keys are committed atomically.
/// If validation fails mid-patch (e.g. bad theme), the transaction
/// aborts cleanly via early return.
pub fn save_settings(conn: &mut Connection, patch: SettingsPatch) -> DbResult<()> {
    let tx = conn.transaction().stringify_err()?;

    if let Some(theme) = patch.theme {
        if !matches!(theme.as_str(), "system" | "light" | "dark") {
            return Err(LoopError::invalid_input(format!(
                "Unsupported theme: {theme}"
            )));
        }

        tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params!["theme", theme],
        )
        .stringify_err()?;
    }

    if let Some(navigator_expanded) = patch.navigator_expanded {
        upsert_setting(&tx, "navigatorExpanded", &navigator_expanded.to_string())?;
    }

    if let Some(last_library_selection_id) = patch.last_library_selection_id {
        upsert_setting(&tx, "lastLibrarySelectionId", &last_library_selection_id)?;
    }

    if let Some(practice_feedback) = patch.practice_show_immediate_feedback {
        upsert_setting(
            &tx,
            "practiceShowImmediateFeedback",
            &practice_feedback.to_string(),
        )?;
    }

    if let Some(auto_submit) = patch.auto_submit_on_timer_end {
        upsert_setting(&tx, "autoSubmitOnTimerEnd", &auto_submit.to_string())?;
    }

    if let Some(optional_subject_ids) = patch.optional_subject_ids {
        let serialized = serde_json::to_string(&optional_subject_ids).stringify_err()?;
        upsert_setting(&tx, "optionalSubjectIds", &serialized)?;
    }

    if let Some(show_optional_results) = patch.show_optional_results {
        upsert_setting(
            &tx,
            "showOptionalResults",
            &show_optional_results.to_string(),
        )?;
    }

    if let Some(has_completed_onboarding) = patch.has_completed_onboarding {
        upsert_setting(
            &tx,
            "hasCompletedOnboarding",
            &has_completed_onboarding.to_string(),
        )?;
    }

    tx.commit().stringify_err()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::db::schema::run_migrations;

    fn connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        run_migrations(&connection).unwrap();
        connection
    }

    #[test]
    fn missing_settings_load_documented_defaults() {
        let settings = load_settings(&connection()).unwrap();
        assert_eq!(settings.theme, "system");
        assert!(!settings.navigator_expanded);
        assert_eq!(settings.last_library_selection_id, None);
        assert!(settings.practice_show_immediate_feedback);
        assert!(settings.auto_submit_on_timer_end);
        assert!(settings.optional_subject_ids.is_empty());
        assert!(!settings.show_optional_results);
        assert!(!settings.has_completed_onboarding);
    }

    #[test]
    fn partial_patch_updates_only_selected_keys() {
        let mut connection = connection();
        save_settings(
            &mut connection,
            SettingsPatch {
                theme: Some("dark".to_string()),
                navigator_expanded: Some(true),
                optional_subject_ids: Some(vec!["math".to_string()]),
                show_optional_results: Some(true),
                has_completed_onboarding: Some(true),
                ..SettingsPatch::default()
            },
        )
        .unwrap();

        let settings = load_settings(&connection).unwrap();
        assert_eq!(settings.theme, "dark");
        assert!(settings.navigator_expanded);
        assert!(settings.practice_show_immediate_feedback);
        assert_eq!(settings.optional_subject_ids, vec!["math"]);
        assert!(settings.show_optional_results);
        assert!(settings.has_completed_onboarding);
    }

    #[test]
    fn invalid_theme_does_not_commit_any_part_of_patch() {
        let mut connection = connection();
        let result = save_settings(
            &mut connection,
            SettingsPatch {
                theme: Some("sepia".to_string()),
                navigator_expanded: Some(true),
                ..SettingsPatch::default()
            },
        );

        assert!(result.unwrap_err().message().contains("Unsupported theme"));
        assert_eq!(load_settings(&connection).unwrap(), Settings::default());
    }
}
