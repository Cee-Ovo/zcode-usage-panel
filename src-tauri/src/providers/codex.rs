//! OpenAI Codex provider — 100% local official-client data.
//!
//! Data source: the Codex CLI's own session rollouts
//! `<CODEX_HOME>/sessions/YYYY/MM/DD/rollout-*.jsonl` (append-only JSONL).
//! Each line is `{timestamp, ordinal, type, payload}`; we consume:
//! - `session_meta` → session id / start time,
//! - `turn_context` → model name,
//! - `event_msg{type:"token_count"}` →
//!     `payload.info.total_token_usage` (cumulative per session — the LAST
//!     event per file is the session total, no double counting), and
//!     `payload.rate_limits` (the official quota the Codex backend pushed:
//!     5-hour window + weekly usage percent, reset timestamps, credits,
//!     plan type).
//!
//! Plan quota (official rate limits) and local harness token usage are kept
//! in SEPARATE fields of the snapshot — never merged into one metric.
//! Reading `auth.json` is limited to the id_token's *decoded claims* (email,
//! plan); the token bytes themselves are never copied, logged, or sent
//! anywhere. This provider makes no network requests.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{
    LocalUsage, LocalUsageRange, ModelUsageRow, ProviderSnapshot, ProviderStatus, QuotaWindow,
    TokenBreakdown,
};

const CODEX_CACHE_SCHEMA_VERSION: u32 = 2;
const RANGE_KEYS: [&str; 6] = ["today", "60m", "24h", "7d", "30d", "all"];

