//! Windows-specific desktop behavior: edge docking, tray popup positioning.
//!
//! On non-Windows builds everything compiles to inert stubs so the data
//! layer can be developed and tested on any host.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
}

impl Side {
    pub fn as_str(&self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
            Side::Top => "top",
        }
    }
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "left" => Some(Side::Left),
            "right" => Some(Side::Right),
            "top" => Some(Side::Top),
            _ => None,
        }
    }
}

#[cfg(windows)]
pub mod native;

#[cfg(windows)]
pub mod snap;

#[cfg(windows)]
pub mod popup_pos;

#[cfg(not(windows))]
pub mod snap {
    //! Inert stubs so the app compiles (and the data layer stays testable)
    //! on non-Windows hosts.
    use crate::settings::{Settings, WindowState};
    use std::sync::Arc;
    use tauri::WebviewWindow;

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
    pub struct SnapManager;

    impl SnapManager {
        pub fn send(&self, _msg: SnapMsg) {}
        pub fn spawn(
            _window: WebviewWindow,
            _get_settings: Arc<dyn Fn() -> Settings + Send + Sync>,
            _save_window: Arc<dyn Fn(WindowState) + Send + Sync>,
        ) -> Self {
            SnapManager
        }
    }
}

#[cfg(not(windows))]
pub mod popup_pos {
    pub fn compute_position(_tray: super::Rect, _w: i32, _h: i32) -> Option<(i32, i32)> {
        None
    }
}

/// Compiled-in availability flag surfaced to the frontend.
pub fn platform_supported() -> bool {
    cfg!(windows)
}
