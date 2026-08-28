//! QQ-style edge docking and auto-hide (Windows).
//!
//! Behavior:
//! - While the user drags the frameless window, `Moved` events stream in.
//!   A side within the snap threshold becomes a *candidate*. After 160 ms of
//!   event quiescence (drag released / paused at the edge) the window docks:
//!   it is aligned flush to the work-area edge and, if auto-hide is on,
//!   slides out of sight after `hide_delay_ms`, leaving a 4 px trigger strip.
//! - While hidden, a 60 ms `GetCursorPos` check (one Win32 call — no message
//!   hooks, no busy loop; runs ONLY while docked-and-hidden) detects the
//!   cursor entering the edge strip and slides the window back in over
//!   `anim_ms` with an ease-out curve.
//! - While docked and visible, the window re-hides only when ALL of:
//!   mouse not over the window, no open overlay (menus/tooltips — reported
//!   by the frontend), window not focused, and `hide_delay_ms` elapsed.
//! - Dragging away from the edge un-docks. Resizing while docked re-clamps.
//! - Work areas are queried per-monitor via Win32 (`MonitorFromWindow` +
//!   `GetMonitorInfoW`), so taskbar placement, 100–200 % DPI and
//!   multi-monitor arrangements are handled in physical pixels throughout.
//! - Display changes (`ScaleFactorChanged`, sleep/resume) and a 4 s
//!   periodic revalidation re-clamp a docked window whose monitor changed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use tauri::Manager;
use tauri::WebviewWindow;

use super::native;
use super::{Rect, Side};
use crate::settings::{Settings, SnapSettings, WindowState};

/// True while the docked main window is auto-hidden (moved off-screen).
/// Mirrored by `Runtime::hidden` so visibility → engine idle mode can read
/// it without touching the snap thread.
pub static MAIN_DOCKED_HIDDEN: AtomicBool = AtomicBool::new(false);

const STRIP_PX: i32 = 4; // visible sliver while hidden
const EDGE_TRIGGER_PX: i32 = 3; // cursor strip that wakes a hidden window
const QUIESCE_MS: u64 = 160; // drag-settle time before docking
const UNDOCK_SLACK_PX: f64 = 16.0; // extra drag distance required to un-dock

#[derive(Clone, Copy, Debug)]
pub enum SnapMsg {
    Moved { x: i32, y: i32, w: u32, h: u32 },
    Resized,
    Focused(bool),
    Hover(bool),
    Interact(bool),
    Reveal,
    DisplayChange,
    ShutDown,
}

#[derive(Clone)]
pub struct SnapManager {
    tx: Sender<SnapMsg>,
}

impl SnapManager {
    pub fn send(&self, msg: SnapMsg) {
        let _ = self.tx.send(msg);
    }

    /// Spawn the docking engine thread for the main window.
    pub fn spawn(
        window: WebviewWindow,
        get_settings: Arc<dyn Fn() -> Settings + Send + Sync>,
        save_window: Arc<dyn Fn(WindowState) + Send + Sync>,
    ) -> SnapManager {
        let (tx, rx) = mpsc::channel::<SnapMsg>();
        thread::Builder::new()
            .name("zup-snap".into())
            .spawn(move || {
                let mut rt = Runtime::new(&window);
                run(&window, &mut rt, rx, get_settings, save_window);
            })
            .expect("failed to spawn snap thread");
        SnapManager { tx }
    }
}

struct Runtime {
    hwnd: Option<usize>,
    side: Option<Side>,
    hidden: bool,
    candidate: Option<Side>,
    last_move: Option<Instant>,
    rect: Option<Rect>,
    hovering: bool,
    interacting: bool,
    focused: bool,
    last_activity: Option<Instant>,
    hide_at: Option<Instant>,
    animating: Arc<AtomicBool>,
    last_persist: Option<Instant>,
    last_revalidate: Option<Instant>,
}

