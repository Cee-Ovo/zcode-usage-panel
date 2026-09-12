//! Tauri IPC commands: read-model queries, settings application, export.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, Window};

use crate::engine::now_ms;
use crate::settings::{self, Settings};
use crate::zcode::aggregate::{
    bucketize, compute_speed_stats, group_by_model, resolve_span, speed_by_model, Agg, Bucket,
    ModelStat, SessionSummary, SpeedStats, TrendRange,
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
    /// Response-speed stats for this model in range (default = no samples,
    /// e.g. the boot-snapshot path has no raw records).
    pub speed: SpeedStats,
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
pub struct SpeedWindowStats {
    /// Trailing window length in ms (24h / 7d), wall-clock relative to now.
    pub window_ms: i64,
    pub speed_tps: Option<f64>,
    pub speed_samples: u64,
    pub completed_requests: u64,
    /// Same convention as SpeedStats: window covers the whole request when
    /// the source records no TTFT (Codex rollouts).
    #[serde(default)]
    pub speed_approximate: bool,
}

/// Fixed trailing windows behind the speed card's trend line, shortest first.
pub(crate) const SPEED_TREND_WINDOWS_MS: &[i64] = &[86_400_000, 7 * 86_400_000];

/// Speed stats over fixed trailing windows, computed from the source's FULL
/// record history — never the selected-range slice — so "7d" means seven days
/// even while the dashboard shows "today". Windows with no samples come back
/// with `speed_tps: None`; the UI hides the line then.
pub(crate) fn speed_trend_windows(records: &[UsageRecord], now: i64) -> Vec<SpeedWindowStats> {
    SPEED_TREND_WINDOWS_MS
        .iter()
        .map(|w| {
            let from = now - w;
            let slice: Vec<&UsageRecord> = records
                .iter()
                .filter(|r| r.ts_ms >= from && r.ts_ms <= now)
                .collect();
            let s = compute_speed_stats(slice);
            SpeedWindowStats {
                window_ms: *w,
                speed_tps: s.speed_tps,
                speed_samples: s.speed_samples,
                completed_requests: s.completed_requests,
                speed_approximate: s.speed_approximate,
            }
        })
        .collect()
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
    /// TTFT / tok-s statistics for the same range (all-None when the source
    /// records no timing fields).
    pub speed: SpeedStats,
    /// Trailing-window tps (24h / 7d) for the trend line under the speed
    /// card; empty on the boot-snapshot path.
    pub speed_windows: Vec<SpeedWindowStats>,
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
pub struct SessionsPageDto {
    pub items: Vec<SessionSummary>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

const DEFAULT_SESSIONS_PAGE_SIZE: usize = 50;
const MAX_SESSIONS_PAGE_SIZE: usize = 100;

/// Filter and page session summaries after the complete summary list has been
/// materialized. Keeping this pure makes the full-history and boundary
/// behavior independently testable without constructing Tauri state.
pub fn query_sessions_page(
    sessions: &[SessionSummary],
    query: &str,
    sort: &str,
    page: usize,
    page_size: usize,
) -> SessionsPageDto {
    let needle = query.trim().to_lowercase();
    let mut matches: Vec<&SessionSummary> = sessions
        .iter()
        .filter(|s| {
            needle.is_empty()
                || s.id.to_lowercase().contains(&needle)
                || s.title.as_deref().unwrap_or("").to_lowercase().contains(&needle)
                || s.project.as_deref().unwrap_or("").to_lowercase().contains(&needle)
                || s.project_path.as_deref().unwrap_or("").to_lowercase().contains(&needle)
                || s.models.iter().any(|m| m.to_lowercase().contains(&needle))
        })
        .collect();

    matches.sort_by(|a, b| {
        let primary = if sort == "tokens" {
            b.agg.total_tokens().cmp(&a.agg.total_tokens())
        } else {
            b.agg.last_ts_ms.cmp(&a.agg.last_ts_ms)
        };
        primary.then_with(|| a.id.cmp(&b.id))
    });

    let total = matches.len();
    let page_size = page_size.clamp(1, MAX_SESSIONS_PAGE_SIZE);
    let start = page.saturating_mul(page_size);
    let items = if start >= total {
        Vec::new()
    } else {
        matches[start..(start + page_size).min(total)]
            .iter()
            .map(|s| (*s).clone())
            .collect()
    };
    SessionsPageDto { items, total, page, page_size }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDetailDto {
    pub name: String,
    /// Source the numbers came from: `zcode` or a local provider id.
    pub source: String,
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

fn active_session_of(inner: &crate::engine::EngineInner) -> Option<ActiveSession> {
    let now = now_ms();
    let id = inner.store.active_session_id()?;
    let summary = inner.store.session_summary(&id)?;
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

/// Model rows with shares + per-model speed stats. Shared by the ZCode
/// dashboard and the local-source dashboard views (same shapes, same
/// caliber).
pub(crate) fn model_rows(records: &[UsageRecord]) -> Vec<ModelRow> {
    let stats = group_by_model(records);
    let speed_by_name = speed_by_model(records);
    let grand: u64 = stats.iter().map(|m| m.agg.total_tokens()).sum();
    stats
        .into_iter()
        .map(|m| {
            let total = m.agg.total_tokens();
            ModelRow {
                share: if grand > 0 { total as f64 / grand as f64 } else { 0.0 },
                speed: speed_by_name.get(&m.name).cloned().unwrap_or_default(),
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
    let inner = state.engine.inner.lock().unwrap();
    dashboard_from_inner(&range_key, &inner, now_ms())
}

fn dashboard_from_inner(range_key: &str, inner: &crate::engine::EngineInner, now: i64) -> DashboardDto {
    let range = TrendRange::from_key(&range_key).unwrap_or(TrendRange::TodayHourly);

    let (from, to, _) = resolve_span(range, now, inner.store.history_start_ms());
    let records = inner.store.range(from, to);
    let models = model_rows(records);
    let agg = records.iter().fold(Agg::default(), |mut a, r| {
        a.add(r);
        a
    });
    let speed = compute_speed_stats(records);
    // Trend windows span the store's full history ( widest window is 7d ),
    // independent of the selected range; nothing to show on the boot path.
    let speed_windows = if inner.store.is_empty() {
        Vec::new()
    } else {
        let widest = SPEED_TREND_WINDOWS_MS.iter().max().copied().unwrap_or(0);
        speed_trend_windows(inner.store.range(now - widest, now), now)
    };
    let active = if inner.store.is_empty() {
        None
    } else {
        active_session_of(&*inner)
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
        speed,
        speed_windows,
        restored,
        data_error: crate::engine::gate_error(inner.error_streak, inner.last_error.clone()),
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
                speed: SpeedStats::default(),
            }
        })
        .collect()
}

#[tauri::command]
pub fn get_trend(range_key: String, state: State<'_, SharedAppState>) -> TrendDto {
    let inner = state.engine.inner.lock().unwrap();
    trend_from_inner(&range_key, &inner, now_ms())
}

fn trend_from_inner(range_key: &str, inner: &crate::engine::EngineInner, now: i64) -> TrendDto {
    let range = TrendRange::from_key(&range_key).unwrap_or(TrendRange::TodayHourly);
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageViewDto {
    pub dash: DashboardDto,
    pub trend: Option<TrendDto>,
    pub cost_summary: CostSummaryDto,
    pub revision: u64,
}

/// One IPC response, one ingestion revision, one time boundary. Models can
/// omit the trend entirely. Legacy commands remain for popup/other callers.
#[tauri::command]
pub fn get_usage_view(range_key: String, include_trend: bool, state: State<'_, SharedAppState>) -> UsageViewDto {
    let inner = state.engine.inner.lock().unwrap();
    usage_view_from_inner(&range_key, include_trend, &inner, &state.pricing, now_ms())
}

fn usage_view_from_inner(range_key: &str, include_trend: bool, inner: &crate::engine::EngineInner, pricing: &PricingManager, now: i64) -> UsageViewDto {
    let dash = dashboard_from_inner(range_key, inner, now);
    let cost_summary = pricing.cost_summary(&dash.range_key, inner.store.range(dash.from_ms, dash.to_ms));
    UsageViewDto {
        trend: include_trend.then(|| trend_from_inner(range_key, inner, now)),
        dash,
        cost_summary,
        revision: inner.store.total_ingested,
    }
}

#[tauri::command]
pub fn get_sessions_page(
    query: String,
    sort: String,
    page: usize,
    page_size: usize,
    state: State<'_, SharedAppState>,
) -> SessionsPageDto {
    let mut inner = state.engine.inner.lock().unwrap();
    let mut sessions: Vec<SessionSummary> = if inner.store.is_empty() {
        inner.boot.as_ref().map(|boot| boot.sessions.clone()).unwrap_or_default()
    } else {
        inner.store.session_summaries().to_vec()
    };
    drop(inner);
    // Local sources merge into the same list with their display prefixes
    // (cx- / cc- / dsh-); only enabled providers contribute sessions.
    let providers = current_settings(&state).providers;
    if providers.codex_enabled {
        sessions.extend(state.hub.local_session_summaries(crate::providers::PROVIDER_CODEX));
    }
    if providers.dsh_enabled {
        sessions.extend(state.hub.local_session_summaries(crate::providers::PROVIDER_DSH));
    }
    if providers.claude_code_enabled {
        sessions.extend(state.hub.local_session_summaries(crate::providers::PROVIDER_CLAUDE_CODE));
    }
    query_sessions_page(
        &sessions,
        &query,
        &sort,
        page,
        if page_size == 0 { DEFAULT_SESSIONS_PAGE_SIZE } else { page_size },
    )
}

#[tauri::command]
pub fn get_session_detail(session_id: String, state: State<'_, SharedAppState>) -> Option<SessionDetailDto> {
    // Prefixed ids (cx- / cc- / dsh-) route to the owning local provider;
    // unprefixed ids stay on the ZCode engine.
    if crate::providers::session_index::split_prefixed(&session_id).is_some() {
        return state.hub.local_session_detail(&session_id);
    }
    let inner = state.engine.inner.lock().unwrap();
    let summary = inner.store.session_summary(&session_id)?;
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

/// ZCode-density dashboard view for one local source (codex / dsh /
/// claude-code): aggregates, model rows, trend buckets and the official-
/// price cost estimate over the shared record schema.
#[tauri::command]
pub fn get_local_usage_view(
    provider: String,
    range_key: String,
    include_trend: bool,
    state: State<'_, SharedAppState>,
) -> Option<UsageViewDto> {
    state
        .hub
        .local_usage_view(&provider, &range_key, include_trend, &state.pricing)
}

/// Per-model detail over one source's raw records. `records` is the whole
/// unfiltered set for that source; rows of other models are ignored.
fn model_detail_from_records(
    name: &str,
    source: &str,
    records: &[UsageRecord],
    now: i64,
) -> Option<ModelDetailDto> {
    let mut all_agg = Agg::default();
    for r in records.iter().filter(|r| r.model == name) {
        all_agg.add(r);
    }
    if all_agg.requests == 0 {
        return None;
    }
    let fold = |from_ms: i64| -> Agg {
        records
            .iter()
            .filter(|r| r.model == name && r.ts_ms >= from_ms && r.ts_ms <= now)
            .fold(Agg::default(), |mut a, r| {
                a.add(r);
                a
            })
    };
    let today = fold(crate::zcode::aggregate::local_day_start_ms(now));
    let last_7d = fold(now - 7 * 24 * 3600_000);
    let last_30d = fold(now - 30 * 24 * 3600_000);
    let (t_from, t_to) = (now - 30 * 24 * 3600_000, now);
    let mine_30d: Vec<UsageRecord> = records
        .iter()
        .filter(|r| r.model == name && r.ts_ms >= t_from && r.ts_ms <= t_to)
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
        name: name.to_string(),
        source: source.to_string(),
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

/// Model detail for one source. An empty `provider` (or `zcode`) reads the
/// ZCode engine store; a local source id reads that provider's cached
/// records, gated on it being enabled in settings.
#[tauri::command]
pub fn get_model_detail(
    name: String,
    provider: Option<String>,
    state: State<'_, SharedAppState>,
) -> Option<ModelDetailDto> {
    let now = now_ms();
    let provider = provider.unwrap_or_default();
    if provider.is_empty() || provider == crate::providers::PROVIDER_ZCODE {
        let inner = state.engine.inner.lock().unwrap();
        return model_detail_from_records(
            &name,
            crate::providers::PROVIDER_ZCODE,
            inner.store.all(),
            now,
        );
    }
    let settings = current_settings(&state).providers;
    let enabled = match provider.as_str() {
        crate::providers::PROVIDER_CODEX => settings.codex_enabled,
        crate::providers::PROVIDER_DSH => settings.dsh_enabled,
        crate::providers::PROVIDER_CLAUDE_CODE => settings.claude_code_enabled,
        _ => false,
    };
    if !enabled {
        return None;
    }
    let records = state.hub.local_records(&provider);
    model_detail_from_records(&name, &provider, &records, now)
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
        if data_dir_changed {
            // A different data root invalidates everything already ingested:
            // rebuild from scratch instead of appending the new root's records
            // onto the old root's numbers.
            state.engine.reset_data_root();
        } else if paused_changed {
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

/// Per-line cost breakdown for one model over a range. `provider`
/// (optional) scopes the records to a local source (codex / dsh /
/// claude-code); without it the ZCode engine's records are used.
#[tauri::command]
pub fn cost_detail(
    range: String,
    model: String,
    provider: Option<String>,
    state: State<'_, SharedAppState>,
) -> CostDetailDto {
    if let Some(provider) = provider.as_deref() {
        return state
            .hub
            .local_cost_detail(provider, &range, &model, &state.pricing, now_ms());
    }
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
pub fn history_health(state: State<'_, SharedAppState>) -> crate::providers::history::HistoryHealth {
    state.hub.history_health()
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

#[cfg(test)]
mod sessions_page_tests {
    use super::*;

    #[test]
    fn usage_view_shares_range_revision_and_optional_trend() {
        let (engine, _rx) = crate::engine::Engine::new();
        let pricing = PricingManager::new(None);
        let mut inner = engine.inner.lock().unwrap();
        inner.store.ingest(vec![UsageRecord {
            ts_ms: 1_756_300_000_000,
            model: "unpriced-test-model".into(),
            session_id: Some("s".into()), project: None,
            input_tokens: 100, output_tokens: 20, reasoning_tokens: None,
            cache_read_tokens: None, cache_write_tokens: None, source_file: "synthetic".into(),
            ..Default::default()
        }]);
        let view = usage_view_from_inner("all", true, &inner, &pricing, 1_756_300_060_000);
        assert_eq!(view.revision, 1);
        assert_eq!(view.dash.agg.requests, 1);
        let trend = view.trend.unwrap();
        assert_eq!(view.dash.from_ms, trend.from_ms);
        assert_eq!(view.dash.to_ms, trend.to_ms);
        assert_eq!(view.dash.range_key, trend.range_key);
        assert_eq!(trend.buckets.iter().map(|b| b.agg.requests).sum::<u64>(), 1);
        assert!(usage_view_from_inner("all", false, &inner, &pricing, 1_756_300_060_000).trend.is_none());
    }

    fn summary(id: &str, project: Option<&str>, model: &str, last: i64, tokens: u64) -> SessionSummary {
        let mut agg = Agg::default();
        agg.requests = 1;
        agg.input = tokens;
        // Real Aggs accumulate this via add(); the fixture mirrors one record
        // whose display total equals its input.
        agg.total_sum = tokens;
        agg.last_ts_ms = Some(last);
        agg.first_ts_ms = Some(last);
        SessionSummary {
            id: id.into(),
            project: project.map(str::to_owned),
            models: vec![model.into()],
            agg,
            ..Default::default()
        }
    }

    #[test]
    fn searches_complete_history_before_paging() {
        let mut all: Vec<_> = (0..600)
            .map(|i| summary(&format!("session-{i:04}"), Some("older-project"), "model-a", i, i as u64))
            .collect();
        all[7].project = Some("historic-project".into());
        all[7].models = vec!["historic-model".into()];

        let result = query_sessions_page(&all, "historic-model", "recent", 0, 100);
        assert_eq!(result.total, 1);
        assert_eq!(result.items[0].id, "session-0007");
    }

    #[test]
    fn searches_match_title_and_project_path() {
        let mut a = summary("s-title", None, "m", 10, 5);
        a.title = Some("优化登录性能".into());
        let mut b = summary("s-path", None, "m", 9, 5);
        b.project = Some("panel".into());
        b.project_path = Some("/home/u/projects/zcode-usage-panel".into());
        let all = vec![a, b, summary("s-none", None, "m", 8, 5)];

        // Real title text matches.
        assert_eq!(query_sessions_page(&all, "登录性能", "recent", 0, 10).items[0].id, "s-title");
        // Full workspace path matches even though only the folder name shows.
        let hit = query_sessions_page(&all, "zcode-usage-panel", "recent", 0, 10);
        assert_eq!(hit.total, 1);
        assert_eq!(hit.items[0].id, "s-path");
        // Folder-name substring also matches via the path.
        assert_eq!(query_sessions_page(&all, "projects/zcode", "recent", 0, 10).total, 1);
        assert_eq!(query_sessions_page(&all, "不存在", "recent", 0, 10).total, 0);
    }

    #[test]
    fn sort_and_pagination_boundaries_are_stable() {
        let all = vec![
            summary("b", None, "m", 10, 5),
            summary("a", None, "m", 10, 5),
            summary("c", None, "m", 9, 20),
        ];
        let recent = query_sessions_page(&all, "", "recent", 0, 2);
        assert_eq!(recent.items.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
        let tokens = query_sessions_page(&all, "", "tokens", 1, 2);
        assert_eq!(tokens.total, 3);
        assert_eq!(tokens.items.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["b"]);
        assert!(query_sessions_page(&all, "", "recent", 2, 2).items.is_empty());
        assert_eq!(query_sessions_page(&all, "", "recent", 0, 999).page_size, 100);
    }
}

#[cfg(test)]
mod model_detail_tests {
    use super::*;

    fn rec(model: &str, ts_ms: i64, input: u64, session: &str) -> UsageRecord {
        UsageRecord {
            ts_ms,
            model: model.into(),
            session_id: Some(session.into()),
            input_tokens: input,
            output_tokens: 10,
            source_file: "synthetic".into(),
            ..Default::default()
        }
    }

    /// One source's records only: other models and other sources must not
    /// leak into the detail, and the DTO carries the source it came from.
    #[test]
    fn model_detail_scopes_to_model_and_stamps_source() {
        let now = 1_756_300_000_000;
        let day = 24 * 3600_000;
        let records = vec![
            rec("gpt-5.6-sol", now - 1_000, 300, "s1"),
            rec("gpt-5.6-sol", now - 2 * day, 100, "s2"),
            rec("gpt-5.6-mini", now - 1_000, 999, "s3"),
            rec("gpt-5.6-sol", now - 40 * day, 77, "s4"),
        ];
        let detail = model_detail_from_records("gpt-5.6-sol", "codex", &records, now).unwrap();
        assert_eq!(detail.name, "gpt-5.6-sol");
        assert_eq!(detail.source, "codex");
        assert_eq!(detail.all_time.requests, 3);
        assert_eq!(detail.last_30d.requests, 2);
        assert_eq!(detail.last_7d.requests, 2);
        // Top sessions only count the 30-day window, heaviest first.
        assert_eq!(
            detail.top_sessions,
            vec![("s1".to_string(), 310), ("s2".to_string(), 110)]
        );
        assert!(model_detail_from_records("not-used", "codex", &records, now).is_none());
        // A provider with no records of its own yields nothing rather than
        // borrowing another source's numbers.
        assert!(model_detail_from_records("gpt-5.6-sol", "dsh", &[], now).is_none());
    }
}

#[cfg(test)]
mod speed_trend_tests {
    use super::*;

    fn speed_rec(ts_ms: i64, output: u64, ttft: u64, duration: u64) -> UsageRecord {
        UsageRecord {
            ts_ms,
            model: "m".into(),
            output_tokens: output,
            ttft_ms: Some(ttft),
            duration_ms: Some(duration),
            source_file: "synthetic".into(),
            ..Default::default()
        }
    }

    /// Windows are wall-clock trailing spans: a 3-day-old request counts only
    /// toward 7d, a 10-day-old one toward neither — and each window keeps the
    /// exact speed caliber (weighted by generation time).
    #[test]
    fn speed_trend_windows_bucket_by_wall_clock() {
        let now = 1_756_300_000_000_i64;
        let day = 86_400_000_i64;
        let records = vec![
            // 100 tokens over (2000-1000)ms = 100 tps, inside both windows.
            speed_rec(now - 3_600_000, 100, 1000, 2000),
            // 100 tokens over (11000-1000)ms = 10 tps, only inside 7d.
            speed_rec(now - 3 * day, 100, 1000, 11_000),
            // Outside every window.
            speed_rec(now - 10 * day, 999, 1000, 2000),
        ];
        let windows = speed_trend_windows(&records, now);
        assert_eq!(windows.len(), 2);
        let h24 = &windows[0];
        let d7 = &windows[1];
        assert_eq!(h24.window_ms, day);
        assert_eq!(h24.speed_samples, 1);
        assert!((h24.speed_tps.unwrap() - 100.0).abs() < 1e-6);
        assert_eq!(d7.window_ms, 7 * day);
        assert_eq!(d7.speed_samples, 2);
        // Weighted: 200 tokens over 11000ms of generation.
        assert!((d7.speed_tps.unwrap() - 200.0 * 1000.0 / 11_000.0).abs() < 1e-6);
        // Empty history still yields both windows, honestly null.
        let empty = speed_trend_windows(&[], now);
        assert_eq!(empty.len(), 2);
        assert!(empty.iter().all(|w| w.speed_tps.is_none() && w.speed_samples == 0));
    }
}
