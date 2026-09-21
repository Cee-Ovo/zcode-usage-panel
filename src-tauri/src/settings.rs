//! User settings, persisted to `<appConfigDir>/settings.json`.
//!
//! Compatibility: every struct derives `Default` and deserializes with
//! `#[serde(default)]` on the container, so a v1.1 `settings.json` (without
//! provider/launcher/quota-alert sections) loads losslessly — that IS the
//! settings migration. Unknown fields are kept out by serde's default
//! behavior for known structs, and no key is ever removed on upgrade.

use serde::{Deserialize, Serialize};

pub const DEFAULT_SNAP_SIDES: SnapSides = SnapSides {
    left: true,
    right: true,
    top: true,
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SnapSides {
    pub left: bool,
    pub right: bool,
    pub top: bool,
}

impl Default for SnapSides {
    fn default() -> Self {
        DEFAULT_SNAP_SIDES
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SnapSettings {
    pub enabled: bool,
    pub auto_hide: bool,
    /// Logical pixels — multiplied by the monitor scale factor at runtime.
    pub threshold_px: f64,
    pub hide_delay_ms: u64,
    pub anim_ms: u64,
    pub sides: SnapSides,
}

impl Default for SnapSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_hide: true,
            threshold_px: 24.0,
            hide_delay_ms: 600,
            anim_ms: 200,
            sides: SnapSides::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
    /// "left" | "right" | "top" when docked.
    pub dock_side: Option<String>,
    pub dock_hidden: bool,
}

/// Per-provider switches + cadences. Secrets are NOT here — they live in the
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderSettings {
    pub codex_enabled: bool,
    /// Override for the Codex CLI data dir (`~/.codex` by default).
    pub codex_home: Option<String>,
    pub codex_refresh_ms: u64,
    pub dsh_enabled: bool,
    /// Override for the DeepSeek Harness data dir (`~/.dsh` by default).
    pub dsh_home: Option<String>,
    pub dsh_refresh_ms: u64,
    pub claude_code_enabled: bool,
    /// Override for the Claude Code config dir (`~/.claude` by default,
    /// `$CLAUDE_CONFIG_DIR` honored).
    pub claude_code_home: Option<String>,
    pub claude_code_refresh_ms: u64,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            codex_enabled: true,
            codex_home: None,
            codex_refresh_ms: crate::providers::cadence::CODEX_MS,
            dsh_enabled: true,
            dsh_home: None,
            dsh_refresh_ms: crate::providers::cadence::DSH_MS,
            claude_code_enabled: true,
            claude_code_home: None,
            claude_code_refresh_ms: crate::providers::cadence::CLAUDE_CODE_MS,
        }
    }
}

/// ZCode quick-launch configuration.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct LauncherSettings {
    pub enabled: bool,
    /// User-specified exe path; `None` ⇒ auto-detect.
    pub exe_path: Option<String>,
    /// Start ZCode together with this app.
    pub autostart: bool,
}

impl Default for LauncherSettings {
    fn default() -> Self {
        Self { enabled: true, exe_path: None, autostart: false }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// `None` ⇒ auto-detect (`ZCODE_HOME`, then `<home>/.zcode`).
    pub data_dir: Option<String>,
    pub refresh_debounce_ms: u64,
    /// "today" | "60m" | "24h" | "7d" | "30d" | "all"
    pub default_range: String,
    /// "light" | "dark" | "system" (light is the product default)
    pub theme: String,
    pub always_on_top: bool,
    pub monitoring_paused: bool,
    /// true ⇒ closing the window minimizes to tray instead of quitting.
    pub close_to_tray: bool,
    /// Launch at Windows sign-in (applied via the autostart plugin).
    pub autostart: bool,
    /// Optional remote price-table URL (same schema as prices_builtin.json).
    /// Pulled on a background thread when set.
    pub pricing_remote_url: Option<String>,
    pub snap: SnapSettings,
    pub window: WindowState,
    /// Local data sources (added v1.2; defaults keep old files valid).
    pub providers: ProviderSettings,
    pub launcher: LauncherSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            data_dir: None,
            refresh_debounce_ms: 600,
            default_range: "today".into(),
            theme: "light".into(),
            always_on_top: false,
            monitoring_paused: false,
            close_to_tray: true,
            autostart: false,
            pricing_remote_url: None,
            snap: SnapSettings::default(),
            window: WindowState::default(),
            providers: ProviderSettings::default(),
            launcher: LauncherSettings::default(),
        }
    }
}

pub fn load(app: &tauri::AppHandle) -> Settings {
    use tauri::Manager;
    let dir = app.path().app_config_dir();
    let Ok(dir) = dir else { return Settings::default() };
    let path = dir.join("settings.json");
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("[zup] settings parse failed ({e}); using defaults");
            Settings::default()
        }),
        Err(_) => Settings::default(),
    }
}

pub fn save(app: &tauri::AppHandle, settings: &Settings) {
    use tauri::Manager;
    let Ok(dir) = app.path().app_config_dir() else { return };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("settings.json");
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        // Write-to-temp + rename keeps the file atomic even if the app dies
        // mid-write; settings corruption would lose dock/window state.
        let tmp = dir.join("settings.json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}