impl Runtime {
    fn new(window: &WebviewWindow) -> Self {
        Runtime {
            hwnd: window.hwnd().ok().map(|h| h.0 as usize),
            side: None,
            hidden: false,
            candidate: None,
            last_move: None,
            rect: None,
            hovering: false,
            interacting: false,
            focused: false,
            last_activity: None,
            hide_at: None,
            animating: Arc::new(AtomicBool::new(false)),
            last_persist: None,
            last_revalidate: None,
        }
    }

    fn work_area(&self) -> Option<(Rect, Rect)> {
        self.hwnd.and_then(native::work_area_of_window)
    }

    fn scale(&self, window: &WebviewWindow) -> f64 {
        window.scale_factor().unwrap_or(1.0)
    }
}

fn flush_position(side: Side, rect: Rect, work: Rect) -> (i32, i32) {
    match side {
        Side::Left => (work.left, rect.top),
        Side::Right => (work.right - rect.width(), rect.top),
        Side::Top => (rect.left, work.top),
    }
}

fn hidden_position(side: Side, rect: Rect, work: Rect) -> (i32, i32) {
    match side {
        Side::Left => (work.left - rect.width() + STRIP_PX, rect.top),
        Side::Right => (work.right - STRIP_PX, rect.top),
        Side::Top => (rect.left, work.top - rect.height() + STRIP_PX),
    }
}

fn edge_distances(rect: &Rect, work: &Rect) -> (f64, f64, f64) {
    (
        (rect.left - work.left).max(0) as f64,
        (work.right - rect.right).max(0) as f64,
        (rect.top - work.top).max(0) as f64,
    )
}

/// Animation length for reveal (respecting the 150–250 ms window).
fn snap_anim(window: &WebviewWindow) -> u64 {
    let _ = window;
    200
}

fn pick_candidate(rect: &Rect, work: &Rect, threshold: f64, snap: &SnapSettings) -> Option<Side> {
    let (dl, dr, dt) = edge_distances(rect, work);
    let mut best: Option<(f64, Side)> = None;
    let consider = |dist: f64, side: Side, best: &mut Option<(f64, Side)>| {
        if dist <= threshold {
            if best.map(|(d, _)| dist < d).unwrap_or(true) {
                *best = Some((dist, side));
            }
        }
    };
    if snap.sides.left {
        consider(dl, Side::Left, &mut best);
    }
    if snap.sides.right {
        consider(dr, Side::Right, &mut best);
    }
    if snap.sides.top {
        consider(dt, Side::Top, &mut best);
    }
    best.map(|(_, s)| s)
}

