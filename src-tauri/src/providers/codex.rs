//! OpenAI Codex provider — 100% local official-client data.
//!
//! Data source: the Codex CLI's own session rollouts
//! `<CODEX_HOME>/sessions/YYYY/MM/DD/rollout-*.jsonl` (append-only JSONL),
//! plus everything the CLI moved to `<CODEX_HOME>/archived_sessions/` —
//! archived history is still real local usage and is scanned with the same
//! incremental watermarks. Each line is `{timestamp, ordinal, type, payload}`;
//! we consume:
//! - `session_meta` → session id / start time / `cwd` (workspace path),
//! - `turn_context` → model name,
//! - `event_msg{type:"user_message"}` / `response_item{role:"user"}` → the
//!     first real user input, used as the session title (same convention as
//!     the ZCode session table: generated summary or first user input),
//! - `event_msg{type:"token_count"}` →
//!     `payload.info.total_token_usage` (cumulative per session — the LAST
//!     event per file is the session total, no double counting), and
//!     `payload.rate_limits` (the official quota the Codex backend pushed:
//!     5-hour window + weekly usage percent, reset timestamps, credits,
//!     plan type).
//!
//! Every counter delta is also appended to the file's `records` history in
//! the shared [`crate::zcode::usage::UsageRecord`] schema (see
//! `local_usage::CODEX_DELTA`), which is what feeds the multi-source
//! Sessions page and the per-source dashboards.
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

use super::local_usage::{
    aggregate_local, delta_record, FileEntry, SessionUsage, TotalTokenUsage, CODEX_DELTA,
};
use super::{ProviderSnapshot, ProviderStatus};

// v4: per-record `duration_ms` derived from event timestamps (input item →
// last output item) — older caches hold records without it and are only
// appended to by byte watermark, so they must be rebuilt, not reused.
const CODEX_CACHE_SCHEMA_VERSION: u32 = 4;

// ---------------------------------------------------------------------------
// Wire types (subset of Codex's rollout schema)
// ---------------------------------------------------------------------------

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

    /// All per-file session accumulators (for the multi-source session
    /// index). Files without usage are filtered by the index, not here.
    pub fn sessions(&self) -> impl Iterator<Item = &SessionUsage> {
        self.cache.files.values().map(|entry| &entry.session)
    }

    /// Session contributions for the unified index.
    pub fn contribs(&self) -> Vec<super::session_index::SessionContrib<'_>> {
        self.sessions()
            .map(super::session_index::SessionContrib::from_usage)
            .collect()
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

        let (email, _jwt_plan) = read_account_claims(&self.home.join("auth.json"));
        if let Some(email) = &email {
            snap.account = Some(email.clone());
        } else {
            snap.status = ProviderStatus::NotConfigured;
            snap.error = Some("Codex 未登录(无 auth.json)".into());
        }

        let mut files = Vec::new();
        collect_jsonl(&self.home.join("sessions"), &mut files);
        collect_jsonl(&self.home.join("archived_sessions"), &mut files);
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
            // Rollouts without a session_meta line (truncated archives) keep
            // the file stem as the honest id — never an invented one.
            // Resolved before the entry borrow so a failed read can still
            // drop the key from the cache below.
            let needs_id = self
                .cache
                .files
                .get(&key)
                .map(|e| e.session.session_id.is_empty())
                .unwrap_or(true);
            let entry = self.cache.files.entry(key).or_default();
            if needs_id {
                entry.session.session_id = fallback_session_id(path);
            }
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

        snap.local_usage = Some(aggregate_local(
            self.cache.files.values().map(|entry| &entry.session),
            now_ms,
        ));

        // 官方套餐额度(rate_limits)不再展示:本应用只统计本地 token 用量。
        // 解析仍保留在缓存里,便于未来需要时恢复,不影响任何当前路径。
        let _ = &self.cache.last_rate_limits;

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

/// Session id fallback from the file name: rollout files embed the session
/// id in their stem (`rollout-<ts>-<uuid>.jsonl`); the stem is real on-disk
/// data, just less pretty than the `session_meta` id.
fn fallback_session_id(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "codex-session".to_string())
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
    let source_file = path.to_string_lossy().into_owned();
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
        apply_line(&line, &mut entry.session, best_rl, &source_file);
    }
    entry.offset = offset;
    entry.complete = entry.offset == size;
    Ok(grew)
}

