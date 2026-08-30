//! AI service provider framework: unified quota model + adapters.
//!
//! Every external service (ZCode local usage, OpenAI Codex, Antigravity,
//! Volcengine token packages) is a `Provider` that produces a
//! `ProviderSnapshot`. Providers are isolated: a failing provider degrades to
//! an error snapshot and never takes the app down. All parsing/signing logic
//! lives in pure, injectable functions so it can be unit-tested without
//! network or a running Tauri app.
//!
//! Layout:
//! - `mod.rs`     — unified data model (snapshots, quota windows, packages)
//! - `hub.rs`     — scheduler thread: per-provider cadence, retry/backoff,
//!                  stale detection, event emission
//! - `history.rs` — SQLite snapshot persistence, retention, forecast
//! - `quota_alerts.rs` — threshold/expiry/stale alerts with cooldowns
//! - `secrets.rs` — credential storage (OS keyring, never plaintext files)
//! - `codex.rs` / `antigravity.rs` / `volcengine.rs` — adapters
//! - `zlauncher.rs` — ZCode executable detection + launch state machine

pub mod antigravity;
pub mod codex;
pub mod history;
pub mod hub;
pub mod quota_alerts;
pub mod secrets;
pub mod volcengine;
pub mod zlauncher;

use serde::{Deserialize, Serialize};

/// Re-export the shared epoch-ms clock for examples/tests.
pub use crate::engine::now_ms;

pub const PROVIDER_ZCODE: &str = "zcode";
pub const PROVIDER_CODEX: &str = "codex";
pub const PROVIDER_ANTIGRAVITY: &str = "antigravity";
pub const PROVIDER_VOLCENGINE: &str = "volcengine";

/// Lifecycle status of a provider's data (not of the service itself).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    /// Fresh data available.
    Ok,
    /// Adapter enabled but no credentials / setup done yet.
    NotConfigured,
    /// Underlying client not detected on this machine.
    NotInstalled,
    /// Provider disabled by the user.
    Disabled,
    /// Data present but older than the staleness threshold.
    Stale,
    /// Last poll failed; last known data (if any) is kept for display.
    Error,
}

impl ProviderStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderStatus::Ok => "ok",
            ProviderStatus::NotConfigured => "not_configured",
            ProviderStatus::NotInstalled => "not_installed",
            ProviderStatus::Disabled => "disabled",
            ProviderStatus::Stale => "stale",
            ProviderStatus::Error => "error",
        }
    }
}

/// Linear-regression depletion forecast. Always surfaced as a *prediction*,
/// never mixed with official numbers.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Forecast {
    /// Predicted time remaining until the window is exhausted (ms).
    pub eta_ms: i64,
    /// Consumption rate used for the estimate, quota units per day.
    pub rate_per_day: f64,
    /// Number of history samples behind the regression.
    pub samples: usize,
    /// "low" | "medium" | "high" — span/sample based confidence label.
    pub confidence: String,
}

impl Default for Forecast {
    fn default() -> Self {
        Self { eta_ms: 0, rate_per_day: 0.0, samples: 0, confidence: "low".into() }
    }
}

/// One usage window (Codex 5h, weekly, monthly plans, a token package…).
/// Fields the source does not provide stay `None` — never invented.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct QuotaWindow {
    /// Stable key, e.g. "5h" | "weekly" | "monthly" | "package:123".
    pub key: String,
    /// Display label.
    pub label: String,
    pub used_percent: Option<f64>,
    pub total_quota: Option<f64>,
    pub used_quota: Option<f64>,
    pub remaining_quota: Option<f64>,
    /// "tokens" | "credits" | "requests" | "usd" … display-only.
    pub unit: Option<String>,
    pub reset_at_ms: Option<i64>,
    pub window_minutes: Option<u64>,
    pub forecast: Option<Forecast>,
}

impl Default for QuotaWindow {
    fn default() -> Self {
        Self {
            key: String::new(),
            label: String::new(),
            used_percent: None,
            total_quota: None,
            used_quota: None,
            remaining_quota: None,
            unit: None,
            reset_at_ms: None,
            window_minutes: None,
            forecast: None,
        }
    }
}

impl QuotaWindow {
    /// Effective usage percent, preferring the official value and falling
    /// back to a computed one only when both totals exist.
    pub fn effective_percent(&self) -> Option<f64> {
        self.used_percent.or_else(|| match (self.used_quota, self.total_quota) {
            (Some(u), Some(t)) if t > 0.0 => Some((u / t) * 100.0),
            _ => None,
        })
    }
}

/// One purchased token package (Volcengine-style).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct PackageInfo {
    pub instance_no: String,
    pub name: String,
    pub configuration: String,
    pub product: String,
    pub total_amount: f64,
    pub available_amount: f64,
    /// total − available, computed (the billing API has no explicit field).
    pub used_amount: f64,
    /// "千Token" → 1000 etc. Display keeps the raw unit label.
    pub unit: String,
    pub unit_multiplier: f64,
    pub effective_ms: Option<i64>,
    pub expiry_ms: Option<i64>,
    pub status: String,
    pub usage_percent: Option<f64>,
}

