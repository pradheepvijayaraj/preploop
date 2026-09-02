mod backend;
pub mod search;
mod startup;
pub mod taxonomy;
mod updater;

use tauri::Manager;

#[tauri::command]
fn supports_in_app_updates() -> bool {
    // The Linux updater replaces AppImages, not package-manager installations.
    !cfg!(target_os = "linux") || std::env::var_os("APPIMAGE").is_some()
}

/// Tauri application entry point.
///
/// STARTUP FLOW:
/// 1. Register application logging.
/// 2. In the `.setup()` hook, open the SQLite database, run migrations,
///    and store the shared connection as managed state (`DbState`).
/// 3. Register all `#[tauri::command]` handlers.
/// 4. Start the event loop.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(startup::StartupWindow::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .setup(|app| {
            startup::arm_fallback(app.handle());
            #[cfg(target_os = "macos")]
            {
                use tauri::menu::{
                    AboutMetadata, Menu, MenuItem, MenuItemKind, PredefinedMenuItem,
                };
                use tauri::Emitter;
                let menu = Menu::default(app.handle())?;
                if let Some(MenuItemKind::Submenu(app_menu)) = menu.items()?.first() {
                    app_menu.remove_at(0)?;
                    let about = PredefinedMenuItem::about(
                        app,
                        Some("About PrepLoop"),
                        Some(AboutMetadata {
                            name: Some("PrepLoop".into()),
                            version: Some(format!("v {}", app.package_info().version)),
                            short_version: Some(String::new()),
                            ..Default::default()
                        }),
                    )?;
                    let check_updates = MenuItem::with_id(
                        app,
                        "check-for-updates",
                        "Check for Updates…",
                        true,
                        None::<&str>,
                    )?;
                    app_menu.insert(&about, 0)?;
                    app_menu.insert(&check_updates, 1)?;
                }
                app.set_menu(menu)?;
                app.on_menu_event(|app, event| {
                    if event.id().as_ref() == "check-for-updates" {
                        if let Err(error) = app.emit_to("main", "check-for-updates", ()) {
                            log::warn!("Could not request an update check: {error}");
                        }
                    }
                });
            }

            // Initialise the shared database connection (#13 / #21).
            // This runs once at startup; the resulting `DbState` is
            // injected into every `#[tauri::command]` via `State<DbState>`.
            let db_state =
                backend::db::init_database(app.handle()).expect("Failed to initialise database");
            let resource_dir = app
                .path()
                .resource_dir()
                .expect("Failed to resolve application resource directory");
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("Failed to resolve application data directory");
            let search_index = backend::db::SearchIndexState::new(
                Some(resource_dir.join("models/granite-r2-q8_0.gguf")),
                data_dir.join("search-index"),
                Some(resource_dir.join("models/search-index/generation-001/vectors.bin")),
            );
            app.manage(db_state.clone());
            app.manage(search_index.clone());
            backend::commands::schedule_search_rebuild(db_state, search_index);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            startup::startup_ready,
            supports_in_app_updates,
            updater::get_pending_update,
            updater::download_pending_update,
            updater::install_pending_update,
            backend::commands::load_settings,
            backend::commands::save_settings,
            backend::commands::import_question_bank,
            backend::commands::refresh_question_bank_taxonomy,
            backend::commands::sync_bundled_question_bank,
            backend::commands::archive_missing_bundled_question_banks,
            backend::commands::get_question_banks,
            backend::commands::get_question_bank,
            backend::commands::get_question_bank_with_questions,
            backend::commands::search_questions,
            backend::commands::warm_question_search,
            backend::commands::delete_question_bank,
            backend::commands::create_test_attempt,
            backend::commands::list_test_attempt_history,
            backend::commands::save_answer,
            backend::commands::get_practice_question_feedback,
            backend::commands::toggle_flag,
            backend::commands::update_time_remaining,
            backend::commands::pause_test,
            backend::commands::resume_test,
            backend::commands::submit_test,
            backend::commands::get_test_attempt,
            backend::commands::calculate_test_result,
            backend::commands::get_question_review,
            backend::commands::get_session_payload,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