fn run(
    window: &WebviewWindow,
    rt: &mut Runtime,
    rx: Receiver<SnapMsg>,
    get_settings: Arc<dyn Fn() -> Settings + Send + Sync>,
    save_window: Arc<dyn Fn(WindowState) + Send + Sync>,
) {
    // ---- initial restore ---------------------------------------------------
    let initial_snap = get_settings().snap;
    let initial_win = get_settings().window;
    if initial_snap.enabled {
            if let Some(side) = initial_win.dock_side.as_deref().and_then(Side::from_str) {
                if let Some((_, work)) = rt.work_area() {
                    let Ok(pos) = window.outer_position() else { return };
                    let Ok(size) = window.outer_size() else { return };
                    let (w, h) = (size.width as i32, size.height as i32);
                    let (x, y) = if initial_win.dock_hidden {
                        rt.hidden = true;
                        MAIN_DOCKED_HIDDEN.store(true, Ordering::Relaxed);
                        crate::visibility::update(window.app_handle());
                        hidden_position(
                            side,
                            Rect {
                                left: pos.x,
                                top: pos.y,
                                right: pos.x + w,
                                bottom: pos.y + h,
                            },
                            work,
                        )
                    } else {
                        flush_position(
                            side,
                            Rect {
                                left: pos.x,
                                top: pos.y,
                                right: pos.x + w,
                                bottom: pos.y + h,
                            },
                            work,
                        )
                    };
                    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
                    rt.rect = Some(Rect {
                        left: x,
                        top: y,
                        right: x + w,
                        bottom: y + h,
                    });
                    rt.side = Some(side);
                }
            } else if initial_win.width > 0 && initial_win.height > 0 {
            let _ = window.set_size(tauri::PhysicalSize::new(
                initial_win.width,
                initial_win.height,
            ));
            let _ = window.set_position(tauri::PhysicalPosition::new(
                initial_win.x,
                initial_win.y,
            ));
        }
    } else if initial_win.width > 0 && initial_win.height > 0 {
        let _ = window.set_size(tauri::PhysicalSize::new(initial_win.width, initial_win.height));
        let _ = window.set_position(tauri::PhysicalPosition::new(initial_win.x, initial_win.y));
    }

    loop {
        let timeout = if rt.hidden {
            Duration::from_millis(60)
        } else if rt.side.is_some() {
            Duration::from_millis(200)
        } else {
            Duration::from_millis(500)
        };
        match rx.recv_timeout(timeout) {
            Ok(msg) => {
                if !handle(window, rt, msg, &get_settings) {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        tick(window, rt, &get_settings, &save_window);
    }
    // Exiting: persist the final geometry.
    persist(window, rt, &save_window, true);
}

/// Returns false when the loop should stop.
fn handle(
    window: &WebviewWindow,
    rt: &mut Runtime,
    msg: SnapMsg,
    get_settings: &Arc<dyn Fn() -> Settings + Send + Sync>,
) -> bool {
    match msg {
        SnapMsg::ShutDown => return false,
        SnapMsg::Moved { x, y, w, h } => {
            if rt.animating.load(Ordering::Relaxed) {
                return true; // our own animation move — ignore
            }
            let rect = Rect {
                left: x,
                top: y,
                right: x + w as i32,
                bottom: y + h as i32,
            };
            rt.rect = Some(rect);
            rt.last_move = Some(Instant::now());
            rt.last_activity = Some(Instant::now());

            let settings = get_settings();
            let snap = settings.snap;
            if !snap.enabled {
                return true;
            }
            let Some((_, work)) = rt.work_area() else { return true };
            let threshold = snap.threshold_px * rt.scale(window);
            match rt.side {
                Some(side) => {
                    // Dragged away from the docked edge? → un-dock.
                    let (dl, dr, dt) = edge_distances(&rect, &work);
                    let dist = match side {
                        Side::Left => dl,
                        Side::Right => dr,
                        Side::Top => dt,
                    };
                    if dist > threshold + UNDOCK_SLACK_PX {
                        rt.side = None;
                        rt.hidden = false;
                        MAIN_DOCKED_HIDDEN.store(false, Ordering::Relaxed);
                        crate::visibility::update(window.app_handle());
                        rt.hide_at = None;
                        rt.candidate = None;
                    }
                }
                None => {
                    rt.candidate = pick_candidate(&rect, &work, threshold, &snap);
                }
            }
        }
        SnapMsg::Resized => {
            // Re-clamp to the docked edge after a resize.
            if let (Some(side), Some(rect), Some((_, work))) = (rt.side, rt.rect, rt.work_area()) {
                let (x, y) = if rt.hidden {
                    hidden_position(side, rect, work)
                } else {
                    flush_position(side, rect, work)
                };
                let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
            }
            rt.last_activity = Some(Instant::now());
        }
        SnapMsg::Focused(b) => {
            rt.focused = b;
            if b {
                rt.last_activity = Some(Instant::now());
            }
        }
        SnapMsg::Hover(b) => {
            rt.hovering = b;
            if b {
                rt.last_activity = Some(Instant::now());
            }
        }
        SnapMsg::Interact(b) => {
            rt.interacting = b;
            if b {
                rt.last_activity = Some(Instant::now());
            }
        }
        SnapMsg::Reveal => {
            // Tray / single-instance activation: slide a hidden docked
            // window back into view.
            if rt.side.is_some() && rt.hidden {
                if let (Some(side), Some(rect), Some((_, work))) = (rt.side, rt.rect, rt.work_area())
                {
                    let (x, y) = flush_position(side, rect, work);
                    animate_to(window, rt, rect.left, rect.top, x, y, snap_anim(window));
                }
                rt.hidden = false;
                MAIN_DOCKED_HIDDEN.store(false, Ordering::Relaxed);
                crate::visibility::update(window.app_handle());
                rt.hide_at = None;
                rt.last_activity = Some(Instant::now());
            }
        }
        SnapMsg::DisplayChange => {
            rt.last_revalidate = None; // force revalidation on next tick
        }
    }
    true
}

fn tick(
    window: &WebviewWindow,
    rt: &mut Runtime,
    get_settings: &Arc<dyn Fn() -> Settings + Send + Sync>,
    save_window: &Arc<dyn Fn(WindowState) + Send + Sync>,
) {
    let settings = get_settings();
    let snap = settings.snap;

    // Feature turned off while docked → bring the window back on screen.
    if !snap.enabled {
        if rt.side.is_some() {
            if rt.hidden {
                if let (Some(side), Some(rect), Some((_, work))) = (rt.side, rt.rect, rt.work_area())
                {
                    let (x, y) = flush_position(side, rect, work);
                    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
                }
            }
            rt.side = None;
            rt.hidden = false;
            MAIN_DOCKED_HIDDEN.store(false, Ordering::Relaxed);
            crate::visibility::update(window.app_handle());
            rt.candidate = None;
            rt.hide_at = None;
        }
        return;
    }

    // Dock after the drag settles.
    if rt.candidate.is_some()
        && rt
            .last_move
            .map(|t| t.elapsed() >= Duration::from_millis(QUIESCE_MS))
            .unwrap_or(false)
    {
        let side = rt.candidate.take().unwrap();
        if let (Some(rect), Some((_, work))) = (rt.rect, rt.work_area()) {
            let (x, y) = flush_position(side, rect, work);
            animate_to(window, rt, rect.left, rect.top, x, y, snap.anim_ms.min(240).max(80));
            rt.side = Some(side);
            rt.hidden = false;
            MAIN_DOCKED_HIDDEN.store(false, Ordering::Relaxed);
            crate::visibility::update(window.app_handle());
            rt.last_activity = Some(Instant::now());
            rt.hide_at = Some(Instant::now() + Duration::from_millis(snap.hide_delay_ms.max(300)));
            persist(window, rt, save_window, false);
        }
        return;
    }

    match (rt.side, rt.hidden) {
        (Some(side), true) => {
            // Hidden: watch the edge strip for the cursor.
            if let (Some(_), Some((_, work))) = (rt.hwnd, rt.work_area()) {
                if let Some((cx, cy)) = native::cursor_position() {
                    let hit = match side {
                        Side::Left => cx <= work.left + EDGE_TRIGGER_PX && cy >= work.top && cy <= work.bottom,
                        Side::Right => cx >= work.right - EDGE_TRIGGER_PX && cy >= work.top && cy <= work.bottom,
                        Side::Top => cy <= work.top + EDGE_TRIGGER_PX && cx >= work.left && cx <= work.right,
                    };
                    if hit {
                        if let Some(rect) = rt.rect {
                            let (x, y) = flush_position(side, rect, work);
                            animate_to(window, rt, rect.left, rect.top, x, y, snap.anim_ms.min(240).max(80));
                        }
                        rt.hidden = false;
                        MAIN_DOCKED_HIDDEN.store(false, Ordering::Relaxed);
                        crate::visibility::update(window.app_handle());
                        rt.last_activity = Some(Instant::now());
                        rt.hide_at = None;
                        persist(window, rt, save_window, false);
                    }
                }
            }
        }
        (Some(_side), false) => {
            // Visible + docked: auto-hide when idle.
            let idle_long_enough = rt
                .last_activity
                .map(|t| t.elapsed() >= Duration::from_millis(snap.hide_delay_ms))
                .unwrap_or(true);
            let hide_deadline_passed = rt
                .hide_at
                .map(|t| Instant::now() >= t)
                .unwrap_or(true);
            if snap.auto_hide
                && idle_long_enough
                && hide_deadline_passed
                && !rt.hovering
                && !rt.interacting
                && !rt.focused
                && !rt.animating.load(Ordering::Relaxed)
            {
                if let (Some(side), Some(rect), Some((_, work))) = (rt.side, rt.rect, rt.work_area())
                {
                    let (x, y) = hidden_position(side, rect, work);
                    animate_to(window, rt, rect.left, rect.top, x, y, snap.anim_ms.min(240).max(80));
                    rt.hidden = true;
                    MAIN_DOCKED_HIDDEN.store(true, Ordering::Relaxed);
                    crate::visibility::update(window.app_handle());
                    persist(window, rt, save_window, false);
                }
            }
        }
        _ => {}
    }

    // Periodic revalidation (display change, sleep/resume, monitor unplug).
    let revalidate_due = rt
        .last_revalidate
        .map(|t| t.elapsed() >= Duration::from_secs(4))
        .unwrap_or(true);
    if rt.side.is_some() && revalidate_due {
        rt.last_revalidate = Some(Instant::now());
        if let (Some(side), Some(rect), Some((_, work))) = (rt.side, rt.rect, rt.work_area()) {
            let target = if rt.hidden {
                hidden_position(side, rect, work)
            } else {
                flush_position(side, rect, work)
            };
            if rect.left != target.0 || rect.top != target.1 {
                if rt.animating.load(Ordering::Relaxed) {
                    return;
                }
                let _ = window.set_position(tauri::PhysicalPosition::new(target.0, target.1));
                rt.rect = Some(Rect {
                    left: target.0,
                    top: target.1,
                    right: target.0 + rect.width(),
                    bottom: target.1 + rect.height(),
                });
            }
        }
    }
}

/// Animate the window position with an ease-out cubic curve.
fn animate_to(
    window: &WebviewWindow,
    rt: &mut Runtime,
    from_x: i32,
    from_y: i32,
    to_x: i32,
    to_y: i32,
    ms: u64,
) {
    // Cancel any in-flight animation first.
    rt.animating.store(true, Ordering::Relaxed);
    let flag = rt.animating.clone();
    let win = window.clone();
    thread::spawn(move || {
        let steps = (ms / 16).max(1);
        for i in 1..=steps {
            if !flag.load(Ordering::Relaxed) {
                return; // superseded
            }
            let t = i as f64 / steps as f64;
            let eased = 1.0 - (1.0 - t).powi(3);
            let x = from_x as f64 + (to_x as f64 - from_x as f64) * eased;
            let y = from_y as f64 + (to_y as f64 - from_y as f64) * eased;
            let _ = win.set_position(tauri::PhysicalPosition::new(x as i32, y as i32));
            thread::sleep(Duration::from_millis(16));
        }
        let _ = win.set_position(tauri::PhysicalPosition::new(to_x, to_y));
        flag.store(false, Ordering::Relaxed);
    });
    // Optimistically update the believed rect so logic uses the target.
    if let Some(rect) = &mut rt.rect {
        let (w, h) = (rect.width(), rect.height());
        *rect = Rect {
            left: to_x,
            top: to_y,
            right: to_x + w,
            bottom: to_y + h,
        };
    }
}

fn persist(
    window: &WebviewWindow,
    rt: &Runtime,
    save_window: &Arc<dyn Fn(WindowState) + Send + Sync>,
    force: bool,
) {
    let now = Instant::now();
    if !force {
        if let Some(last) = rt.last_persist {
            if now.duration_since(last) < Duration::from_millis(700) {
                return;
            }
        }
    }
    let (x, y, w, h) = match rt.rect {
        Some(r) => (r.left, r.top, r.width() as u32, r.height() as u32),
        None => {
            let pos = window.outer_position().unwrap_or_default();
            let size = window.outer_size().unwrap_or_default();
            (pos.x, pos.y, size.width, size.height)
        }
    };
    save_window(WindowState {
        x,
        y,
        width: w,
        height: h,
        maximized: false,
        dock_side: rt.side.map(|s| s.as_str().to_string()),
        dock_hidden: rt.hidden,
    });
}
