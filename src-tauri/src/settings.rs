//! User settings, persisted to `<appConfigDir>/settings.json`.
//!
//! Compatibility: every struct derives `Default` and deserializes with
//! `#[serde(default)]` on the container, so a v1.1 `settings.json` (without
//! provider/launcher/quota-alert sections) loads losslessly — that IS the
//! settings migration. Unknown fields are kept out by serde's default
//! behavior for known structs, and no key is ever removed on upgrade.

use serde::{Deserialize, Serialize};

use crate::providers::quota_alerts::QuotaAlertRules;

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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AlertRuleState {
    pub enabled: bool,
    /// Token surge: 10-minute total exceeds `multiplier` × the trailing
    /// hour's 10-minute average (and at least `min_tokens`).
    pub spike_multiplier: f64,
    pub spike_min_tokens: u64,
    /// Single-session total threshold (tokens).
    pub session_total_tokens: u64,
    /// Cache hit drop: recent hit rate falls this far below the trailing
    /// baseline (0.25 = 25 points).
    pub cache_hit_drop: f64,
    pub cache_min_requests: u64,
    /// Model burst: N requests for one model within 5 minutes.
    pub model_burst_per_5m: u64,
    /// Data staleness: no new records for N minutes.
    pub staleness_minutes: u64,
}

impl Default for AlertRuleState {
    fn default() -> Self {
        Self {
            enabled: true,
            spike_multiplier: 8.0,
            spike_min_tokens: 2_000_000,
            session_total_tokens: 50_000_000,
            cache_hit_drop: 0.25,
            cache_min_requests: 20,
            model_burst_per_5m: 400,
            staleness_minutes: 120,
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
/// OS keyring (see providers/secrets.rs).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderSettings {
    pub codex_enabled: bool,
    /// Override for the Codex CLI data dir (`~/.codex` by default).
    pub codex_home: Option<String>,
    pub codex_refresh_ms: u64,
    pub antigravity_enabled: bool,
    pub antigravity_refresh_ms: u64,
    pub volcengine_enabled: bool,
    pub volcengine_refresh_ms: u64,
    pub volcengine_region: String,
    /// Optional substring filter for package names (empty = show all).
    pub volcengine_filter: String,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            codex_enabled: true,
            codex_home: None,
            codex_refresh_ms: crate::providers::cadence::CODEX_MS,
            antigravity_enabled: true,
            antigravity_refresh_ms: crate::providers::cadence::ANTIGRAVITY_MS,
            volcengine_enabled: true,
            volcengine_refresh_ms: crate::providers::cadence::VOLCENGINE_MS,
            volcengine_region: crate::providers::volcengine::DEFAULT_REGION.into(),
            volcengine_filter: String::new(),
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
    pub notifications: AlertRuleState,
    pub window: WindowState,
    /// Multi-provider quota dashboard (added v1.2; defaults keep old files valid).
    pub providers: ProviderSettings,
    pub launcher: LauncherSettings,
    pub quota_alerts: QuotaAlertRules,
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
            notifications: AlertRuleState::default(),
            window: WindowState::default(),
            providers: ProviderSettings::default(),
            launcher: LauncherSettings::default(),
            quota_alerts: QuotaAlertRules::default(),
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
