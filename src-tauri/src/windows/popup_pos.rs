//! Tray popup positioning (Windows).
//!
//! Given the tray icon rect (physical px, from Tauri's TrayIconEvent) and the
//! popup size, place the popup near the icon while:
//! - respecting the work area of the monitor that contains the icon,
//! - adapting to taskbar position (bottom / top / left / right — inferred
//!   from where the icon rect sits relative to the monitor and work rects),
//! - clamping so the popup never crosses monitor edges.

use super::native;
use super::Rect;

pub fn compute_position(
    tray_rect: Rect,
    popup_w: i32,
    popup_h: i32,
) -> Option<(i32, i32)> {
    let cx = (tray_rect.left + tray_rect.right) / 2;
    let cy = (tray_rect.top + tray_rect.bottom) / 2;
    let (monitor, work) = native::work_area_of_point(cx, cy)?;

    // Which side of the monitor does the taskbar occupy?
    let gap = 8; // breathing room from the taskbar
    let margin = 8; // and from work-area edges

    // Distance from the tray icon to each work-area edge tells the story:
    // a bottom taskbar puts the icon near monitor.bottom (below work.bottom).
    let near_bottom = tray_rect.bottom > work.bottom - 4
        || (monitor.bottom - work.bottom) >= (work.top - monitor.top)
            && tray_rect.top > work.bottom - 4;
    let near_top = tray_rect.top < work.top + 4;
    let near_left = tray_rect.left < work.left + 4 && !near_top && !near_bottom;
    let near_right = tray_rect.right > work.right - 4 && !near_top && !near_bottom;

    let (mut x, mut y) = if near_top {
        // taskbar at top → popup below the icon
        (
            cx - popup_w / 2,
            work.top + gap,
        )
    } else if near_left {
        (
            work.left + gap,
            cy - popup_h / 2,
        )
    } else if near_right {
        (
            work.right - popup_w - gap,
            cy - popup_h / 2,
        )
    } else {
        // default: taskbar at bottom → popup above the icon
        (
            cx - popup_w / 2,
            work.bottom - popup_h - gap,
        )
    };

    // Clamp inside the work area.
    x = x.clamp(work.left + margin, (work.right - popup_w - margin).max(work.left + margin));
    y = y.clamp(work.top + margin, (work.bottom - popup_h - margin).max(work.top + margin));
    Some((x, y))
}
