//! Tauri IPC commands: read-model queries, settings application, export.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, Window};

use crate::engine::now_ms;
use crate::settings::{self, Settings};
use crate::zcode::aggregate::{
    bucketize, group_by_model, resolve_span, Agg, Bucket, ModelStat, SessionSummary, TrendRange,
};
use crate::zcode::pricing::{
    CostDetailDto, CostSummaryDto, OverrideDto, PricingManager, PricingRefreshResultDto,
    PricingTableDto,
};
use crate::zcode::usage::UsageRecord;

pub struct AppState {
    pub settings: Arc<RwLock<Settings>>,
    pub engine: crate::engine::Engine,
    pub pricing: Arc<PricingManager>,
    pub settings_dirty: AtomicBool,
    pub snap: OnceLock<crate::windows::snap::SnapManager>,
    pub hub: crate::providers::hub::ProviderHub,
    pub secrets: Arc<dyn crate::providers::secrets::SecretStorage>,
}

pub type SharedAppState = Arc<AppState>;

pub fn current_settings(state: &AppState) -> Settings {
    state.settings.read().unwrap().clone()
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRow {
    pub name: String,
    pub agg: Agg,
    /// Share of total tokens in range (0..1).
    pub share: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSwitch {
    pub ts_ms: i64,
    pub model: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveSession {
    pub session_id: String,
    pub project: Option<String>,
    pub session_total_tokens: u64,
    pub session_agg: Agg,
    pub tokens_last_5m: u64,
    pub tokens_per_min: f64,
    pub last_request_ms: Option<i64>,
    pub active_model: Option<String>,
    pub model_switches: Vec<ModelSwitch>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardDto {
    pub range_key: String,
    pub from_ms: i64,
    pub to_ms: i64,
    pub agg: Agg,
    pub models: Vec<ModelRow>,
    pub active_session: Option<ActiveSession>,
    /// true while numbers come from the persisted boot snapshot.
    pub restored: bool,
    pub data_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrendDto {
    pub range_key: String,
    pub from_ms: i64,
    pub to_ms: i64,
    pub buckets: Vec<Bucket>,
    pub restored: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetailDto {
    pub summary: SessionSummary,
    pub buckets: Vec<Bucket>,
    pub models: Vec<ModelStat>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDetailDto {
    pub name: String,
    pub today: Agg,
    pub last_7d: Agg,
    pub last_30d: Agg,
    pub all_time: Agg,
    pub avg_tokens_per_request: f64,
    pub hit_rate: Option<f64>,
    pub last_used_ms: Option<i64>,
    pub trend_30d: Vec<Bucket>,
    pub top_sessions: Vec<(String, u64)>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStatusDto {
    pub path: String,
    pub records_read: u64,
    pub lines_skipped: u64,
    pub offset: u64,
    pub watermark: i64,
    pub table: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnoseDto {
    pub root: Option<String>,
    pub root_source: String,
    pub jsonl_files: Vec<FileStatusDto>,
    pub sqlite_files: Vec<FileStatusDto>,
    pub untracked_jsonl: usize,
    pub untracked_sqlite: usize,
    pub notes: Vec<String>,
    pub record_count: u64,
    pub last_refresh_ms: Option<i64>,
    pub error: Option<String>,
    /// Last 3 raw records — for eyeballing against ZCode's own Usage page.
    pub recent_records: Vec<UsageRecord>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapDto {
    pub settings: Settings,
    pub version: String,
    pub config_dir: Option<String>,
    pub cache_dir: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn active_session_of(inner: &mut crate::engine::EngineInner) -> Option<ActiveSession> {
    let now = now_ms();
    let id = inner.store.active_session_id()?;
    let summary = inner
        .store
        .session_summaries()
        .iter()
        .find(|s| s.id == id)?
        .clone();
    let recent = inner.store.session_records(&id, now - 30 * 60_000);
    let last_5m: u64 = recent
        .iter()
        .filter(|r| r.ts_ms >= now - 5 * 60_000)
        .map(|r| {
            r.input_tokens + r.output_tokens + r.reasoning_tokens.unwrap_or(0)
                + r.cache_read_tokens.unwrap_or(0) + r.cache_write_tokens.unwrap_or(0)
        })
        .sum();

    // Model switch log: unique-model transitions in chronological order.
    let mut switches: Vec<ModelSwitch> = Vec::new();
    for r in &recent {
        if switches.last().map(|s| s.model != r.model).unwrap_or(true) {
            switches.push(ModelSwitch { ts_ms: r.ts_ms, model: r.model.clone() });
        }
        if switches.len() >= 50 {
            break;
        }
    }
    Some(ActiveSession {
        session_id: id,
        project: summary.project.clone(),
        session_total_tokens: summary.agg.total_tokens(),
        session_agg: summary.agg.clone(),
        tokens_last_5m: last_5m,
        tokens_per_min: (last_5m as f64) / 5.0,
        last_request_ms: summary.agg.last_ts_ms,
        active_model: recent.last().map(|r| r.model.clone()),
        model_switches: switches,
    })
}

fn model_rows(records: &[UsageRecord]) -> Vec<ModelRow> {
    let stats = group_by_model(records);
    let grand: u64 = stats.iter().map(|m| m.agg.total_tokens()).sum();
    stats
        .into_iter()
        .map(|m| {
            let total = m.agg.total_tokens();
            ModelRow {
                share: if grand > 0 { total as f64 / grand as f64 } else { 0.0 },
                name: m.name,
                agg: m.agg,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_bootstrap(app: AppHandle, state: State<'_, SharedAppState>) -> BootstrapDto {
    let settings = current_settings(&state);
    BootstrapDto {
        settings,
        version: app.package_info().version.to_string(),
        config_dir: app.path().app_config_dir().ok().map(|p| p.to_string_lossy().into_owned()),
        cache_dir: app.path().app_cache_dir().ok().map(|p| p.to_string_lossy().into_owned()),
    }
}

#[tauri::command]
pub fn get_dashboard(range_key: String, state: State<'_, SharedAppState>) -> DashboardDto {
    let range = TrendRange::from_key(&range_key).unwrap_or(TrendRange::TodayHourly);
    let now = now_ms();
    let mut inner = state.engine.inner.lock().unwrap();

    let (from, to, _) = resolve_span(range, now, inner.store.history_start_ms());
    let records = inner.store.range(from, to);
    let models = model_rows(records);
    let agg = records.iter().fold(Agg::default(), |mut a, r| {
        a.add(r);
        a
    });
    let active = if inner.store.is_empty() {
        None
    } else {
        active_session_of(&mut *inner)
    };    let restored = inner.store.is_empty() && inner.boot.is_some();

    let (agg, models) = if inner.store.is_empty() {
        if let Some(boot) = &inner.boot {
            (boot.today_agg.clone(), boot_rows(boot))
        } else {
            (agg, models)
        }
    } else {
        (agg, models)
    };

    DashboardDto {
        range_key: range.key().to_string(),
        from_ms: from,
        to_ms: to,
        agg,
        models,
        active_session: active,
        restored,
        data_error: inner.last_error.clone(),
    }
}

fn boot_rows(boot: &crate::engine::BootSnapshot) -> Vec<ModelRow> {
    let grand: u64 = boot.today_models.iter().map(|m| m.agg.total_tokens()).sum();
    boot.today_models
        .iter()
        .map(|m| {
            let total = m.agg.total_tokens();
            ModelRow {
                share: if grand > 0 { total as f64 / grand as f64 } else { 0.0 },
                name: m.name.clone(),
                agg: m.agg.clone(),
            }
        })
        .collect()
}

#[tauri::command]
pub fn get_trend(range_key: String, state: State<'_, SharedAppState>) -> TrendDto {
    let range = TrendRange::from_key(&range_key).unwrap_or(TrendRange::TodayHourly);
    let now = now_ms();
    let inner = state.engine.inner.lock().unwrap();
    let (from, to, n) = resolve_span(range, now, inner.store.history_start_ms());
    let restored = inner.store.is_empty() && inner.boot.is_some();
    let buckets = if inner.store.is_empty() {
        Vec::new()
    } else {
        bucketize(inner.store.range(from, to), from, to, n)
    };
    TrendDto {
        range_key: range.key().to_string(),
        from_ms: from,
        to_ms: to,
        buckets,
        restored,
    }
}

#[tauri::command]
pub fn get_sessions(state: State<'_, SharedAppState>) -> Vec<SessionSummary> {
    let mut inner = state.engine.inner.lock().unwrap();
    if inner.store.is_empty() {
        if let Some(boot) = &inner.boot {
            return boot.sessions.iter().take(500).cloned().collect();
        }
        return Vec::new();
    }
    inner.store.session_summaries().iter().take(500).cloned().collect()
}

#[tauri::command]
pub fn get_session_detail(session_id: String, state: State<'_, SharedAppState>) -> Option<SessionDetailDto> {
    let mut inner = state.engine.inner.lock().unwrap();
    let summary = inner
        .store
        .session_summaries()
        .iter()
        .find(|s| s.id == session_id)?
        .clone();
    let from = summary.agg.first_ts_ms?;
    let to = summary.agg.last_ts_ms?;
    let records = inner.store.range(from, to);
    let mine: Vec<UsageRecord> = records
        .iter()
        .filter(|r| r.session_id.as_deref() == Some(summary.id.as_str()))
        .cloned()
        .collect();
    let buckets = bucketize(&mine, from, to + 1, 48.min(mine.len().max(1)));
    let models = group_by_model(&mine);
    Some(SessionDetailDto { summary, buckets, models })
}

#[tauri::command]
pub fn get_model_detail(name: String, state: State<'_, SharedAppState>) -> Option<ModelDetailDto> {
    let now = now_ms();
    let inner = state.engine.inner.lock().unwrap();
    let mut all_agg = Agg::default();
    for r in inner.store.all() {
        if r.model == name {
            all_agg.add(r);
        }
    }
    if all_agg.requests == 0 {
        return None;
    }
    let fold = |from_ms: i64| -> Agg {
        inner
            .store
            .range(from_ms, now)
            .iter()
            .filter(|r| r.model == name)
            .fold(Agg::default(), |mut a, r| {
                a.add(r);
                a
            })
    };
    let today = fold(crate::zcode::aggregate::local_day_start_ms(now));
    let last_7d = fold(now - 7 * 24 * 3600_000);
    let last_30d = fold(now - 30 * 24 * 3600_000);
    let (t_from, t_to) = (now - 30 * 24 * 3600_000, now);
    let mine_30d: Vec<UsageRecord> = inner
        .store
        .range(t_from, t_to)
        .iter()
        .filter(|r| r.model == name)
        .cloned()
        .collect();
    let trend_30d = bucketize(&mine_30d, t_from, t_to, 30);
    let mut session_totals: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for r in &mine_30d {
        if let Some(sid) = &r.session_id {
            *session_totals.entry(sid.clone()).or_insert(0) +=
                r.input_tokens + r.output_tokens + r.reasoning_tokens.unwrap_or(0)
                    + r.cache_read_tokens.unwrap_or(0) + r.cache_write_tokens.unwrap_or(0);
        }
    }
    let mut top_sessions: Vec<(String, u64)> = session_totals.into_iter().collect();
    top_sessions.sort_by(|a, b| b.1.cmp(&a.1));
    top_sessions.truncate(10);

    let total_tokens = all_agg.total_tokens();
    Some(ModelDetailDto {
        name,
        today,
        last_7d,
        last_30d,
        all_time: all_agg.clone(),
        avg_tokens_per_request: if all_agg.requests > 0 {
            total_tokens as f64 / all_agg.requests as f64
        } else {
            0.0
        },
        hit_rate: all_agg.cache_hit_rate(),
        last_used_ms: all_agg.last_ts_ms,
        trend_30d,
        top_sessions,
    })
}

#[tauri::command]
pub fn get_alerts(state: State<'_, SharedAppState>) -> Vec<crate::alerts::AlertEvent> {
    let inner = state.engine.inner.lock().unwrap();
    inner.alert_log.clone()
}

#[tauri::command]
pub fn diagnose(state: State<'_, SharedAppState>) -> DiagnoseDto {
    let settings = current_settings(&state);
    let inner = state.engine.inner.lock().unwrap();
    let (root, root_source) = if settings.data_dir.is_some() {
        (settings.data_dir.clone(), "configured".to_string())
    } else if std::env::var("ZCODE_HOME").is_ok() {
        (std::env::var("ZCODE_HOME").ok(), "env:ZCODE_HOME".to_string())
    } else {
        (dirs::home_dir().map(|h| h.join(".zcode").to_string_lossy().into_owned()), "default:<home>/.zcode".to_string())
    };

    let jsonl_files = inner
        .jsonl
        .values()
        .map(|s| FileStatusDto {
            path: s.path.to_string_lossy().into_owned(),
            records_read: s.records_read,
            lines_skipped: s.lines_skipped,
            offset: s.offset,
            watermark: 0,
            table: None,
            last_error: s.last_error.clone(),
        })
        .collect();
    let sqlite_files = inner
        .sqlite
        .values()
        .map(|s| FileStatusDto {
            path: s.path.to_string_lossy().into_owned(),
            records_read: s.records_read,
            lines_skipped: 0,
            offset: 0,
            watermark: s.watermark,
            table: s.table.as_ref().map(|t| t.name.clone()),
            last_error: s.last_error.clone(),
        })
        .collect();
    let untracked_jsonl = inner
        .layout
        .as_ref()
        .map(|l| l.jsonl_files.len().saturating_sub(inner.jsonl.len()))
        .unwrap_or(0);
    let untracked_sqlite = inner
        .layout
        .as_ref()
        .map(|l| l.sqlite_files.len().saturating_sub(inner.sqlite.len()))
        .unwrap_or(0);
    let recent_records = inner.store.all().iter().rev().take(3).cloned().collect();
    DiagnoseDto {
        root,
        root_source,
        jsonl_files,
        sqlite_files,
        untracked_jsonl,
        untracked_sqlite,
        notes: inner.layout.as_ref().map(|l| l.notes.clone()).unwrap_or_default(),
        record_count: inner.store.len() as u64,
        last_refresh_ms: inner.last_refresh,
        error: inner.last_error.clone(),
        recent_records,
    }
}

/// Apply a full settings document. Side effects: autostart, always-on-top,
/// theme event, engine re-kick. Persisted atomically.
#[tauri::command]
pub fn set_settings(
    app: AppHandle,
    window: Window,
    state: State<'_, SharedAppState>,
    new_settings: Settings,
) -> Result<Settings, String> {
    {
        let mut guard = state.settings.write().unwrap();
        let aot_changed = guard.always_on_top != new_settings.always_on_top;
        let autostart_changed = true; // cheap to re-apply unconditionally
        let data_dir_changed = guard.data_dir != new_settings.data_dir;
        let paused_changed = guard.monitoring_paused != new_settings.monitoring_paused;
        let providers_changed = guard.providers != new_settings.providers
            || guard.launcher != new_settings.launcher
            || guard.quota_alerts != new_settings.quota_alerts;
        *guard = new_settings.clone();
        drop(guard);

        if aot_changed {
            let _ = window.set_always_on_top(new_settings.always_on_top);
        }
        if autostart_changed {
            apply_autostart(&app, new_settings.autostart);
        }
        if data_dir_changed || paused_changed {
            state.engine.kick();
        }
        if providers_changed {
            state.hub.kick();
        }
    }
    settings::save(&app, &new_settings);
    state.settings_dirty.store(false, Ordering::Relaxed);
    crate::tray::sync_checks(&app, &new_settings);
    let _ = app.emit("settings-changed", &new_settings);
    Ok(new_settings)
}

fn apply_autostart(app: &AppHandle, enable: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let autolaunch = app.autolaunch();
    let _ = if enable { autolaunch.enable() } else { autolaunch.disable() };
}

#[tauri::command]
pub fn refresh_now(state: State<'_, SharedAppState>) {
    state.engine.kick();
}

/// Hide the main window to tray (frontend title-bar button). Also re-evaluates
/// UI visibility so the engine suspends while nothing is visible.
#[tauri::command]
pub fn hide_main_window(app: AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }
    crate::visibility::update(&app);
}

// ---------------------------------------------------------------------------
// Official-API cost estimation commands
// ---------------------------------------------------------------------------

/// Model names seen in the data (boot-aware, like `get_active_models`).
fn all_model_names(inner: &crate::engine::EngineInner) -> Vec<String> {
    if inner.store.is_empty() {
        if let Some(boot) = &inner.boot {
            return boot.all_models.iter().map(|m| m.name.clone()).collect();
        }
        return Vec::new();
    }
    inner.store.all_model_names()
}

/// Cost summary over a range: same range strings/parsing as `get_dashboard`.
#[tauri::command]
pub fn cost_summary(range: String, state: State<'_, SharedAppState>) -> CostSummaryDto {
    let r = TrendRange::from_key(&range).unwrap_or(TrendRange::TodayHourly);
    let now = now_ms();
    let inner = state.engine.inner.lock().unwrap();
    let (from, to, _) = resolve_span(r, now, inner.store.history_start_ms());
    let records = inner.store.range(from, to);
    state.pricing.cost_summary(r.key(), records)
}

/// Per-line cost breakdown for one model over a range.
#[tauri::command]
pub fn cost_detail(range: String, model: String, state: State<'_, SharedAppState>) -> CostDetailDto {
    let r = TrendRange::from_key(&range).unwrap_or(TrendRange::TodayHourly);
    let now = now_ms();
    let inner = state.engine.inner.lock().unwrap();
    let (from, to, _) = resolve_span(r, now, inner.store.history_start_ms());
    let records = inner.store.range(from, to);
    state.pricing.cost_detail(&model, records)
}

/// Full price table with current effective prices (promo + overrides applied).
#[tauri::command]
pub fn pricing_table(state: State<'_, SharedAppState>) -> PricingTableDto {
    let unknown = {
        let inner = state.engine.inner.lock().unwrap();
        state.pricing.unknown_models(&all_model_names(&inner))
    };
    let url = current_settings(&state).pricing_remote_url.clone();
    state.pricing.build_table_dto(unknown, url)
}

/// Trigger a network refresh (remote price table when configured + FX rate).
#[tauri::command]
pub async fn pricing_refresh(
    state: State<'_, SharedAppState>,
) -> Result<PricingRefreshResultDto, String> {
    let pm = state.pricing.clone();
    let url = current_settings(&state).pricing_remote_url.clone();
    Ok(tauri::async_runtime::spawn_blocking(move || pm.refresh(url.as_deref()))
        .await
        .unwrap_or_else(|e| crate::zcode::pricing::PricingRefreshResultDto {
            ok: false,
            fx_ok: false,
            error: Some(format!("refresh task failed: {e}")),
            refreshed_at: crate::zcode::pricing::now_iso(),
        }))
}

/// Set or clear a flat price override for one model (persisted), then return
/// the latest table.
#[tauri::command]
pub fn pricing_override(
    state: State<'_, SharedAppState>,
    model: String,
    o: Option<OverrideDto>,
) -> PricingTableDto {
    state.pricing.set_override(&model, o);
    let unknown = {
        let inner = state.engine.inner.lock().unwrap();
        state.pricing.unknown_models(&all_model_names(&inner))
    };
    let url = current_settings(&state).pricing_remote_url.clone();
    state.pricing.build_table_dto(unknown, url)
}

// ---------------------------------------------------------------------------
// Window-behavior bridges (docking, popup, lifecycle)
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn dock_hover(state: State<'_, SharedAppState>, inside: bool) {
    if let Some(snap) = state.snap.get() {
        snap.send(crate::windows::snap::SnapMsg::Hover(inside));
    }
}

#[tauri::command]
pub fn dock_interact(state: State<'_, SharedAppState>, active: bool) {
    if let Some(snap) = state.snap.get() {
        snap.send(crate::windows::snap::SnapMsg::Interact(active));
    }
}

#[tauri::command]
pub fn popup_close(app: AppHandle) {
    crate::popup::hide(&app);
}

#[tauri::command]
pub fn quit_app(app: AppHandle, state: State<'_, SharedAppState>) {
    state.engine.save_snapshot();
    let s = current_settings(&state);
    settings::save(&app, &s);
    app.exit(0);
}

// ---------------------------------------------------------------------------
// Multi-provider quota dashboard
// ---------------------------------------------------------------------------

/// All provider snapshots (cached — instant, never triggers network).
#[tauri::command]
pub fn providers_overview(state: State<'_, SharedAppState>) -> Vec<crate::providers::ProviderSnapshot> {
    state.hub.overview()
}

/// Force a refresh (one provider id, or all when omitted/null).
#[tauri::command]
pub fn providers_refresh(state: State<'_, SharedAppState>, provider: Option<String>) {
    state.hub.refresh_now(provider);
}

#[tauri::command]
pub fn quota_alerts_list(state: State<'_, SharedAppState>) -> Vec<crate::providers::quota_alerts::AlertEvent> {
    state.hub.quota_alert_log()
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPointDto {
    pub ts_ms: i64,
    pub used_percent: Option<f64>,
    pub used: Option<f64>,
    pub remaining: Option<f64>,
}

/// Quota-window history for the trend view. `range`: "today" | "7d" | "30d".
#[tauri::command]
pub fn providers_history(
    state: State<'_, SharedAppState>,
    provider: String,
    window: String,
    range: String,
) -> Vec<HistoryPointDto> {
    let now = now_ms();
    let from = match range.as_str() {
        "today" => crate::zcode::aggregate::local_day_start_ms(now),
        "7d" => now - 7 * 24 * 3600_000,
        _ => now - 30 * 24 * 3600_000,
    };
    state
        .hub
        .history_for(&provider, &window, from, now)
        .into_iter()
        .map(|p| HistoryPointDto { ts_ms: p.ts_ms, used_percent: p.used_percent, used: p.used, remaining: p.remaining })
        .collect()
}

/// Daily consumption deltas for one window over N days.
#[tauri::command]
pub fn providers_consumption(
    state: State<'_, SharedAppState>,
    provider: String,
    window: String,
    days: u32,
) -> Vec<(i64, f64)> {
    state.hub.consumption(&provider, &window, days.clamp(1, 90), now_ms())
}

// -- ZCode launcher ----------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LauncherActionDto {
    /// "Focused" | "Started" | "NotFound" | "Failed" | ""
    pub result: String,
    pub snapshot: crate::providers::ProviderSnapshot,
}

#[tauri::command]
pub fn zcode_status(state: State<'_, SharedAppState>) -> crate::providers::ProviderSnapshot {
    let settings = current_settings(&state);
    let (_, snap) = state.hub.launcher_action("status", &settings);
    snap
}

#[tauri::command]
pub fn zcode_launch(state: State<'_, SharedAppState>) -> LauncherActionDto {
    let settings = current_settings(&state);
    let (result, snapshot) = state.hub.launcher_action("launch", &settings);
    LauncherActionDto { result, snapshot }
}

#[tauri::command]
pub fn zcode_reveal(state: State<'_, SharedAppState>) -> LauncherActionDto {
    let settings = current_settings(&state);
    let (result, snapshot) = state.hub.launcher_action("reveal", &settings);
    LauncherActionDto { result, snapshot }
}

// -- Volcengine credentials (OS keyring; values never come back out) ---------

#[tauri::command]
pub fn volcengine_credentials_status(
    state: State<'_, SharedAppState>,
) -> crate::providers::hub::CredentialsStatusDto {
    crate::providers::hub::credentials_status(&state.secrets)
}

#[tauri::command]
pub fn volcengine_credentials_set(
    state: State<'_, SharedAppState>,
    ak: String,
    sk: String,
) -> Result<(), String> {
    crate::providers::hub::set_volcengine_credentials(&state.secrets, &ak, &sk)?;
    state.hub.refresh_now(Some("volcengine".into()));
    Ok(())
}

#[tauri::command]
pub fn volcengine_credentials_clear(state: State<'_, SharedAppState>) -> Result<(), String> {
    crate::providers::hub::clear_volcengine_credentials(&state.secrets)
}

#[tauri::command]
pub fn volcengine_test(state: State<'_, SharedAppState>) -> Result<String, String> {
    let region = current_settings(&state).providers.volcengine_region.clone();
    crate::providers::hub::test_volcengine(&state.secrets, &region)
}

/// All model names seen in the data (for the rate editor's model picker).
#[tauri::command]
pub fn get_active_models(state: State<'_, SharedAppState>) -> Vec<String> {
    let inner = state.engine.inner.lock().unwrap();
    if inner.store.is_empty() {
        if let Some(boot) = &inner.boot {
            return boot.all_models.iter().map(|m| m.name.clone()).collect();
        }
        return Vec::new();
    }
    inner.store.all_model_names()
}

#[tauri::command]
pub async fn export_data(
    app: AppHandle,
    state: State<'_, SharedAppState>,
    scope: String,
    format: String,
    range_key: String,
    suggested_name: String,
) -> Result<String, String> {
    let settings = current_settings(&state);
    let data = crate::export::build_export(&state.engine, &settings, &scope, &range_key)
        .map_err(|e| e.to_string())?;
    let (content, ext, filter_name) = crate::export::render(&data, &format)?;
    let default_name = if suggested_name.is_empty() {
        format!("zcode-usage-{scope}-{}.{}", now_ms(), ext)
    } else {
        format!("{}.{}", suggested_name.trim_end_matches(&format!(".{ext}")), ext)
    };
    tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_dialog::DialogExt;
        let file = app
            .dialog()
            .file()
            .add_filter(&filter_name, &[&ext])
            .set_file_name(&default_name)
            .blocking_save_file();
        let Some(path) = file else {
            return Err("cancelled".to_string());
        };
        let path = path.into_path().map_err(|e| e.to_string())?;
        std::fs::write(&path, content).map_err(|e| e.to_string())?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}
