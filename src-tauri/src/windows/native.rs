//! Thin Win32 wrappers. All coordinates are physical pixels.
//!
//! The only interop point with Tauri is the raw HWND value (`usize`), which
//! sidesteps windows-crate version drift between our dependency and
//! Tauri's.

#![allow(non_snake_case)]

use super::Rect;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetCursorPos, SetWindowPos, GA_ROOT, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_SHOWWINDOW,
};

fn hwnd_of(raw: usize) -> HWND {
    HWND(raw as *mut core::ffi::c_void)
}

/// (monitor rect, work-area rect) of the monitor nearest to `window_hwnd`.
pub fn work_area_of_window(window_hwnd: usize) -> Option<(Rect, Rect)> {
    let monitor = unsafe { MonitorFromWindow(hwnd_of(window_hwnd), MONITOR_DEFAULTTONEAREST) };
    monitor_info(monitor)
}

/// (monitor rect, work-area rect) of the monitor containing a point.
pub fn work_area_of_point(x: i32, y: i32) -> Option<(Rect, Rect)> {
    let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    monitor_info(monitor)
}

fn monitor_info(monitor: windows::Win32::Graphics::Gdi::HMONITOR) -> Option<(Rect, Rect)> {
    if monitor.is_invalid() {
        return None;
    }
    let mut info = MONITORINFO {
        cbSize: core::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let ok = unsafe { GetMonitorInfoW(monitor, &mut info as *mut MONITORINFO) };
    if !ok.as_bool() {
        return None;
    }
    Some((info.rcMonitor.into(), info.rcWork.into()))
}

impl From<RECT> for Rect {
    fn from(r: RECT) -> Self {
        Rect {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        }
    }
}

/// Current global cursor position (physical px).
pub fn cursor_position() -> Option<(i32, i32)> {
    let mut pt = POINT::default();
    let ok = unsafe { GetCursorPos(&mut pt as *mut POINT) };
    ok.is_ok().then_some((pt.x, pt.y))
}

/// Is the point over the given top-level window?
pub fn point_is_over_window(x: i32, y: i32, window_hwnd: usize) -> bool {
    let hovered = unsafe { windows::Win32::UI::WindowsAndMessaging::WindowFromPoint(POINT { x, y }) };
    if hovered.is_invalid() {
        return false;
    }
    let root = unsafe { GetAncestor(hwnd_of(window_hwnd), GA_ROOT) };
    let hovered_root = unsafe { GetAncestor(hovered, GA_ROOT) };
    (hovered_root.0 as usize) == (root.0 as usize)
}

/// Show a window WITHOUT stealing activation (for the tray popup).
pub fn show_no_activate(window_hwnd: usize) {
    unsafe {
        let _ = SetWindowPos(
            hwnd_of(window_hwnd),
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW | SWP_NOACTIVATE,
        );
    }
}