fn apply_line(
    line: &str,
    session: &mut SessionUsage,
    best_rl: &mut Option<(i64, RateLimits)>,
    source_file: &str,
) {
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
            if session.project_path.is_none() {
                session.project_path = p
                    .get("cwd")
                    .and_then(|c| c.as_str())
                    .map(str::to_string);
            }
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
        Some("response_item") => {
            // First user input as the session title — the same convention as
            // the ZCode session table (generated summary or first user
            // input). Real text only; nothing is invented.
            if session.title.is_none() {
                session.title = first_user_text(&v["payload"]);
            }
            // Request boundaries for the approximate duration. Usage events
            // are flushed after the turn's tool calls, so they are useless
            // as an end marker; the *output* items are written as the model
            // finishes streaming.
            if ts_ms > 0 {
                if is_request_input(&v["payload"]) {
                    if session.pending_input_ms.is_none() {
                        session.pending_input_ms = Some(ts_ms);
                    }
                } else {
                    session.pending_output_ms = Some(ts_ms);
                }
            }
        }
        Some("event_msg") => {
            let p = &v["payload"];
            if p.get("type").and_then(|t| t.as_str()) == Some("user_message") {
                if session.title.is_none() {
                    session.title = p
                        .get("message")
                        .and_then(|m| m.as_str())
                        .and_then(|s| normalize_title(s));
                }
            }
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
                        // Approximate request window: from the input item that
                        // started the request to the last streamed output
                        // item. Windows outside [100 ms, 10 min] are not
                        // measurements (timestamp artifacts / stale anchors).
                        let end = session.pending_output_ms.unwrap_or(ts_ms);
                        let duration_ms = match session.pending_input_ms {
                            Some(start) if end > start => Some((end - start) as u64),
                            _ => None,
                        }
                        .filter(|d| {
                            (MIN_DERIVED_REQUEST_MS..=MAX_DERIVED_REQUEST_MS).contains(d)
                        });
                        session.pending_input_ms = None;
                        session.pending_output_ms = None;
                        session.records.push(delta_record(
                            &delta,
                            CODEX_DELTA,
                            ts_ms,
                            &model,
                            &session.session_id,
                            session.project_path.as_deref(),
                            source_file,
                            duration_ms,
                        ));
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

/// User text from a `response_item` message payload (content is a list of
/// typed parts; only plain text counts). Synthetic / XML-ish command bodies
/// are skipped — they are not a human-written session topic.
fn first_user_text(payload: &serde_json::Value) -> Option<String> {
    if payload.get("type").and_then(|t| t.as_str()) != Some("message") {
        return None;
    }
    if payload.get("role").and_then(|r| r.as_str()) != Some("user") {
        return None;
    }
    let text = payload
        .get("content")?
        .as_array()?
        .iter()
        .filter_map(|part| {
            let t = part.get("type").and_then(|t| t.as_str())?;
            if t == "text" || t == "input_text" {
                part.get("text").and_then(|x| x.as_str())
            } else {
                None
            }
        })
        .find(|s| !s.trim().is_empty())?;
    normalize_title(text)
}

/// Trim / sanity-cap a candidate title. Truncation is presentation, not
/// fabrication — the full text stays available in the source file.
fn normalize_title(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() || s.starts_with('<') {
        return None;
    }
    let mut out = String::from(s);
    if out.chars().count() > 300 {
        out = out.chars().take(300).collect();
    }
    Some(out)
}

/// A response_item that feeds the *next* model request: user messages and
/// tool-call outputs. Assistant-side items (messages, reasoning,
/// function_call, …) are response output and land near the response end, so
/// they must not anchor the request start.
fn is_request_input(payload: &serde_json::Value) -> bool {
    let kind = payload.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if kind == "message" {
        return payload.get("role").and_then(|r| r.as_str()) == Some("user");
    }
    // function_call_output / custom_tool_call_output / local_shell_call_output …
    kind.ends_with("_output") && kind != "message_output"
}

/// Accepted window for the timestamp-derived duration. Below the floor the
/// window is a timestamp-resolution artifact (usage events land a few ms
/// after a fast tool output); above the ceiling it cannot be a single
/// request — it is a stale anchor, e.g. a session resumed long after its
/// last input item. Windows outside the range yield no duration at all.
const MIN_DERIVED_REQUEST_MS: u64 = 100;
const MAX_DERIVED_REQUEST_MS: u64 = 10 * 60_000;

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
    target.add(delta);
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
    fn derives_approximate_duration_from_event_timestamps() {
        // Real event shape (2026-09-10 rollout): the request starts with its
        // input item and the model's streamed output items land at the end;
        // the usage event is flushed afterwards (possibly after a tool ran),
        // so only output items can bound the request. TTFT stays None.
        let (_dir, home) = tmp_home();
        let item = |ts: &str, payload: serde_json::Value| {
            serde_json::json!({ "timestamp": ts, "type": "response_item", "payload": payload })
                .to_string()
        };
        let user_input = item("2026-08-29T13:43:38.000Z", serde_json::json!({
            "type": "message", "role": "user",
            "content": [{ "type": "input_text", "text": "继续重构" }]
        }));
        let assistant_out = item("2026-08-29T13:43:44.900Z", serde_json::json!({
            "type": "message", "role": "assistant",
            "content": [{ "type": "output_text", "text": "…" }]
        }));
        let tool_output = item("2026-08-29T13:43:50.000Z", serde_json::json!({
            "type": "function_call_output", "call_id": "c1", "output": "ok"
        }));
        let reasoning_out = item("2026-08-29T13:43:52.500Z", serde_json::json!({
            "type": "reasoning", "summary": []
        }));
        let usage = |ts: &str, input: u64, out: u64, reasoning: u64, total: u64| {
            serde_json::json!({
                "timestamp": ts,
                "type": "event_msg",
                "payload": { "type": "token_count", "info": { "total_token_usage": {
                    "input_tokens": input, "cached_input_tokens": 800,
                    "cache_write_input_tokens": 0, "output_tokens": out,
                    "reasoning_output_tokens": reasoning, "total_tokens": total } } }
            })
            .to_string()
        };
        write_rollout(
            &home,
            "rollout-dur.jsonl",
            &[
                meta_line(),
                turn_line(),
                user_input,
                // Request 1: input 13:43:38.000 → last output 13:43:44.900
                // = 6 900 ms. The usage event lands 827 ms later (tool flush)
                // and must not extend the window.
                assistant_out,
                usage("2026-08-29T13:43:45.727Z", 1000, 100, 14, 1114),
                // Request 2: tool output 13:43:50.000 → reasoning 13:43:52.500
                // = 2 500 ms.
                tool_output,
                reasoning_out,
                usage("2026-08-29T13:43:53.000Z", 900, 120, 30, 1020),
                // Request 3: a 10 ms window is a timestamp artifact → no
                // duration is recorded (and thus no speed sample).
                item("2026-08-29T13:43:56.000Z", serde_json::json!({
                    "type": "function_call_output", "call_id": "c2", "output": "ok"
                })),
                item("2026-08-29T13:43:56.010Z", serde_json::json!({
                    "type": "message", "role": "assistant",
                    "content": [{ "type": "output_text", "text": "ok" }]
                })),
                usage("2026-08-29T13:43:57.000Z", 950, 130, 30, 1080),
                // Request 4: a stale anchor (session resumed long after its
                // last input) would produce an hours-long window → dropped.
                item("2026-08-29T14:05:00.000Z", serde_json::json!({
                    "type": "message", "role": "assistant",
                    "content": [{ "type": "output_text", "text": "晚些时候继续" }]
                })),
                usage("2026-08-29T14:05:01.000Z", 990, 140, 30, 1130),
            ],
        );
        let mut entry = FileEntry::default();
        let mut rl = None;
        let p = home.join("sessions/2026/08/29").join("rollout-dur.jsonl");
        assert!(advance_file(&p, &mut entry, &mut rl).unwrap());
        let recs = &entry.session.records;
        assert_eq!(recs.len(), 4);
        assert_eq!(recs[0].duration_ms, Some(6_900));
        assert_eq!(recs[0].ttft_ms, None);
        assert_eq!(recs[1].duration_ms, Some(2_500));
        assert_eq!(recs[2].duration_ms, None, "sub-100ms windows are dropped");
        assert_eq!(recs[3].duration_ms, None, "over-10min windows are dropped");

        let speed = crate::zcode::aggregate::compute_speed_stats(recs);
        assert_eq!(speed.ttft_samples, 0);
        assert_eq!(speed.speed_samples, 2);
        assert!(speed.speed_approximate);
        // Cumulative counters: request 1 delta = 100+14, request 2 delta =
        // (120−100)+(30−14) = 36, request 3 is not a sample.
        // 150 tokens over 9 400 ms.
        assert!((speed.speed_tps.unwrap() - 150.0 / 9.4).abs() < 1e-6);
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
        assert!(entry.session.records.is_empty());
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
        assert!(cache_path.exists());

        // A second provider (fresh process) reloads quota instantly from cache.
        let mut q = CodexProvider::new(Some(cache_path));
        q.with_home(home);
        let snap2 = q.poll(1_788_030_000_000);
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
            "f.jsonl",
        );
        apply_line(
            &token_event_line("2026-08-29T13:44:45Z", 40, 40),
            &mut session,
            &mut rate_limits,
            "f.jsonl",
        );

        assert_eq!(session.responses, 2);
        assert_eq!(session.all_time.total_tokens, 140);
        assert_eq!(session.records[1].display_total_tokens(), 40);
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
            "f.jsonl",
        );
        session.model = "gpt-5.6-luna".into();
        apply_line(
            &token_event_line("2026-08-29T13:44:45Z", 150, 150),
            &mut session,
            &mut rate_limits,
            "f.jsonl",
        );

        assert_eq!(session.model_totals["gpt-5.6-sol"].total_tokens, 100);
        assert_eq!(session.model_totals["gpt-5.6-luna"].total_tokens, 50);
        assert_eq!(session.model_requests["gpt-5.6-sol"], 1);
        assert_eq!(session.model_requests["gpt-5.6-luna"], 1);
    }

    #[test]
    fn old_cache_schema_is_rebuilt_and_persisted_as_v3() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("codex-cache.json");
        std::fs::write(&cache_path, r#"{"schema_version":2,"files":{}}"#).unwrap();
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
    fn full_event_history_is_retained_across_polls() {
        // Session detail views and "all"-range dashboards need the complete
        // record history — nothing is pruned after 30 days anymore.
        let (_dir, home) = tmp_home();
        let old_ts = "2026-01-02T10:00:00.000Z";
        let line = token_event_line(old_ts, 10, 24);
        write_rollout(&home, "rollout-old.jsonl", &[meta_line(), turn_line(), line]);
        let mut p = CodexProvider::new(None);
        p.with_home(home);
        let snap = p.poll(1_800_000_000_000);
        let lu = snap.local_usage.unwrap();
        assert_eq!(lu.all_time.total_tokens, 24);
        let session = p.sessions().next().unwrap();
        assert_eq!(session.records.len(), 1);
        assert_eq!(session.records[0].ts_ms.to_string(), chrono::DateTime::parse_from_rfc3339(old_ts).unwrap().timestamp_millis().to_string());
    }

    #[test]
    fn archived_sessions_are_scanned_and_counted() {
        let (_dir, home) = tmp_home();
        // Active session under sessions/, archived one under archived_sessions/.
        write_rollout(
            &home,
            "rollout-live.jsonl",
            &[meta_line(), turn_line(), token_line(100, 214)],
        );
        let archived = home.join("archived_sessions").join("2026").join("05");
        std::fs::create_dir_all(&archived).unwrap();
        std::fs::write(
            archived.join("rollout-archived.jsonl"),
            [meta_line(), turn_line(), token_line(50, 164)].join("\n") + "\n",
        )
        .unwrap();
        let mut p = CodexProvider::new(None);
        p.with_home(home);
        let snap = p.poll(1_788_030_000_000);
        let lu = snap.local_usage.unwrap();
        assert_eq!(lu.sessions, 2);
        assert_eq!(lu.all_time.total_tokens, 214 + 164);
    }

    #[test]
    fn cwd_and_first_user_message_become_project_and_title() {
        let (_dir, home) = tmp_home();
        let meta_with_cwd = r#"{"timestamp":"2026-08-29T13:43:37.330Z","ordinal":1,"type":"session_meta","payload":{"session_id":"s-cwd","cwd":"D:\\work\\crawler_Xianyu"}}"#;
        let user_line = r#"{"timestamp":"2026-08-29T13:43:38.000Z","ordinal":2,"type":"event_msg","payload":{"type":"user_message","message":"修复反爬限流的重试策略"}}"#;
        write_rollout(
            &home,
            "rollout-titled.jsonl",
            &[meta_with_cwd.into(), user_line.into(), turn_line(), token_line(10, 124)],
        );
        let mut p = CodexProvider::new(None);
        p.with_home(home);
        p.poll(1_788_030_000_000);
        let session = p.sessions().next().unwrap();
        assert_eq!(session.title.as_deref(), Some("修复反爬限流的重试策略"));
        assert_eq!(session.project_path.as_deref(), Some("D:\\work\\crawler_Xianyu"));
        // The record carries the workspace too (session-level context).
        assert_eq!(session.records[0].project.as_deref(), Some("D:\\work\\crawler_Xianyu"));
    }

    #[test]
    fn rollout_without_meta_gets_file_stem_as_session_id() {
        let (_dir, home) = tmp_home();
        write_rollout(
            &home,
            "rollout-2026-08-29T13-43-37-ab12cd34.jsonl",
            &[turn_line(), token_line(5, 119)],
        );
        let mut p = CodexProvider::new(None);
        p.with_home(home);
        p.poll(1_788_030_000_000);
        let session = p.sessions().next().unwrap();
        assert_eq!(
            session.session_id,
            "rollout-2026-08-29T13-43-37-ab12cd34"
        );
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

}
