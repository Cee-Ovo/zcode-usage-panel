//! Local data-source framework: unified snapshot model + adapters.
//!
//! Every local source (ZCode engine aggregates, OpenAI Codex session logs,
//! DSH logs, Claude Code transcripts) is a `Provider` that produces a
//! `ProviderSnapshot`. Providers are isolated: a failing provider degrades to
//! an error snapshot and never takes the app down. All parsing logic lives in
//! pure, injectable functions so it can be unit-tested without a running
//! Tauri app.
//!
//! Layout:
//! - `mod.rs`     — unified data model (snapshots, local usage, launcher)
//! - `hub.rs`     — scheduler thread: per-provider cadence, retry/backoff,
//!                  stale detection, event emission, session queries
//! - `codex.rs` / `dsh.rs` / `claude_code.rs` — local-log adapters
//! - `local_usage.rs` / `session_index.rs` — shared aggregation helpers
//! - `zlauncher.rs` — ZCode executable detection + launch state machine

pub mod claude_code;
pub mod codex;
pub mod dsh;
pub mod hub;
pub mod local_usage;
pub mod session_index;
pub mod zlauncher;

use serde::{Deserialize, Serialize};

/// Re-export the shared epoch-ms clock for examples/tests.
pub use crate::engine::now_ms;

pub const PROVIDER_ZCODE: &str = "zcode";
pub const PROVIDER_CODEX: &str = "codex";
pub const PROVIDER_DSH: &str = "dsh";
pub const PROVIDER_CLAUDE_CODE: &str = "claude-code";

/// Lifecycle status of a provider's data (not of the service itself).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    /// Fresh data available.
    Ok,
    /// Adapter enabled but no data directory found yet.
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

/// Per-model local harness token usage.
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
pub struct LocalUsageRange {
    pub key: String,
    pub breakdown: TokenBreakdown,
    pub sessions: u64,
    pub models: Vec<ModelUsageRow>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct LocalUsage {
    pub today: TokenBreakdown,
    pub last_7d: TokenBreakdown,
    pub all_time: TokenBreakdown,
    pub sessions: u64,
    pub models: Vec<ModelUsageRow>,
    pub ranges: Vec<LocalUsageRange>,
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
        Self {
            state: "not_installed".into(),
            exe_path: None,
            version: None,
            detected_via: None,
        }
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
    pub local_usage: Option<LocalUsage>,
    pub launcher: Option<LauncherStatus>,
    /// Human-readable data provenance, e.g. "Codex 本地 session 文件".
    pub source: String,
    /// Optional official doc/pricing URL shown in tooltips.
    pub source_url: Option<String>,
    pub notes: Vec<String>,
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

    /// Overall health: ok when the source has fresh data; degraded
    /// (stale/error) entries are surfaced in status pills.
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
    pub const DSH_MS: u64 = 60_000;
    pub const CLAUDE_CODE_MS: u64 = 60_000;
}

#[cfg(test)]
mod tests {
    use super::*;

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