// ---------------------------------------------------------------------------
// Wire types (subset of Codex's rollout schema)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TotalTokenUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    #[serde(default)]
    pub cache_write_input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub reasoning_output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RateLimitWindow {
    #[serde(default)]
    pub used_percent: Option<f64>,
    #[serde(default)]
    pub window_minutes: Option<u64>,
    /// Unix seconds.
    #[serde(default)]
    pub resets_at: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RateLimits {
    #[serde(default)]
    pub limit_id: Option<String>,
    #[serde(default)]
    pub primary: Option<RateLimitWindow>,
    #[serde(default)]
    pub secondary: Option<RateLimitWindow>,
    #[serde(default)]
    pub credits: Option<serde_json::Value>,
    #[serde(default)]
    pub plan_type: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct SessionUsage {
    session_id: String,
    model: String,
    first_ts_ms: i64,
    last_ts_ms: i64,
    /// Number of token_count events (= model responses) in this session.
    responses: u64,
    /// Latest cumulative counters, used as the baseline for incremental
    /// parsing. This is deliberately distinct from `all_time`: counters may
    /// reset within one rollout file.
    totals: TotalTokenUsage,
    /// Exact accumulated deltas for this file, including counter resets.
    #[serde(default)]
    all_time: TotalTokenUsage,
    /// Exact accumulated deltas by the model active at each token event.
    #[serde(default)]
    model_totals: HashMap<String, TotalTokenUsage>,
    #[serde(default)]
    model_requests: HashMap<String, u64>,
    /// Timestamped deltas retained for rolling range aggregation. Entries
    /// older than 30 days are pruned after each poll.
    #[serde(default)]
    recent: Vec<UsageEvent>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct UsageEvent {
    ts_ms: i64,
    model: String,
    delta: TotalTokenUsage,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct FileEntry {
    /// Byte watermark — rollouts are append-only, so unchanged prefixes are
    /// never re-parsed.
    offset: u64,
    complete: bool,
    session: SessionUsage,
}

/// Persisted parse cache: per-file watermarks + the freshest official rate
/// limits seen. Saved atomically; corrupt files fall back to empty.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CodexCache {
    #[serde(default)]
    schema_version: u32,
    files: HashMap<String, FileEntry>,
    #[serde(default)]
    last_rate_limits: Option<(i64, RateLimits)>,
    #[serde(default)]
    saved_at_ms: i64,
}

impl CodexCache {
    fn fresh() -> Self {
        Self {
            schema_version: CODEX_CACHE_SCHEMA_VERSION,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

pub struct CodexProvider {
    home: PathBuf,
    cache: CodexCache,
    cache_path: Option<PathBuf>,
    cache_needs_persist: bool,
}

impl CodexProvider {
    pub fn new(cache_path: Option<PathBuf>) -> Self {
        let (cache, cache_needs_persist) = cache_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| {
                let parsed = serde_json::from_str::<CodexCache>(&t).ok()?;
                if parsed.schema_version == CODEX_CACHE_SCHEMA_VERSION {
                    Some((parsed, false))
                } else {
                    Some((CodexCache::fresh(), true))
                }
            })
            .unwrap_or_else(|| (CodexCache::fresh(), false));
        Self {
            home: default_home(),
            cache,
            cache_path,
            cache_needs_persist,
        }
    }

    /// Point at a (possibly different) CODEX_HOME; a changed root resets the
    /// parse cache since watermarks belong to specific files.
    pub fn with_home(&mut self, home: PathBuf) -> &mut Self {
        if self.home != home {
            // A cache loaded from disk can legitimately be paired with an
            // explicitly configured home after construction. Keep it when
            // its file keys belong to that home; otherwise switching roots
            // must discard watermarks and parse state.
            let belongs_to_home = !self.cache.files.is_empty()
                && self
                    .cache
                    .files
                    .keys()
                    .all(|key| Path::new(key).starts_with(&home));
            if !belongs_to_home {
                self.cache = CodexCache::fresh();
                self.cache_needs_persist = true;
            }
        }
        self.home = home;
        self
    }

    pub fn persist_cache(&self) {
        if let Some(p) = &self.cache_path {
            if let Ok(json) = serde_json::to_string(&self.cache) {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let tmp = p.with_extension("tmp");
                if std::fs::write(&tmp, json).is_ok() {
                    let _ = std::fs::rename(&tmp, p);
                }
            }
        }
    }

    /// One poll cycle. Never panics; every failure degrades to a status.
    pub fn poll(&mut self, now_ms: i64) -> ProviderSnapshot {
        let mut snap = ProviderSnapshot::empty(super::PROVIDER_CODEX, ProviderStatus::Ok, now_ms);
        snap.source = "Codex 本地 session 文件(官方客户端数据,离线读取)".into();
        snap.source_url = Some("https://developers.openai.com/codex/".into());

        if !self.home.is_dir() {
            snap.status = ProviderStatus::NotInstalled;
            snap.error = Some("未检测到 Codex CLI 数据目录(可指定 CODEX_HOME)".into());
            return snap;
        }

        let (email, jwt_plan) = read_account_claims(&self.home.join("auth.json"));
        if let Some(email) = &email {
            snap.account = Some(email.clone());
        } else {
            snap.status = ProviderStatus::NotConfigured;
            snap.error = Some("Codex 未登录(无 auth.json)".into());
        }

        let mut files = Vec::new();
        collect_jsonl(&self.home.join("sessions"), &mut files);
        files.sort();

        let live: std::collections::HashSet<String> = files
            .iter()
            .map(|f| f.to_string_lossy().into_owned())
            .collect();
        let file_count_before = self.cache.files.len();
        self.cache.files.retain(|k, _| live.contains(k));

        let mut changed = self.cache_needs_persist || file_count_before != self.cache.files.len();
        let mut best_rl = self.cache.last_rate_limits.take();
        for path in &files {
            let key = path.to_string_lossy().into_owned();
            let entry = self.cache.files.entry(key).or_default();
            match advance_file(path, entry, &mut best_rl) {
                Ok(grew) => changed |= grew,
                Err(why) => {
                    self.cache
                        .files
                        .remove(&path.to_string_lossy().into_owned());
                    snap.notes
                        .push(format!("跳过无法读取的 session 文件:{why}"));
                }
            }
        }
        self.cache.last_rate_limits = best_rl;
        self.cache.saved_at_ms = now_ms;
        changed |= prune_recent(&mut self.cache.files, now_ms);

        snap.local_usage = Some(aggregate_local(&self.cache.files, now_ms));

        if let Some((ts_ms, rl)) = self.cache.last_rate_limits.clone() {
            if let Some(p) = &rl.primary {
                snap.windows.push(window_from(p, "5h", "5 小时窗口"));
            }
            if let Some(s) = &rl.secondary {
                snap.windows.push(window_from(s, "weekly", "周额度"));
            }
            snap.plan_name = rl
                .plan_type
                .clone()
                .or(jwt_plan)
                .map(|p| format!("ChatGPT {p}"));
            if let Some(credits) = &rl.credits {
                if let Some(true) = credits.get("has_credits").and_then(|v| v.as_bool()) {
                    if let Some(balance) = credits.get("balance").and_then(|v| v.as_str()) {
                        snap.notes.push(format!(
                            "Credits 余额:{balance}(API 抵扣额度,与套餐额度独立)"
                        ));
                    }
                }
            }
            let age_min = now_ms.saturating_sub(ts_ms) / 60_000;
            snap.notes.push(format!(
                "额度来自 Codex 官方 rate_limits,{age_min} 分钟前更新(Codex 发起请求时刷新)"
            ));
            if now_ms.saturating_sub(ts_ms) > 6 * 3600_000 {
                snap.status = ProviderStatus::Stale;
            }
        } else if snap.status == ProviderStatus::Ok {
            snap.status = ProviderStatus::NotConfigured;
            snap.error =
                Some("尚未从本地 session 获取到官方额度(用 Codex 发起一次对话后即可)".into());
        }

        if changed {
            self.persist_cache();
            self.cache_needs_persist = false;
        }
        snap
    }
}

pub fn default_home() -> PathBuf {
    if let Some(p) = std::env::var_os("CODEX_HOME") {
        let p = PathBuf::from(p);
        if p.is_dir() {
            return p;
        }
    }
    dirs::home_dir()
        .map(|h| h.join(".codex"))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

fn window_from(w: &RateLimitWindow, key: &str, label: &str) -> QuotaWindow {
    QuotaWindow {
        key: key.into(),
        label: label.into(),
        used_percent: w.used_percent,
        unit: Some("% 套餐额度".into()),
        reset_at_ms: w.resets_at.filter(|t| *t > 0).map(|t| t * 1000),
        window_minutes: w.window_minutes,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Parsing helpers (pure, testable)
// ---------------------------------------------------------------------------

/// Advance one rollout file past its watermark. Only complete lines are
/// consumed (a half-written trailing line waits for the next poll).
fn advance_file(
    path: &Path,
    entry: &mut FileEntry,
    best_rl: &mut Option<(i64, RateLimits)>,
) -> Result<bool, String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let size = f.metadata().map_err(|e| e.to_string())?.len();
    if size < entry.offset {
        // Truncated/rewritten → re-read from the start.
        entry.offset = 0;
        entry.session = SessionUsage::default();
    }
    if size == entry.offset {
        entry.complete = true;
        return Ok(false);
    }
    f.seek(SeekFrom::Start(entry.offset))
        .map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(f);
    let mut grew = false;
    let mut offset = entry.offset;
    loop {
        let mut line = String::new();
        let Ok(n) = reader.read_line(&mut line) else {
            break;
        };
        if n == 0 {
            break;
        }
        if !line.ends_with('\n') {
            break; // half line — hold back
        }
        offset += n as u64;
        grew = true;
        apply_line(&line, &mut entry.session, best_rl);
    }
    entry.offset = offset;
    entry.complete = entry.offset == size;
    Ok(grew)
}

fn apply_line(line: &str, session: &mut SessionUsage, best_rl: &mut Option<(i64, RateLimits)>) {
    if !line.contains("\"type\"") {
        return;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return;
    };
    let ts_ms = v
        .get("timestamp")
        .and_then(|t| t.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis())
        .unwrap_or(0);
    match v.get("type").and_then(|t| t.as_str()) {
        Some("session_meta") => {
            let p = &v["payload"];
            session.session_id = p
                .get("session_id")
                .or_else(|| p.get("id"))
                .and_then(|s| s.as_str())
                .unwrap_or_default()
                .to_string();
            if session.first_ts_ms == 0 {
                session.first_ts_ms = ts_ms;
            }
            if ts_ms > session.last_ts_ms {
                session.last_ts_ms = ts_ms;
            }
        }
        Some("turn_context") => {
            if let Some(m) = v["payload"].get("model").and_then(|m| m.as_str()) {
                session.model = m.to_string();
            }
        }
        Some("event_msg") => {
            let p = &v["payload"];
            if p.get("type").and_then(|t| t.as_str()) == Some("token_count") {
                if let Some(t) = p.get("info").and_then(|i| i.get("total_token_usage")) {
                    if let Ok(parsed) = serde_json::from_value::<TotalTokenUsage>(t.clone()) {
                        // Cumulative counters are normally monotonic, but a
                        // new reasoning session can reset them. A decrease
                        // therefore means the post-reset value, not zero.
                        let delta = counter_delta(&parsed, &session.totals);
                        add_total(&mut session.all_time, &delta);
                        let model = event_model(&session.model, p);
                        add_total(
                            session.model_totals.entry(model.clone()).or_default(),
                            &delta,
                        );
                        *session.model_requests.entry(model.clone()).or_default() += 1;
                        session.recent.push(UsageEvent {
                            ts_ms,
                            model,
                            delta,
                        });
                        session.totals = parsed; // cumulative — last wins
                        session.responses += 1;
                        if ts_ms > session.last_ts_ms {
                            session.last_ts_ms = ts_ms;
                        }
                    }
                }
                if let Some(rl) = p.get("rate_limits").filter(|r| !r.is_null()) {
                    if let Ok(parsed) = serde_json::from_value::<RateLimits>(rl.clone()) {
                        if best_rl.as_ref().map(|(t, _)| ts_ms >= *t).unwrap_or(true) {
                            *best_rl = Some((ts_ms, parsed));
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn collect_jsonl(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_jsonl(&p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            out.push(p);
        }
    }
}

fn add_total(target: &mut TotalTokenUsage, delta: &TotalTokenUsage) {
    target.input_tokens += delta.input_tokens;
    target.cached_input_tokens += delta.cached_input_tokens;
    target.cache_write_input_tokens += delta.cache_write_input_tokens;
    target.output_tokens += delta.output_tokens;
    target.reasoning_output_tokens += delta.reasoning_output_tokens;
    target.total_tokens += delta.total_tokens;
}

fn counter_delta(current: &TotalTokenUsage, previous: &TotalTokenUsage) -> TotalTokenUsage {
    fn one(current: u64, previous: u64) -> u64 {
        current.checked_sub(previous).unwrap_or(current)
    }
    TotalTokenUsage {
        input_tokens: one(current.input_tokens, previous.input_tokens),
        cached_input_tokens: one(current.cached_input_tokens, previous.cached_input_tokens),
        cache_write_input_tokens: one(
            current.cache_write_input_tokens,
            previous.cache_write_input_tokens,
        ),
        output_tokens: one(current.output_tokens, previous.output_tokens),
        reasoning_output_tokens: one(
            current.reasoning_output_tokens,
            previous.reasoning_output_tokens,
        ),
        total_tokens: one(current.total_tokens, previous.total_tokens),
    }
}

fn event_model(current: &str, payload: &serde_json::Value) -> String {
    payload
        .get("model")
        .or_else(|| payload.get("info").and_then(|i| i.get("model")))
        .and_then(|m| m.as_str())
        .filter(|m| !m.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            if current.is_empty() {
                "unknown".into()
            } else {
                current.into()
            }
        })
}

fn to_breakdown(total: &TotalTokenUsage, requests: u64) -> TokenBreakdown {
    TokenBreakdown {
        requests,
        input_tokens: total.input_tokens,
        cached_input_tokens: total.cached_input_tokens,
        cache_write_tokens: total.cache_write_input_tokens,
        output_tokens: total.output_tokens,
        reasoning_tokens: total.reasoning_output_tokens,
        // This is the raw Codex total_tokens field. Never derive it by
        // adding cached/input/output fields.
        total_tokens: total.total_tokens,
    }
}

fn in_range(ts_ms: i64, key: &str, now_ms: i64) -> bool {
    match key {
        "today" => ts_ms >= crate::zcode::aggregate::local_day_start_ms(now_ms) && ts_ms <= now_ms,
        "60m" => ts_ms >= now_ms.saturating_sub(60 * 60_000) && ts_ms <= now_ms,
        "24h" => ts_ms >= now_ms.saturating_sub(24 * 60 * 60_000) && ts_ms <= now_ms,
        "7d" => ts_ms >= now_ms.saturating_sub(7 * 24 * 60 * 60_000) && ts_ms <= now_ms,
        "30d" => ts_ms >= now_ms.saturating_sub(30 * 24 * 60 * 60_000) && ts_ms <= now_ms,
        "all" => true,
        _ => false,
    }
}

fn model_rows(models: HashMap<String, TokenBreakdown>) -> Vec<ModelUsageRow> {
    let mut rows: Vec<_> = models
        .into_iter()
        .map(|(model, breakdown)| ModelUsageRow { model, breakdown })
        .collect();
    rows.sort_by(|a, b| {
        b.breakdown
            .total_tokens
            .cmp(&a.breakdown.total_tokens)
            .then_with(|| a.model.cmp(&b.model))
    });
    rows
}

fn prune_recent(files: &mut HashMap<String, FileEntry>, now_ms: i64) -> bool {
    let cutoff = now_ms.saturating_sub(30 * 24 * 60 * 60_000);
    let mut changed = false;
    for entry in files.values_mut() {
        let before = entry.session.recent.len();
        entry.session.recent.retain(|event| event.ts_ms >= cutoff);
        changed |= before != entry.session.recent.len();
    }
    changed
}

fn aggregate_local(files: &HashMap<String, FileEntry>, now_ms: i64) -> LocalUsage {
    let mut usage = LocalUsage::default();
    for key in RANGE_KEYS {
        let mut breakdown = TokenBreakdown::default();
        let mut models: HashMap<String, TokenBreakdown> = HashMap::new();
        let mut sessions = 0;
        for entry in files.values() {
            let s = &entry.session;
            if s.session_id.is_empty() && s.responses == 0 {
                continue;
            }
            if key == "all" {
                sessions += 1;
                breakdown.requests += s.responses;
                let all = to_breakdown(&s.all_time, 0);
                breakdown.input_tokens += all.input_tokens;
                breakdown.cached_input_tokens += all.cached_input_tokens;
                breakdown.cache_write_tokens += all.cache_write_tokens;
                breakdown.output_tokens += all.output_tokens;
                breakdown.reasoning_tokens += all.reasoning_tokens;
                breakdown.total_tokens += all.total_tokens;
                for (model, total) in &s.model_totals {
                    let row = models.entry(model.clone()).or_default();
                    *row = add_breakdown(
                        row,
                        &to_breakdown(total, s.model_requests.get(model).copied().unwrap_or(0)),
                    );
                }
                continue;
            }
            let mut in_session = false;
            for event in &s.recent {
                if !in_range(event.ts_ms, key, now_ms) {
                    continue;
                }
                in_session = true;
                breakdown.requests += 1;
                let delta = to_breakdown(&event.delta, 1);
                breakdown.input_tokens += delta.input_tokens;
                breakdown.cached_input_tokens += delta.cached_input_tokens;
                breakdown.cache_write_tokens += delta.cache_write_tokens;
                breakdown.output_tokens += delta.output_tokens;
                breakdown.reasoning_tokens += delta.reasoning_tokens;
                breakdown.total_tokens += delta.total_tokens;
                let row = models.entry(event.model.clone()).or_default();
                *row = add_breakdown(row, &delta);
            }
            if in_session {
                sessions += 1;
            }
        }
        let range = LocalUsageRange {
            key: key.into(),
            breakdown,
            sessions,
            models: model_rows(models),
        };
        usage.ranges.push(range);
    }
    usage.today = usage.ranges[0].breakdown.clone();
    usage.last_7d = usage.ranges[3].breakdown.clone();
    usage.all_time = usage.ranges[5].breakdown.clone();
    usage.sessions = usage.ranges[5].sessions;
    usage.models = usage.ranges[5].models.clone();
    usage
}

fn add_breakdown(target: &TokenBreakdown, delta: &TokenBreakdown) -> TokenBreakdown {
    TokenBreakdown {
        requests: target.requests + delta.requests,
        input_tokens: target.input_tokens + delta.input_tokens,
        cached_input_tokens: target.cached_input_tokens + delta.cached_input_tokens,
        cache_write_tokens: target.cache_write_tokens + delta.cache_write_tokens,
        output_tokens: target.output_tokens + delta.output_tokens,
        reasoning_tokens: target.reasoning_tokens + delta.reasoning_tokens,
        total_tokens: target.total_tokens + delta.total_tokens,
    }
}

/// Decode ONLY the id_token claims segment (never the signature, never the
/// raw token). Returns (email, plan).
fn read_account_claims(auth_path: &Path) -> (Option<String>, Option<String>) {
    let Ok(text) = std::fs::read_to_string(auth_path) else {
        return (None, None);
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return (None, None);
    };
    let Some(id_token) = v.pointer("/tokens/id_token").and_then(|t| t.as_str()) else {
        return (None, None);
    };
    let claims = decode_jwt_claims(id_token);
    let email = claims
        .get("email")
        .and_then(|e| e.as_str())
        .map(|s| s.to_string());
    let plan = claims
        .get("https://api.openai.com/auth")
        .and_then(|a| a.get("chatgpt_plan_type"))
        .and_then(|p| p.as_str())
        .map(|s| s.to_string());
    (email, plan)
}

/// Base64url-decode the middle JWT segment into JSON claims.
pub fn decode_jwt_claims(token: &str) -> serde_json::Value {
    use base64::Engine;
    let Some((_, rest)) = token.split_once('.') else {
        return serde_json::Value::Null;
    };
    let Some(payload) = rest.split('.').next() else {
        return serde_json::Value::Null;
    };
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .unwrap_or_default();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_home() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join(".codex");
        std::fs::create_dir_all(home.join("sessions/2026/08/29")).unwrap();
        (dir, home)
    }

    fn write_rollout(home: &Path, name: &str, lines: &[String]) -> PathBuf {
        let p = home.join("sessions/2026/08/29").join(name);
        std::fs::write(&p, lines.join("\n") + "\n").unwrap();
        p
    }

    fn meta_line() -> String {
        r#"{"timestamp":"2026-08-29T13:43:37.330Z","ordinal":1,"type":"session_meta","payload":{"session_id":"s1","cwd":"/tmp"}}"#.into()
    }

    fn turn_line() -> String {
        r#"{"timestamp":"2026-08-29T13:43:40.000Z","ordinal":2,"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}"#.into()
    }

    fn token_line(input: u64, total: u64) -> String {
        format!(
            r#"{{"timestamp":"2026-08-29T13:43:45.727Z","ordinal":3,"type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{input},"cached_input_tokens":10,"cache_write_input_tokens":0,"output_tokens":100,"reasoning_output_tokens":14,"total_tokens":{total}}},"last_token_usage":{{}},"model_context_window":258400}}}}}}"#
        )
    }

    fn token_event_line(timestamp: &str, input: u64, total: u64) -> String {
        serde_json::json!({
            "timestamp": timestamp,
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "total_token_usage": {
                        "input_tokens": input,
                        "cached_input_tokens": 0,
                        "cache_write_input_tokens": 0,
                        "output_tokens": 0,
                        "reasoning_output_tokens": 0,
                        "total_tokens": total
                    }
                }
            }
        })
        .to_string()
    }

    fn total(value: u64) -> TotalTokenUsage {
        TotalTokenUsage {
            input_tokens: value,
            total_tokens: value,
            ..Default::default()
        }
    }

    fn rate_line(primary_pct: f64, resets: i64, plan: &str) -> String {
        format!(
            r#"{{"timestamp":"2026-08-29T13:44:00.000Z","ordinal":4,"type":"event_msg","payload":{{"type":"token_count","info":{{}},"rate_limits":{{"limit_id":"codex","primary":{{"used_percent":{primary_pct},"window_minutes":300,"resets_at":{resets}}},"secondary":{{"used_percent":50.0,"window_minutes":10080,"resets_at":1790000000}},"credits":{{"has_credits":true,"unlimited":false,"balance":"1000"}},"plan_type":"{plan}"}}}}}}"#
        )
    }

    #[test]
    fn parses_sessions_and_rate_limits() {
        let (_dir, home) = tmp_home();
        write_rollout(
            &home,
            "rollout-a.jsonl",
            &[
                meta_line(),
                turn_line(),
                token_line(1000, 1114),
                rate_line(8.0, 1788025458, "plus"),
            ],
        );
        let mut p = CodexProvider::new(None);
        p.with_home(home.clone());
        let snap = p.poll(1_788_030_000_000);
        assert_eq!(snap.status, ProviderStatus::NotConfigured); // no auth.json
        let lu = snap.local_usage.unwrap();
        assert_eq!(lu.all_time.total_tokens, 1114);
        assert_eq!(lu.all_time.input_tokens, 1000);
        assert_eq!(lu.sessions, 1);
        assert_eq!(lu.models[0].model, "gpt-5.6-sol");
        assert_eq!(snap.windows.len(), 2);
        assert!((snap.windows[0].used_percent.unwrap() - 8.0).abs() < 1e-9);
        assert_eq!(snap.windows[0].reset_at_ms, Some(1_788_025_458_000));
        assert_eq!(snap.windows[0].window_minutes, Some(300));
        assert_eq!(snap.plan_name.as_deref(), Some("ChatGPT plus"));
        assert!(snap.notes.iter().any(|n| n.contains("1000")));
    }

    #[test]
    fn incremental_append_and_half_line() {
        let (_dir, home) = tmp_home();
        let p = write_rollout(
            &home,
            "rollout-b.jsonl",
            &[meta_line(), turn_line(), token_line(100, 214)],
        );
        let mut entry = FileEntry::default();
        let mut rl = None;
        assert!(advance_file(&p, &mut entry, &mut rl).unwrap());
        assert_eq!(entry.session.totals.total_tokens, 214);

        let mut text = std::fs::read_to_string(&p).unwrap();
        text.push_str(&token_line(200, 314));
        text.push('\n');
        text.push_str(r#"{"timestamp":"2026-08-29T13:46:00.000Z","ordi"#); // half line
        std::fs::write(&p, text).unwrap();
        let before = entry.offset;
        assert!(advance_file(&p, &mut entry, &mut rl).unwrap());
        assert!(entry.offset > before);
        assert!(!entry.complete, "half line must hold back the watermark");
        assert_eq!(
            entry.session.totals.total_tokens, 314,
            "cumulative last-wins"
        );
        assert_eq!(entry.session.responses, 2);

        let mut text = std::fs::read_to_string(&p).unwrap();
        text.push_str(r#"nal":5,"type":"event_msg","payload":{"type":"task_complete"}}"#);
        text.push('\n');
        std::fs::write(&p, text).unwrap();
        assert!(advance_file(&p, &mut entry, &mut rl).unwrap());
        assert!(entry.complete);
        // unchanged file → no growth
        assert!(!advance_file(&p, &mut entry, &mut rl).unwrap());
    }

    #[test]
    fn truncation_rereads_from_start() {
        let (_dir, home) = tmp_home();
        let p = write_rollout(
            &home,
            "rollout-c.jsonl",
            &[meta_line(), token_line(50, 164)],
        );
        let mut entry = FileEntry::default();
        let mut rl = None;
        advance_file(&p, &mut entry, &mut rl).unwrap();
        // Shrink the file (rewrite smaller)
        std::fs::write(&p, meta_line() + "\n").unwrap();
        assert!(advance_file(&p, &mut entry, &mut rl).unwrap());
        assert_eq!(entry.session.totals.total_tokens, 0);
        assert_eq!(entry.session.all_time.total_tokens, 0);
        assert!(entry.session.recent.is_empty());
        assert_eq!(entry.session.session_id, "s1");
    }

    #[test]
    fn missing_home_is_not_installed() {
        let mut p = CodexProvider::new(None);
        p.with_home(PathBuf::from("/nonexistent/codex-home"));
        let snap = p.poll(0);
        assert_eq!(snap.status, ProviderStatus::NotInstalled);
        assert!(snap.error.is_some());
    }

    #[test]
    fn stale_rate_limits_flag() {
        let (_dir, home) = tmp_home();
        write_rollout(
            &home,
            "rollout-d.jsonl",
            &[meta_line(), rate_line(5.0, 1788025458, "plus")],
        );
        let mut p = CodexProvider::new(None);
        p.with_home(home.clone());
        // 10 h after the rate_limits timestamp → stale
        let snap = p.poll(1_788_030_000_000 + 10 * 3600_000);
        assert_eq!(snap.status, ProviderStatus::Stale);
        assert_eq!(snap.windows.len(), 2);
    }

    #[test]
    fn cache_persists_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("codex-cache.json");
        let (_h, home) = tmp_home();
        write_rollout(
            &home,
            "rollout-e.jsonl",
            &[
                meta_line(),
                turn_line(),
                token_line(10, 124),
                rate_line(1.0, 1788025458, "plus"),
            ],
        );
        let mut p = CodexProvider::new(Some(cache_path.clone()));
        p.with_home(home.clone());
        let snap = p.poll(1_788_030_000_000);
        assert_eq!(snap.windows.len(), 2);
        assert!(cache_path.exists());

        // A second provider (fresh process) reloads quota instantly from cache.
        let mut q = CodexProvider::new(Some(cache_path));
        q.with_home(home);
        let snap2 = q.poll(1_788_030_000_000);
        assert_eq!(snap2.windows.len(), 2);
        assert_eq!(snap2.local_usage.unwrap().all_time.total_tokens, 124);
    }

    #[test]
    fn daily_deltas_split_across_midnight() {
        let (_dir, home) = tmp_home();
        // Session spanning two days (24 h apart → different local days in
        // every timezone): day1 +100, day2 cumulative → +80 delta.
        let late_night = r#"{"timestamp":"2026-08-29T12:00:00.000Z","ordinal":3,"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0,"total_tokens":100}}}}"#;
        let after_midnight = r#"{"timestamp":"2026-08-30T12:00:00.000Z","ordinal":4,"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":150,"cached_input_tokens":20,"cache_write_input_tokens":0,"output_tokens":10,"reasoning_output_tokens":0,"total_tokens":180}}}}"#;
        write_rollout(
            &home,
            "rollout-mid.jsonl",
            &[
                meta_line(),
                turn_line(),
                late_night.into(),
                after_midnight.into(),
            ],
        );
        let mut p = CodexProvider::new(None);
        p.with_home(home);
        // "now" = 2026-08-30 14:00 UTC; choose an epoch after day 2's event.
        let now = 1_788_098_400_000i64;
        let snap = p.poll(now);
        let lu = snap.local_usage.unwrap();
        assert_eq!(lu.all_time.total_tokens, 180, "cumulative last-wins");
        // today (day 2) got exactly the second delta: 180-100=80
        assert_eq!(
            lu.today.total_tokens, 80,
            "today bucket = post-midnight delta only"
        );
        // models aggregate from daily deltas → 180 total
        assert_eq!(lu.models[0].breakdown.total_tokens, 180);
    }

    #[test]
    fn garbage_lines_are_skipped() {
        let (_dir, home) = tmp_home();
        write_rollout(
            &home,
            "rollout-f.jsonl",
            &[
                "not json".into(),
                String::new(),
                meta_line(),
                token_line(7, 121),
            ],
        );
        let mut p = CodexProvider::new(None);
        p.with_home(home);
        let snap = p.poll(1_788_030_000_000);
        assert_eq!(snap.local_usage.unwrap().all_time.total_tokens, 121);
    }

    #[test]
    fn six_ranges_use_rolling_boundaries_and_keep_all_time_exact() {
        let now = 1_800_000_000_000i64;
        let events = [
            (now - 30 * 60_000, "sol", 10),
            (now - 2 * 60 * 60_000, "luna", 20),
            (now - 2 * 24 * 60 * 60_000, "sol", 30),
            (now - 10 * 24 * 60 * 60_000, "sol", 40),
            (now - 31 * 24 * 60 * 60_000, "sol", 50),
        ];
        let recent = events
            .iter()
            .map(|(ts_ms, model, value)| UsageEvent {
                ts_ms: *ts_ms,
                model: (*model).into(),
                delta: total(*value),
            })
            .collect();
        let session = SessionUsage {
            session_id: "s1".into(),
            responses: 5,
            all_time: total(150),
            model_totals: HashMap::from([("sol".into(), total(130)), ("luna".into(), total(20))]),
            model_requests: HashMap::from([("sol".into(), 4), ("luna".into(), 1)]),
            recent,
            ..Default::default()
        };
        let files = HashMap::from([(
            "rollout.jsonl".into(),
            FileEntry {
                session,
                ..Default::default()
            },
        )]);

        let usage = aggregate_local(&files, now);
        assert_eq!(
            usage
                .ranges
                .iter()
                .map(|r| r.key.as_str())
                .collect::<Vec<_>>(),
            RANGE_KEYS
        );
        let value = |key: &str| {
            usage
                .ranges
                .iter()
                .find(|range| range.key == key)
                .unwrap()
                .breakdown
                .total_tokens
        };
        assert_eq!(value("60m"), 10);
        assert_eq!(value("24h"), 30);
        assert_eq!(value("7d"), 60);
        assert_eq!(value("30d"), 100);
        assert_eq!(value("all"), 150);
        assert_eq!(usage.all_time.requests, 5);
        assert_eq!(usage.ranges[1].sessions, 1);
        assert_eq!(usage.ranges[1].models[0].model, "sol");
        assert_eq!(
            usage.ranges[5]
                .models
                .iter()
                .map(|m| m.breakdown.requests)
                .sum::<u64>(),
            5
        );
    }

    #[test]
    fn cumulative_counter_reset_counts_the_new_value_as_delta() {
        let mut session = SessionUsage {
            model: "gpt-5.6-sol".into(),
            ..Default::default()
        };
        let mut rate_limits = None;
        apply_line(
            &token_event_line("2026-08-29T13:43:45Z", 100, 100),
            &mut session,
            &mut rate_limits,
        );
        apply_line(
            &token_event_line("2026-08-29T13:44:45Z", 40, 40),
            &mut session,
            &mut rate_limits,
        );

        assert_eq!(session.responses, 2);
        assert_eq!(session.all_time.total_tokens, 140);
        assert_eq!(session.recent[1].delta.total_tokens, 40);
        assert_eq!(session.model_requests["gpt-5.6-sol"], 2);
    }

    #[test]
    fn token_events_are_attributed_to_the_model_active_at_that_event() {
        let mut session = SessionUsage {
            model: "gpt-5.6-sol".into(),
            ..Default::default()
        };
        let mut rate_limits = None;
        apply_line(
            &token_event_line("2026-08-29T13:43:45Z", 100, 100),
            &mut session,
            &mut rate_limits,
        );
        session.model = "gpt-5.6-luna".into();
        apply_line(
            &token_event_line("2026-08-29T13:44:45Z", 150, 150),
            &mut session,
            &mut rate_limits,
        );

        assert_eq!(session.model_totals["gpt-5.6-sol"].total_tokens, 100);
        assert_eq!(session.model_totals["gpt-5.6-luna"].total_tokens, 50);
        assert_eq!(session.model_requests["gpt-5.6-sol"], 1);
        assert_eq!(session.model_requests["gpt-5.6-luna"], 1);
    }

    #[test]
    fn old_cache_schema_is_rebuilt_and_persisted_as_v2() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("codex-cache.json");
        std::fs::write(&cache_path, r#"{"schema_version":1,"files":{}}"#).unwrap();
        let (_home_dir, home) = tmp_home();

        let mut provider = CodexProvider::new(Some(cache_path.clone()));
        assert_eq!(provider.cache.schema_version, CODEX_CACHE_SCHEMA_VERSION);
        assert!(provider.cache_needs_persist);
        provider.with_home(home).poll(1_800_000_000_000);

        let persisted: CodexCache =
            serde_json::from_str(&std::fs::read_to_string(cache_path).unwrap()).unwrap();
        assert_eq!(persisted.schema_version, CODEX_CACHE_SCHEMA_VERSION);
    }

    #[test]
    fn recent_events_are_pruned_without_changing_all_time_totals() {
        let now = 1_800_000_000_000i64;
        let session = SessionUsage {
            all_time: total(60),
            recent: vec![
                UsageEvent {
                    ts_ms: now - 31 * 24 * 60 * 60_000,
                    model: "sol".into(),
                    delta: total(20),
                },
                UsageEvent {
                    ts_ms: now - 2 * 24 * 60 * 60_000,
                    model: "sol".into(),
                    delta: total(40),
                },
            ],
            ..Default::default()
        };
        let mut files = HashMap::from([(
            "rollout.jsonl".into(),
            FileEntry {
                session,
                ..Default::default()
            },
        )]);

        assert!(prune_recent(&mut files, now));
        assert_eq!(files["rollout.jsonl"].session.recent.len(), 1);
        assert_eq!(files["rollout.jsonl"].session.all_time.total_tokens, 60);
    }

    #[test]
    fn jwt_claims_decode() {
        use base64::Engine;
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let claims =
            br#"{"email":"a@b.c","https://api.openai.com/auth":{"chatgpt_plan_type":"pro"}}"#;
        let token = format!("header.{}.signature", engine.encode(claims));
        let v = decode_jwt_claims(&token);
        assert_eq!(v.get("email").and_then(|e| e.as_str()), Some("a@b.c"));
        assert_eq!(
            v.get("https://api.openai.com/auth")
                .and_then(|a| a.get("chatgpt_plan_type"))
                .and_then(|p| p.as_str()),
            Some("pro")
        );
        assert!(decode_jwt_claims("garbage").is_null());
        assert!(decode_jwt_claims("a.b").is_null());
    }

    #[test]
    fn freshest_rate_limits_wins_across_files() {
        let (_dir, home) = tmp_home();
        write_rollout(
            &home,
            "rollout-g1.jsonl",
            &[meta_line(), rate_line(10.0, 1788025458, "plus")],
        );
        let newer = rate_line(30.0, 1788025999, "pro").replace("13:44:00", "13:50:00");
        write_rollout(&home, "rollout-g2.jsonl", &[meta_line(), newer]);
        let mut p = CodexProvider::new(None);
        p.with_home(home);
        let snap = p.poll(1_788_030_000_000);
        let five = snap.windows.iter().find(|w| w.key == "5h").unwrap();
        assert!((five.used_percent.unwrap() - 30.0).abs() < 1e-9);
        assert_eq!(snap.plan_name.as_deref(), Some("ChatGPT pro"));
    }
}
