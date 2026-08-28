//! ZCode Usage Panel — application assembly.
//!
//! Startup order matters:
//! 1. plugins (single-instance must be first),
//! 2. settings load → shared settings cell,
//! 3. engine (background watcher/refresh thread),
//! 4. main window behaviors (always-on-top, edge-docking manager) + reveal,
//! 5. popup window (hidden) and tray icon.

mod alerts;
mod commands;
mod engine;
mod export;
mod popup;
mod settings;
mod tray;
mod visibility;
mod windows;
pub mod zcode;

use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

use tauri::{Manager, WindowEvent};

use crate::commands::{SharedAppState, AppState};
use crate::engine::Engine;
use crate::settings::{Settings, WindowState};
use crate::windows::snap::{SnapManager, SnapMsg};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // Second instance: focus the running one instead.
            let state = app.state::<SharedAppState>();
            tray::reveal_main(app, &state);
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_bootstrap,
            commands::get_dashboard,
            commands::get_trend,
            commands::get_sessions,
            commands::get_session_detail,
            commands::get_model_detail,
            commands::get_alerts,
            commands::get_active_models,
            commands::set_settings,
            commands::diagnose,
            commands::refresh_now,
            commands::hide_main_window,
            commands::cost_summary,
            commands::cost_detail,
            commands::pricing_table,
            commands::pricing_refresh,
            commands::pricing_override,
            commands::export_data,
            commands::dock_hover,
            commands::dock_interact,
            commands::popup_close,
            commands::quit_app,
        ])
        .setup(|app| setup(app))
        .on_window_event(|window, event| on_window_event(window, event))
        .build(tauri::generate_context!())
        .expect("fatal: failed to build Tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                let state = app.state::<SharedAppState>();
                state.engine.save_snapshot();
                let s = state.settings.read().unwrap().clone();
                settings::save(app, &s);
            }
        });
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    let boot_settings = settings::load(&handle);
    let settings_arc = Arc::new(std::sync::RwLock::new(boot_settings.clone()));

    // Engine + shared state.
    let (engine, rx) = Engine::new();
    engine.set_app(handle.clone());
    engine.load_snapshot(&handle);

    // Pricing manager: built-in official price table + FX/override persistence
    // + background refresh; handed to the engine idle loop for the daily pull.
    let pricing_manager = Arc::new(crate::zcode::pricing::PricingManager::new(
        handle.path().app_config_dir().ok(),
    ));
    engine.set_pricing(pricing_manager.clone());

    let state: SharedAppState = Arc::new(AppState {
        settings: settings_arc.clone(),
        engine: engine.clone(),
        pricing: pricing_manager.clone(),
        settings_dirty: std::sync::atomic::AtomicBool::new(false),
        snap: OnceLock::new(),
    });
    app.manage(state.clone());

    // Background refresh thread.
    let thread_settings = settings_arc.clone();
    let engine_for_thread = engine.clone();
    std::thread::Builder::new()
        .name("zup-engine".into())
        .spawn(move || {
            engine_for_thread.run_background(rx, move || {
                thread_settings.read().unwrap().clone()
            });
        })?;

    // Main window: always-on-top, docking engine, delayed reveal (the config
    // keeps it invisible until the docked/restored position is applied).
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.set_always_on_top(boot_settings.always_on_top);

        let get_settings: Arc<dyn Fn() -> Settings + Send + Sync> = {
            let s = settings_arc.clone();
            Arc::new(move || s.read().unwrap().clone())
        };
        let save_window: Arc<dyn Fn(WindowState) + Send + Sync> = {
            let s = settings_arc.clone();
            let h = handle.clone();
            Arc::new(move |ws: WindowState| {
                let snapshot = {
                    let mut guard = s.write().unwrap();
                    guard.window = ws;
                    guard.clone()
                };
                settings::save(&h, &snapshot);
            })
        };
        let snap = SnapManager::spawn(main.clone(), get_settings, save_window);
        let _ = state.snap.set(snap);

        let reveal = main.clone();
        std::thread::spawn(move || {
            // Give the docking thread a beat to apply the restored geometry.
            std::thread::sleep(std::time::Duration::from_millis(60));
            let _ = reveal.show();
        });
    }

    popup::ensure_window(&handle);
    tray::build_tray(&handle)?;
    Ok(())
}

fn on_window_event(window: &tauri::Window, event: &WindowEvent) {
    let app = window.app_handle();
    let state = app.state::<SharedAppState>();
    match window.label() {
        "main" => match event {
            WindowEvent::CloseRequested { api, .. } => {
                let s = state.settings.read().unwrap().clone();
                if s.close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                } else {
                    state.engine.save_snapshot();
                    settings::save(app, &s);
                    app.exit(0);
                }
            }
            WindowEvent::Moved(pos) => {
                if let (Some(snap), Ok(size)) = (state.snap.get(), window.outer_size()) {
                    snap.send(SnapMsg::Moved {
                        x: pos.x,
                        y: pos.y,
                        w: size.width,
                        h: size.height,
                    });
                }
            }
            WindowEvent::Resized(_) => {
                if let Some(snap) = state.snap.get() {
                    snap.send(SnapMsg::Resized);
                }
            }
            WindowEvent::Focused(b) => {
                if let Some(snap) = state.snap.get() {
                    snap.send(SnapMsg::Focused(*b));
                }
                if *b {
                    popup::hide(app);
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(snap) = state.snap.get() {
                    snap.send(SnapMsg::DisplayChange);
                }
            }
            _ => {}
        },
        "popup" => match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            _ => {}
        },
        _ => {}
    }
}

/// Mark settings dirty for the periodic saver (kept for future use by
/// high-frequency writers; direct saves are throttled by callers today).
#[allow(dead_code)]
fn mark_dirty(state: &AppState) {
    state.settings_dirty.store(true, Ordering::Relaxed);
}