impl Default for PackageInfo {
    fn default() -> Self {
        Self {
            instance_no: String::new(),
            name: String::new(),
            configuration: String::new(),
            product: String::new(),
            total_amount: 0.0,
            available_amount: 0.0,
            used_amount: 0.0,
            unit: String::new(),
            unit_multiplier: 1.0,
            effective_ms: None,
            expiry_ms: None,
            status: String::new(),
            usage_percent: None,
        }
    }
}

/// Per-model local harness token usage (Codex local stats are kept strictly
/// separate from official plan quota — never merged into one metric).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct TokenBreakdown {
    pub requests: u64,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelUsageRow {
    pub model: String,
    pub breakdown: TokenBreakdown,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct LocalUsage {
    pub today: TokenBreakdown,
    pub last_7d: TokenBreakdown,
    pub all_time: TokenBreakdown,
    pub sessions: u64,
    pub models: Vec<ModelUsageRow>,
}

/// Launcher status for the ZCode quick-start card/tray entries.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct LauncherStatus {
    /// "not_installed" | "not_running" | "starting" | "running"
    pub state: String,
    /// Detected (or configured) executable path.
    pub exe_path: Option<String>,
    /// File-product version of the exe, when readable.
    pub version: Option<String>,
    /// How the path was found ("registry" | "common_path" | "configured" | …).
    pub detected_via: Option<String>,
}

impl Default for LauncherStatus {
    fn default() -> Self {
        Self { state: "not_installed".into(), exe_path: None, version: None, detected_via: None }
    }
}

/// The unified result one adapter produces per poll.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSnapshot {
    pub provider: String,
    pub status: ProviderStatus,
    pub account: Option<String>,
    pub plan_name: Option<String>,
    pub windows: Vec<QuotaWindow>,
    pub packages: Vec<PackageInfo>,
    pub local_usage: Option<LocalUsage>,
    pub launcher: Option<LauncherStatus>,
    /// Human-readable data provenance, e.g. "Codex 本地 session 文件".
    pub source: String,
    /// Optional official doc/pricing URL shown in tooltips.
    pub source_url: Option<String>,
    pub notes: Vec<String>,
    /// Sanitized error (never contains credentials — see secrets.rs).
    pub error: Option<String>,
    pub updated_at_ms: i64,
    /// Poll bookkeeping (epoch ms) used by the hub.
    pub next_poll_ms: i64,
}

impl ProviderSnapshot {
    pub fn empty(provider: &str, status: ProviderStatus, now_ms: i64) -> Self {
        Self {
            provider: provider.to_string(),
            status,
            account: None,
            plan_name: None,
            windows: Vec::new(),
            packages: Vec::new(),
            local_usage: None,
            launcher: None,
            source: String::new(),
            source_url: None,
            notes: Vec::new(),
            error: None,
            updated_at_ms: now_ms,
            next_poll_ms: 0,
        }
    }

    /// Overall health for the popup footer: ok when every enabled provider
    /// has fresh data; degraded (stale/error) entries are listed.
    pub fn health(&self) -> &'static str {
        match self.status {
            ProviderStatus::Ok | ProviderStatus::Disabled | ProviderStatus::NotConfigured => "ok",
            _ => "degraded",
        }
    }
}

/// Default poll cadences (ms). All are user-tunable in settings.
pub mod cadence {
    pub const CODEX_MS: u64 = 60_000;
    pub const ANTIGRAVITY_MS: u64 = 120_000;
    pub const VOLCENGINE_MS: u64 = 1_800_000;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_percent_prefers_official_then_computes() {
        let mut w = QuotaWindow { key: "5h".into(), ..Default::default() };
        w.used_percent = Some(72.0);
        assert!((w.effective_percent().unwrap() - 72.0).abs() < 1e-9);
        w.used_percent = None;
        w.used_quota = Some(36.0);
        w.total_quota = Some(50.0);
        assert!((w.effective_percent().unwrap() - 72.0).abs() < 1e-9);
        w.total_quota = Some(0.0);
        assert!(w.effective_percent().is_none());
    }

    #[test]
    fn health_degrades_only_on_real_trouble() {
        let ok = ProviderSnapshot::empty("x", ProviderStatus::Ok, 0);
        let nc = ProviderSnapshot::empty("x", ProviderStatus::NotConfigured, 0);
        let err = ProviderSnapshot::empty("x", ProviderStatus::Error, 0);
        assert_eq!(ok.health(), "ok");
        assert_eq!(nc.health(), "ok");
        assert_eq!(err.health(), "degraded");
    }
}
