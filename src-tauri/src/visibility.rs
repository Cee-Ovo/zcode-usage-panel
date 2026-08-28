//! UI visibility → engine idle mode. Hidden UI suspends polling; any
//! show transitions resume and kick one refresh.

use std::sync::atomic::Ordering;

use tauri::{Emitter, Manager};

/// Recompute whether any usable UI is visible and drive the engine's
/// auto-suspend flag. Never panics: every lookup degrades to a safe default.
pub fn update(app: &tauri::AppHandle) {
    let main_visible = app
        .get_webview_window("main")
        .map(|w| w.is_visible().unwrap_or(true))
        .unwrap_or(true); // window not found → assume visible
    let popup_visible = app
        .get_webview_window("popup")
        .map(|w| w.is_visible().unwrap_or(false))
        .unwrap_or(false);
    let docked_hidden = crate::windows::snap::MAIN_DOCKED_HIDDEN.load(Ordering::Relaxed);
    let ui_visible = popup_visible || (main_visible && !docked_hidden);

    if let Some(state) = app.try_state::<crate::commands::SharedAppState>() {
        let engine = state.engine.clone();
        drop(state);
        let changed = engine.set_auto_paused(!ui_visible);
        // Resume (hidden → active) → force one refresh immediately.
        if ui_visible && changed {
            engine.kick();
        }
    }

    let _ = app.emit("ui-visibility", ui_visible);
}
