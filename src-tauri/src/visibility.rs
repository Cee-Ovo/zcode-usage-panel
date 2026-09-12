//! UI visibility → engine idle mode. Hidden UI suspends polling; any
//! show transitions resume and kick one refresh.

use std::sync::atomic::Ordering;

use tauri::{Emitter, Manager};

/// Whether any usable UI is on screen.
///
/// A minimized window still reports `is_visible() == true`, so the minimized
/// flag has to be part of the decision — otherwise minimizing (rather than
/// hiding) would keep the engine polling at full speed. `docked_hidden` is the
/// snap module's "window parked off-screen" state, which `is_visible()` cannot
/// see either.
fn ui_visible(
    main_visible: bool,
    main_minimized: bool,
    popup_visible: bool,
    docked_hidden: bool,
) -> bool {
    popup_visible || (main_visible && !main_minimized && !docked_hidden)
}

/// Recompute whether any usable UI is visible and drive the engine's
/// auto-suspend flag. Never panics: every lookup degrades to a safe default.
pub fn update(app: &tauri::AppHandle) {
    let (main_visible, main_minimized) = app
        .get_webview_window("main")
        .map(|w| {
            (
                w.is_visible().unwrap_or(true),
                w.is_minimized().unwrap_or(false),
            )
        })
        .unwrap_or((true, false)); // window not found → assume visible
    let popup_visible = app
        .get_webview_window("popup")
        .map(|w| w.is_visible().unwrap_or(false))
        .unwrap_or(false);
    let docked_hidden = crate::windows::snap::MAIN_DOCKED_HIDDEN.load(Ordering::Relaxed);
    let visible = ui_visible(main_visible, main_minimized, popup_visible, docked_hidden);

    if let Some(state) = app.try_state::<crate::commands::SharedAppState>() {
        let engine = state.engine.clone();
        let hub = state.hub.clone();
        drop(state);
        let changed = engine.set_auto_paused(!visible);
        // Provider hub backs off its remote cadence while hidden (alerts
        // keep working, just slower).
        hub.set_paused(!visible);
        // Resume (hidden → active) → force one refresh immediately.
        if visible && changed {
            engine.kick();
            hub.kick();
        }
    }

    let _ = app.emit("ui-visibility", visible);
}

#[cfg(test)]
mod tests {
    use super::ui_visible;

    #[test]
    fn visible_window_is_active() {
        assert!(ui_visible(true, false, false, false));
    }

    #[test]
    fn hidden_window_suspends_polling() {
        assert!(!ui_visible(false, false, false, false));
    }

    /// 最小化时 `is_visible()` 仍为 true,必须靠 minimized 标志判定。
    #[test]
    fn minimized_window_suspends_polling() {
        assert!(!ui_visible(true, true, false, false));
    }

    /// 贴边隐藏时窗口仍然 "visible",同样要暂停。
    #[test]
    fn docked_hidden_window_suspends_polling() {
        assert!(!ui_visible(true, false, false, true));
    }

    /// 托盘弹窗可见即视为 UI 可见,哪怕主窗口藏着。
    #[test]
    fn visible_popup_keeps_polling_alive() {
        assert!(ui_visible(false, false, true, false));
        assert!(ui_visible(true, true, true, true));
    }
}
