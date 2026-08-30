//! System tray: menu, today-popup trigger, check-state syncing.

use tauri::menu::{CheckMenuItem, CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::{current_settings, SharedAppState};
use crate::settings::{self, Settings};
use crate::windows::Rect;

#[derive(Clone)]
pub struct TrayHandles {
    pub pause: CheckMenuItem<tauri::Wry>,
    pub snap: CheckMenuItem<tauri::Wry>,
    pub aot: CheckMenuItem<tauri::Wry>,
    pub startup: CheckMenuItem<tauri::Wry>,
}

pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItemBuilder::with_id("show", "显示主面板").build(app)?;
    let today = MenuItemBuilder::with_id("today", "今日用量").build(app)?;
    let launch_zcode = MenuItemBuilder::with_id("launch_zcode", "启动 ZCode").build(app)?;
    let reveal_zcode = MenuItemBuilder::with_id("reveal_zcode", "显示 ZCode").build(app)?;
    let pause = CheckMenuItemBuilder::with_id("pause", "暂停监控").build(app)?;
    let snap = CheckMenuItemBuilder::with_id("snap", "边缘吸附").build(app)?;
    let aot = CheckMenuItemBuilder::with_id("aot", "Always on Top").build(app)?;
    let startup = CheckMenuItemBuilder::with_id("startup", "开机启动").build(app)?;
    let open_settings = MenuItemBuilder::with_id("settings", "设置").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "退出").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&show)
        .item(&today)
        .separator()
        .item(&launch_zcode)
        .item(&reveal_zcode)
        .separator()
        .item(&pause)
        .item(&snap)
        .item(&aot)
        .item(&startup)
        .separator()
        .item(&open_settings)
        .item(&quit)
        .build()?;

    let s = app.state::<SharedAppState>();
    let settings = current_settings(&s);
    pause.set_checked(settings.monitoring_paused);
    snap.set_checked(settings.snap.enabled);
    aot.set_checked(settings.always_on_top);
    startup.set_checked(settings.autostart);
    drop(s);

    TrayIconBuilder::with_id("main-tray")
        .icon(
            app.default_window_icon()
                .cloned()
                .expect("app icon configured"),
        )
        .tooltip("ZCode Usage Panel")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_menu(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } = event
            {
                let (x, y, w, h) = match (rect.position, rect.size) {
                    (tauri::Position::Physical(p), tauri::Size::Physical(s)) => {
                        (p.x, p.y, s.width as i32, s.height as i32)
                    }
                    (tauri::Position::Logical(p), tauri::Size::Logical(s)) => (
                        p.x.round() as i32,
                        p.y.round() as i32,
                        s.width.round() as i32,
                        s.height.round() as i32,
                    ),
                    // Mixed physical/logical variants never occur for tray
                    // icons (both are reported physical); ignore defensively.
                    _ => return,
                };
                let r = Rect {
                    left: x,
                    top: y,
                    right: x + w,
                    bottom: y + h,
                };
                crate::popup::toggle(tray.app_handle(), Some(r));
            }
        })
        .build(app)?;

    app.manage(TrayHandles { pause, snap, aot, startup });
    Ok(())
}

/// Re-sync menu checkmarks after any settings change.
pub fn sync_checks(app: &AppHandle, s: &Settings) {
    if let Some(h) = app.try_state::<TrayHandles>() {
        let _ = h.pause.set_checked(s.monitoring_paused);
        let _ = h.snap.set_checked(s.snap.enabled);
        let _ = h.aot.set_checked(s.always_on_top);
        let _ = h.startup.set_checked(s.autostart);
    }
}

fn handle_menu(app: &AppHandle, id: &str) {
    let state = app.state::<SharedAppState>();
    let mut settings = current_settings(&state);    match id {
        "show" => reveal_main(app, &state),
        "today" => {
            crate::popup::toggle(app, None);
        }
        "launch_zcode" => {
            let s = current_settings(&state);
            let _ = state.hub.launcher_action("launch", &s);
        }
        "reveal_zcode" => {
            let s = current_settings(&state);
            let _ = state.hub.launcher_action("reveal", &s);
        }        "pause" => {
            settings.monitoring_paused = !settings.monitoring_paused;
            apply_settings(app, &state, settings);
        }
        "snap" => {
            settings.snap.enabled = !settings.snap.enabled;
            apply_settings(app, &state, settings);
        }
        "aot" => {
            settings.always_on_top = !settings.always_on_top;
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_always_on_top(settings.always_on_top);
            }
            apply_settings(app, &state, settings);
        }
        "startup" => {
            settings.autostart = !settings.autostart;
            use tauri_plugin_autostart::ManagerExt;
            let _ = if settings.autostart {
                app.autolaunch().enable()
            } else {
                app.autolaunch().disable()
            };
            apply_settings(app, &state, settings);
        }
        "settings" => {
            reveal_main(app, &state);
            drop(state);
            let _ = app.emit("navigate", "settings");
        }
        "quit" => {
            let engine = state.engine.clone();
            drop(state);
            engine.save_snapshot();
            let s = app
                .state::<SharedAppState>()
                .settings
                .read()
                .unwrap()
                .clone();
            settings::save(app, &s);
            app.exit(0);
        }
        _ => {}
    }
}

fn apply_settings(app: &AppHandle, state: &SharedAppState, settings: Settings) {
    *state.settings.write().unwrap() = settings.clone();
    state.engine.kick();
    settings::save(app, &settings);
    sync_checks(app, &settings);
    let _ = app.emit("settings-changed", &settings);
}

pub fn reveal_main(app: &AppHandle, state: &SharedAppState) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        crate::popup::hide(app);
        // If the window is docked-and-hidden, reveal it at the docked edge.
        if let Some(snap) = state.snap.get() {
            snap.send(crate::windows::snap::SnapMsg::Reveal);
        }
        let _ = win.set_focus();
    }
    crate::visibility::update(app);
}
