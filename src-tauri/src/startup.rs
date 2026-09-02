//! Reveal the initially hidden window only after the frontend has applied its theme.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tauri::{AppHandle, Manager, State, WebviewWindow};

#[derive(Clone, Default)]
pub struct StartupWindow(Arc<AtomicBool>);

impl StartupWindow {
    fn reveal(&self, window: &WebviewWindow) -> Result<bool, String> {
        // The watchdog and renderer can race. Never reopen/refocus a window
        // after the user has already seen (and possibly minimized) it.
        if self.0.swap(true, Ordering::AcqRel) {
            return Ok(false);
        }
        if let Err(error) = window.show() {
            self.0.store(false, Ordering::Release);
            return Err(error.to_string());
        }
        Ok(true)
    }
}

#[tauri::command]
pub fn startup_ready(window: WebviewWindow, state: State<StartupWindow>) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Only the main window can complete startup".into());
    }
    if state.reveal(&window)? {
        if let Err(error) = window.set_focus() {
            log::warn!("Could not focus the startup window: {error}");
        }
    }
    Ok(())
}

pub fn arm_fallback(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let state = app.state::<StartupWindow>().inner().clone();
    // A failed frontend must not leave an invisible, apparently unlaunchable app.
    // This recovery path does not focus/raise an already-visible window.
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(10));
        if state.0.load(Ordering::Acquire) {
            return;
        }
        log::warn!("Startup readiness timed out; revealing the window for recovery");
        if let Err(error) = state.reveal(&window) {
            log::error!("Could not reveal the startup window: {error}");
        }
    });
}
