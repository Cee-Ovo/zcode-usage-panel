//! Tray popup window: show near the tray icon, dismiss on outside click.
//!
//! Focus policy: the popup is shown with `SWP_NOACTIVATE` (and created with
//! `.focused(false)`) so it never steals keyboard focus — like the Windows
//! clock flyout. Dismissal uses a low-frequency cursor check (120 ms, one
//! `WindowFromPoint` call, only while the popup is open): when the cursor
//! leaves the popup for ~600 ms it slides away. Escape also closes it
//! (frontend → `popup_close` command).

use tauri::{AppHandle, Emitter, Manager};

pub fn ensure_window(app: &AppHandle) {
    if app.get_webview_window("popup").is_some() {
        return;
    }
    let _ = tauri::WebviewWindowBuilder::new(
        app,
        "popup",
        tauri::WebviewUrl::App("popup.html".into()),
    )
    .title("ZCode Usage")
    .inner_size(340.0, 480.0)
    .decorations(false)
    .resizable(false)
    .skip_taskbar(true)
    .always_on_top(true)
    .visible(false)
    .focused(false)
    .shadow(true)
    .build();
}

pub fn toggle(app: &AppHandle, tray_rect: Option<crate::windows::Rect>) {
    if let Some(popup) = app.get_webview_window("popup") {
        if popup.is_visible().unwrap_or(false) {
            hide(app);
            crate::visibility::update(app);
            return;
        }
        let scale = popup.scale_factor().unwrap_or(1.0);
        let (w, h) = popup
            .outer_size()
            .map(|s| (s.width as i32, s.height as i32))
            .unwrap_or(((340.0 * scale) as i32, (480.0 * scale) as i32));

        let rect = tray_rect.or_else(fallback_rect_from_cursor);
        if let Some(rect) = rect {
            #[cfg(windows)]
            if let Some((x, y)) =
                crate::windows::popup_pos::compute_position(rect, w, h)
            {
                let _ = popup.set_position(tauri::PhysicalPosition::new(x, y));
            }
            #[cfg(not(windows))]
            let _ = rect;
        }
        let _ = popup.show();
        #[cfg(windows)]
        if let Ok(hwnd) = popup.hwnd() {
            crate::windows::native::show_no_activate(hwnd.0 as usize);
        }
        let _ = app.emit_to("popup", "popup-refresh", ());
        spawn_dismiss_watcher(app.clone());
        crate::visibility::update(app);
    }
}

pub fn hide(app: &AppHandle) {
    if let Some(popup) = app.get_webview_window("popup") {
        let _ = popup.hide();
    }
    crate::visibility::update(app);
}

#[cfg(windows)]
fn fallback_rect_from_cursor() -> Option<crate::windows::Rect> {
    let (x, y) = crate::windows::native::cursor_position()?;
    Some(crate::windows::Rect {
        left: x - 8,
        top: y - 8,
        right: x + 8,
        bottom: y + 8,
    })
}

#[cfg(not(windows))]
fn fallback_rect_from_cursor() -> Option<crate::windows::Rect> {
    None
}

/// Watch the cursor while the popup is open; hide after it leaves.
#[cfg(windows)]
fn spawn_dismiss_watcher(app: AppHandle) {
    std::thread::spawn(move || {
        let popup = match app.get_webview_window("popup") {
            Some(p) => p,
            None => return,
        };
        let hwnd = match popup.hwnd() {
            Ok(h) => h.0 as usize,
            Err(_) => return,
        };
        let mut outside_since: Option<std::time::Instant> = None;
        for _ in 0..500 {
            // ~60 s hard cap while visible
            std::thread::sleep(std::time::Duration::from_millis(120));
            if !popup.is_visible().unwrap_or(false) {
                return;
            }
            let over = crate::windows::native::cursor_position()
                .map(|(x, y)| crate::windows::native::point_is_over_window(x, y, hwnd))
                .unwrap_or(true); // can't tell → don't dismiss
            if over {
                outside_since = None;
            } else {
                let since = *outside_since.get_or_insert(std::time::Instant::now());
                if since.elapsed() >= std::time::Duration::from_millis(600) {
                    let _ = popup.hide();
                    crate::visibility::update(&app);
                    return;
                }
            }
        }
    });
}

#[cfg(not(windows))]
fn spawn_dismiss_watcher(_app: AppHandle) {}
