//! DeepSeek Harness (DSH) provider — 100% local session-log data.
//!
//! Data source: the DeepSeek Harness CLI's own session logs under
//! `<DSH_HOME>/sessions/` (`~/.dsh` by default, `$DSH_HOME` overrides, and
//! the panel's own setting wins over both). Logs are one append-only JSONL
//! event stream per session, stored either as raw `.jsonl` lines or as
//! zstd-framed `.jsonl.zstd` (the DSH default).
//!
//! Event envelope (per the official persistence catalog):
//! `{ "type": <kind>, "seq": <monotonic int>, "time": <unix epoch ms>,
//!    "data": { … } }`. We consume:
//! - `request/header`  → `data.header.config.{provider, model}` (call config,
//!   and `cwd` when the harness records it — probed defensively, never
//!   invented),
//! - `model/selection` → current model when the header shape differs,
//! - `assistant/message` → `data.usage` (TokenUsage:
//!   `inputTokens` = uncached input only, `outputTokens` already contains
//!   `reasoningTokens`, `cacheReadTokens?`, `cacheWriteTokens?`).
//!
//! Conventions kept honest:
//! - `total_tokens` is derived as input + cacheRead + cacheWrite + output —
//!   the documented disjoint fields. `reasoningTokens` is a subset of output
//!   and is displayed separately, never added again.
//! - Every usage event is also appended to the file's `records` history in
//!   the shared [`crate::zcode::usage::UsageRecord`] schema (see
//!   `local_usage::DSH_DELTA`), powering the multi-source Sessions page and
//!   per-source dashboards with the exact same caliber machinery as ZCode.
//! - No speed/TTFT metrics: DSH stream records carry timing, but their
//!   on-disk shape is not publicly documented, so nothing is derived.
//! - Raw `.jsonl` files advance by byte watermark; framed `.jsonl.zstd`
//!   files are re-decoded only when their size changes and de-duplicated by
//!   the event `seq`. Every unknown event/field is skipped — never a panic,
//!   never cross-provider contamination.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::local_usage::{
    aggregate_local, delta_record, SessionUsage, TotalTokenUsage, DSH_DELTA,
};
use super::{ProviderSnapshot, ProviderStatus};

const DSH_CACHE_SCHEMA_VERSION: u32 = 2;
/// Defensive cap for one framed session log (they are per-session files; a
/// bigger value means we are looking at something that is not a DSH log).
const MAX_FRAMED_FILE: u64 = 64 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Parse cache
// ---------------------------------------------------------------------------

fn default_last_seq() -> i64 {
    // Session events are contiguous from seq 0, so "nothing seen yet" must
    // sort below 0.
    -1
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DshFileEntry {
    /// Last observed file size (change detector + truncation reset).
    #[serde(default)]
    pub size: u64,
    /// Raw-file byte watermark (only complete lines advance it).
    #[serde(default)]
    pub offset: u64,
    /// Framed-file event watermark: events with `seq <= last_seq` are known.
    #[serde(default = "default_last_seq")]
    pub last_seq: i64,
    #[serde(default)]
    pub session: SessionUsage,
}

impl Default for DshFileEntry {
    fn default() -> Self {
        Self {
            size: 0,
            offset: 0,
            last_seq: default_last_seq(),
            session: SessionUsage::default(),
        }
    }
}

/// Persisted parse cache: per-file watermarks. Saved atomically; corrupt
/// files fall back to empty.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DshCache {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    files: HashMap<String, DshFileEntry>,
    #[serde(default)]
    saved_at_ms: i64,
}

impl DshCache {
    fn fresh() -> Self {
        Self {
            schema_version: DSH_CACHE_SCHEMA_VERSION,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

pub struct DshProvider {
    home: PathBuf,
    cache: DshCache,
    cache_path: Option<PathBuf>,
    cache_needs_persist: bool,
}

impl DshProvider {
    pub fn new(cache_path: Option<PathBuf>) -> Self {
        let (cache, cache_needs_persist) = cache_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| {
                let parsed = serde_json::from_str::<DshCache>(&t).ok()?;
                if parsed.schema_version == DSH_CACHE_SCHEMA_VERSION {
                    Some((parsed, false))
                } else {
                    Some((DshCache::fresh(), true))
                }
            })
            .unwrap_or_else(|| (DshCache::fresh(), false));
        Self {
            home: default_home(),
            cache,
            cache_path,
            cache_needs_persist,
        }
    }

    /// Point at a (possibly different) DSH_HOME; a changed root discards
    /// watermarks that belong to another tree.
    pub fn with_home(&mut self, home: PathBuf) -> &mut Self {
        if self.home != home {
            let belongs_to_home = !self.cache.files.is_empty()
                && self
                    .cache
                    .files
                    .keys()
                    .all(|key| Path::new(key).starts_with(&home));
            if !belongs_to_home && !self.cache.files.is_empty() {
                self.cache = DshCache::fresh();
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
        let mut snap = ProviderSnapshot::empty(super::PROVIDER_DSH, ProviderStatus::Ok, now_ms);
        snap.source = "DeepSeek Harness 本地 session 日志(离线读取)".into();
        snap.source_url = Some("https://www.deepseek.com/harness/".into());

        if !self.home.is_dir() {
            snap.status = ProviderStatus::NotInstalled;
            snap.error = Some(
                "未检测到 DeepSeek Harness 数据目录(默认 ~/.dsh;可在「设置 → DSH」指定路径)".into(),
            );
            return snap;
        }

        let sessions_root = self.home.join("sessions");
        let scan_root = if sessions_root.is_dir() { sessions_root } else { self.home.clone() };
        let mut files = Vec::new();
        collect_session_logs(&scan_root, &mut files);
        files.sort();

        let live: std::collections::HashSet<String> = files
            .iter()
            .map(|f| f.to_string_lossy().into_owned())
            .collect();
        let file_count_before = self.cache.files.len();
        self.cache.files.retain(|k, _| live.contains(k));

        let mut changed =
            self.cache_needs_persist || file_count_before != self.cache.files.len();
        for path in &files {
            let key = path.to_string_lossy().into_owned();
            let framed = is_framed(path);
            let entry = self.cache.files.entry(key).or_default();
            if entry.session.session_id.is_empty() {
                entry.session.session_id = session_id_from_path(path);
            }
            match advance_file(path, entry, framed) {
                Ok(grew) => changed |= grew,
                Err(why) => {
                    self.cache
                        .files
                        .remove(&path.to_string_lossy().into_owned());
                    snap.notes
                        .push(format!("跳过无法读取的 session 日志:{why}"));
                }
            }
        }
        self.cache.saved_at_ms = now_ms;

        let has_events = self
            .cache
            .files
            .values()
            .any(|entry| entry.session.responses > 0);
        snap.local_usage = Some(aggregate_local(
            self.cache.files.values().map(|entry| &entry.session),
            now_ms,
        ));

        if !has_events {
            if files.is_empty() {
                snap.status = ProviderStatus::NotConfigured;
                snap.error = Some(
                    "DSH 数据目录存在,但其中没有 session 日志(在 DSH 里发起一次对话后自动出现)".into(),
                );
            } else {
                snap.notes
                    .push("已发现 session 日志,但尚未记录到任何 token usage 事件".into());
            }
        } else {
            snap.notes.push(
                "来自 DSH session 日志的 assistant/message usage 事件;reasoning 已含在 Output 中,总量不重复累计".into(),
            );
        }
        snap.notes.push("session 日志统计 · 不计入 ZCode 总 Token".into());

        if changed {
            self.persist_cache();
            self.cache_needs_persist = false;
        }
        snap
    }
}

pub fn default_home() -> PathBuf {
    if let Some(p) = std::env::var_os("DSH_HOME") {
        let p = PathBuf::from(p);
        if p.is_dir() {
            return p;
        }
    }
    dirs::home_dir()
        .map(|h| h.join(".dsh"))
        .unwrap_or_else(|| PathBuf::from(".dsh"))
}

fn is_framed(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    name.ends_with(".jsonl.zstd") || name.ends_with(".jsonl.zst")
}

fn collect_session_logs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_session_logs(&p, out);
        } else if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".jsonl")
                || lower.ends_with(".jsonl.zstd")
                || lower.ends_with(".jsonl.zst")
            {
                out.push(p);
            }
        }
    }
}

/// Session id from the file layout: DSH stores one log per session inside a
/// per-session directory, so the parent directory name is the id; a flat
/// `sessions/<id>.jsonl` layout falls back to the file stem.
fn session_id_from_path(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    // `.jsonl.zstd` file_stem is `log` — strip the intermediate extension.
    let stem = stem.strip_suffix(".jsonl").unwrap_or(&stem).to_string();
    let parent = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !parent.is_empty() && parent != "sessions" {
        parent
    } else if !stem.is_empty() {
        stem
    } else {
        "dsh-session".to_string()
    }
}

// ---------------------------------------------------------------------------
// File advancing (raw + framed)
// ---------------------------------------------------------------------------

/// Advance one log file past its watermark. Raw files consume complete lines
/// only; framed files are re-decoded when the size changed and de-duplicated
/// by event `seq`.
fn advance_file(path: &Path, entry: &mut DshFileEntry, framed: bool) -> Result<bool, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    if framed && size > MAX_FRAMED_FILE {
        return Err(format!("framed log exceeds {MAX_FRAMED_FILE} bytes (not a session log?)"));
    }
    if size < entry.size || (size < entry.offset && !framed) {
        // Truncated/rewritten → re-read from the start.
        entry.size = 0;
        entry.offset = 0;
        entry.last_seq = default_last_seq();
        entry.session = SessionUsage::default();
    }
    if size == 0 {
        entry.size = 0;
        return Ok(false);
    }
    let source_file = path.to_string_lossy().into_owned();
    if framed {
        if size == entry.size {
            return Ok(false); // unchanged framed log — nothing to decode
        }
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut decoder = zstd::stream::read::Decoder::new(file)
            .map_err(|e| format!("zstd 解码失败:{e}"))?;
        let mut text = String::new();
        decoder
            .read_to_string(&mut text)
            .map_err(|e| format!("zstd 读取失败:{e}"))?;
        let mut grew = false;
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let Some((seq, _)) = parse_event(line) else {
                continue;
            };
            if seq <= entry.last_seq {
                continue; // already counted on a previous poll
            }
            if seq > entry.last_seq {
                entry.last_seq = seq;
            }
            grew = true;
            apply_event_line(line, &mut entry.session, &source_file);
        }
        entry.size = size;
        return Ok(grew);
    }

    // Raw JSONL: byte-watermark incremental read (half lines hold back).
    if size == entry.offset {
        entry.size = size;
        return Ok(false);
    }
    file.seek(SeekFrom::Start(entry.offset))
        .map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(file);
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
        apply_event_line(&line, &mut entry.session, &source_file);
    }
    entry.offset = offset;
    entry.size = size;
    Ok(grew)
}

// ---------------------------------------------------------------------------
// Event parsing (pure, testable)
// ---------------------------------------------------------------------------

/// Extract the event envelope `(seq, ts_ms)` from one JSONL line. Returns
/// `None` for lines that are not session events (garbage, headers, …) or
/// that carry no `seq` — the framed-file watermark cannot de-duplicate
/// seq-less events safely, so they are skipped rather than double-counted.
fn parse_event(line: &str) -> Option<(i64, i64)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let node = if v.get("type").is_some() { &v } else { v.get("event")? };
    if node.get("type").and_then(|t| t.as_str()).is_none() {
        return None;
    }
    let seq = envelope_int(&v, &["seq"])?;
    let ts = envelope_ts(&v);
    Some((seq, ts))
}

fn envelope_int(v: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    for node in [v.get("event"), Some(v)].into_iter().flatten() {
        for key in keys {
            if let Some(n) = node.get(*key).and_then(|x| x.as_i64()) {
                return Some(n);
            }
        }
    }
    None
}

/// Event time: catalog says `time` is unix epoch ms; accept `timestamp`
/// numbers/strings defensively.
fn envelope_ts(v: &serde_json::Value) -> i64 {
    for node in [v.get("event"), Some(v)].into_iter().flatten() {
        if let Some(t) = node.get("time").and_then(|x| x.as_i64()) {
            return t;
        }
        if let Some(t) = node.get("timestamp") {
            if let Some(n) = t.as_i64() {
                return n;
            }
            if let Some(s) = t.as_str() {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                    return dt.timestamp_millis();
                }
                if let Ok(n) = s.trim().parse::<i64>() {
                    return n;
                }
            }
        }
    }
    0
}

fn data_of(v: &serde_json::Value) -> Option<&serde_json::Value> {
    let node = if v.get("type").is_some() { v } else { v.get("event")? };
    node.get("data")
}

fn kind_of(v: &serde_json::Value) -> Option<&str> {
    let node = if v.get("type").is_some() { v } else { v.get("event")? };
    node.get("type").and_then(|t| t.as_str())
}

/// Parse one raw token-usage object (DSH TokenUsage field names).
fn parse_usage(v: &serde_json::Value) -> Option<TotalTokenUsage> {
    let num = |key: &str| v.get(key).and_then(|x| x.as_u64());
    let input = num("inputTokens");
    let output = num("outputTokens");
    if input.is_none() && output.is_none() {
        return None;
    }
    let input = input.unwrap_or(0);
    let output = output.unwrap_or(0);
    let cache_read = num("cacheReadTokens").unwrap_or(0);
    let cache_write = num("cacheWriteTokens").unwrap_or(0);
    let reasoning = num("reasoningTokens").unwrap_or(0);
    Some(TotalTokenUsage {
        input_tokens: input,
        cached_input_tokens: cache_read,
        cache_write_input_tokens: cache_write,
        output_tokens: output,
        reasoning_output_tokens: reasoning,
        // Documented disjoint fields; reasoning is inside output and must not
        // be added again.
        total_tokens: input
            .saturating_add(cache_read)
            .saturating_add(cache_write)
            .saturating_add(output),
    })
}

/// Model id from a `request/header` payload (`data.header.config.model`) or a
/// `model/selection` payload (shape not frozen — probe common paths).
fn model_from_payload(data: &serde_json::Value) -> Option<String> {
    const PATHS: &[&[&str]] = &[
        &["header", "config", "model"],
        &["header", "model"],
        &["config", "model"],
        &["model"],
        &["modelId"],
        &["selection", "model"],
        &["selection", "modelId"],
        &["selection", "model", "modelId"],
    ];
    for path in PATHS {
        if let Some(m) = data
            .pointer(&path.iter().map(|s| format!("/{s}")).collect::<String>())
            .and_then(|x| x.as_str())
            .filter(|m| !m.is_empty())
        {
            return Some(m.to_string());
        }
    }
    None
}

/// Workspace path from a header-ish payload. The DSH catalog does not
/// document a `cwd` field; common shapes are probed defensively and the
/// value is used only when the harness genuinely recorded one.
fn cwd_from_payload(data: &serde_json::Value) -> Option<String> {
    const PATHS: &[&[&str]] = &[
        &["header", "cwd"],
        &["header", "config", "cwd"],
        &["config", "cwd"],
        &["cwd"],
        &["header", "workspace"],
    ];
    for path in PATHS {
        if let Some(c) = data
            .pointer(&path.iter().map(|s| format!("/{s}")).collect::<String>())
            .and_then(|x| x.as_str())
            .filter(|c| !c.trim().is_empty())
        {
            return Some(c.to_string());
        }
    }
    None
}

/// Apply one JSONL line to the session accumulator (pure, no I/O).
fn apply_event_line(line: &str, session: &mut SessionUsage, source_file: &str) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return;
    };
    let Some(kind) = kind_of(&v) else {
        return;
    };
    let ts_ms = envelope_ts(&v);
    let data = data_of(&v).cloned().unwrap_or(serde_json::Value::Null);

    match kind {
        "request/header" | "model/selection" => {
            if let Some(model) = model_from_payload(&data) {
                session.model = model;
            }
            if session.project_path.is_none() {
                session.project_path = cwd_from_payload(&data);
            }
        }
        "assistant/message" => {
            let usage_node = data.get("usage").or_else(|| v.get("usage"));
            let Some(usage) = usage_node.and_then(parse_usage) else {
                return;
            };
            let model = if session.model.is_empty() {
                "unknown".to_string()
            } else {
                session.model.clone()
            };
            session.responses += 1;
            session.all_time.add(&usage);
            session
                .model_totals
                .entry(model.clone())
                .or_default()
                .add(&usage);
            *session.model_requests.entry(model.clone()).or_insert(0) += 1;
            session.records.push(delta_record(
                &usage,
                DSH_DELTA,
                ts_ms,
                &model,
                &session.session_id,
                session.project_path.as_deref(),
                source_file,
            ));
            if session.first_ts_ms == 0 || (ts_ms > 0 && ts_ms < session.first_ts_ms) {
                session.first_ts_ms = ts_ms;
            }
            if ts_ms > session.last_ts_ms {
                session.last_ts_ms = ts_ms;
            }
        }
        _ => {}
    }
}

/// Public helper for tests and future diagnostics: build a session summary
/// from raw lines with a known session id.
pub fn session_from_lines(session_id: &str, lines: &[String]) -> SessionUsage {
    let mut session = SessionUsage {
        session_id: session_id.to_string(),
        ..Default::default()
    };
    for line in lines {
        apply_event_line(line, &mut session, "test");
    }
    session
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_home() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join(".dsh");
        std::fs::create_dir_all(home.join("sessions").join("sess-abc")).unwrap();
        (dir, home)
    }

    fn header_event(seq: i64, time: i64, model: &str) -> String {
        serde_json::json!({
            "type": "request/header",
            "seq": seq,
            "time": time,
            "data": { "header": { "config": { "provider": "deepseek", "model": model } } }
        })
        .to_string()
    }

    fn selection_event(seq: i64, time: i64, model: &str) -> String {
        serde_json::json!({
            "type": "model/selection",
            "seq": seq,
            "time": time,
            "data": { "selection": { "modelId": model } }
        })
        .to_string()
    }

    fn assistant_event(seq: i64, time: i64, input: u64, output: u64, reasoning: u64) -> String {
        serde_json::json!({
            "type": "assistant/message",
            "seq": seq,
            "time": time,
            "data": {
                "usage": {
                    "inputTokens": input,
                    "outputTokens": output,
                    "reasoningTokens": reasoning,
                    "cacheReadTokens": 100,
                    "cacheWriteTokens": 40
                }
            }
        })
        .to_string()
    }

    fn write_raw(home: &Path, session: &str, lines: &[String]) -> PathBuf {
        let p = home.join("sessions").join(session).join("log.jsonl");
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(&p, lines.join("\n") + "\n").unwrap();
        p
    }

    fn write_framed(home: &Path, session: &str, lines: &[String]) -> PathBuf {
        let p = home.join("sessions").join(session).join("log.jsonl.zstd");
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        let raw = lines.join("\n") + "\n";
        let mut encoder = zstd::stream::Encoder::new(Vec::new(), 3).unwrap();
        use std::io::Write;
        encoder.write_all(raw.as_bytes()).unwrap();
        let compressed = encoder.finish().unwrap();
        std::fs::write(&p, compressed).unwrap();
        p
    }

    const T0: i64 = 1_800_000_000_000;

    #[test]
    fn parses_usage_and_keeps_reasoning_out_of_totals() {
        let session = session_from_lines(
            "sess-abc",
            &[
                header_event(0, T0, "deepseek-chat"),
                assistant_event(1, T0 + 4_000, 1000, 500, 200),
            ],
        );
        assert_eq!(session.responses, 1);
        // total = input + cacheRead + cacheWrite + output (reasoning ⊂ output)
        assert_eq!(session.all_time.total_tokens, 1000 + 100 + 40 + 500);
        assert_eq!(session.all_time.output_tokens, 500);
        assert_eq!(session.all_time.reasoning_output_tokens, 200);
        assert_eq!(session.all_time.cached_input_tokens, 100);
        assert_eq!(session.model_requests["deepseek-chat"], 1);
        assert_eq!(session.records[0].ts_ms, T0 + 4_000);
        // Shared-schema record keeps the same caliber + nested reasoning.
        assert_eq!(session.records[0].display_total_tokens(), 1000 + 100 + 40 + 500);
        assert_eq!(session.records[0].generated_tokens(), 500);
        assert_eq!(session.records[0].cache_read_tokens, Some(100));
    }

    #[test]
    fn cwd_in_header_becomes_project_path() {
        let with_cwd = serde_json::json!({
            "type": "request/header",
            "seq": 0,
            "time": T0,
            "data": { "header": { "config": { "model": "deepseek-chat" }, "cwd": "D:\\work\\panel" } }
        })
        .to_string();
        let session = session_from_lines(
            "sess-abc",
            &[with_cwd, assistant_event(1, T0 + 1_000, 10, 5, 0)],
        );
        assert_eq!(session.project_path.as_deref(), Some("D:\\work\\panel"));
        assert_eq!(session.records[0].project.as_deref(), Some("D:\\work\\panel"));
        // Headers without a cwd field stay honestly empty.
        let bare = session_from_lines(
            "sess-def",
            &[header_event(0, T0, "deepseek-chat"), assistant_event(1, T0, 10, 5, 0)],
        );
        assert_eq!(bare.project_path, None);
        assert_eq!(bare.records[0].project, None);
    }

    #[test]
    fn model_switches_are_attributed_per_event() {
        let session = session_from_lines(
            "s",
            &[
                header_event(0, T0, "deepseek-chat"),
                assistant_event(1, T0 + 1_000, 100, 50, 0),
                selection_event(2, T0 + 2_000, "deepseek-reasoner"),
                assistant_event(3, T0 + 3_000, 200, 80, 60),
            ],
        );
        assert_eq!(session.model_requests["deepseek-chat"], 1);
        assert_eq!(session.model_requests["deepseek-reasoner"], 1);
        assert_eq!(session.records[1].model, "deepseek-reasoner");
    }

    #[test]
    fn poll_reads_raw_and_framed_logs() {
        let (_dir, home) = tmp_home();
        write_raw(
            &home,
            "sess-abc",
            &[header_event(0, T0, "deepseek-chat"), assistant_event(1, T0 + 1_000, 100, 50, 0)],
        );
        write_framed(
            &home,
            "sess-def",
            &[header_event(0, T0, "deepseek-reasoner"), assistant_event(1, T0 + 2_000, 300, 90, 40)],
        );
        let mut p = DshProvider::new(None);
        p.with_home(home.clone());
        let snap = p.poll(T0 + 60_000);
        assert_eq!(snap.status, ProviderStatus::Ok);
        let lu = snap.local_usage.unwrap();
        assert_eq!(lu.all_time.requests, 2);
        assert_eq!(lu.all_time.total_tokens, (100 + 100 + 40 + 50) + (300 + 100 + 40 + 90));
        assert_eq!(lu.sessions, 2);
        let models: Vec<&str> = lu.models.iter().map(|m| m.model.as_str()).collect();
        assert!(models.contains(&"deepseek-chat"));
        assert!(models.contains(&"deepseek-reasoner"));
    }

    #[test]
    fn framed_incremental_uses_seq_watermark() {
        let (_dir, home) = tmp_home();
        let p = write_framed(
            &home,
            "sess-abc",
            &[header_event(0, T0, "deepseek-chat"), assistant_event(1, T0, 10, 5, 0)],
        );
        let mut entry = DshFileEntry::default();
        assert!(advance_file(&p, &mut entry, true).unwrap());
        assert_eq!(entry.session.all_time.total_tokens, 10 + 100 + 40 + 5);

        // Rewrite the file with one MORE event (same prefix).
        write_framed(
            &home,
            "sess-abc",
            &[
                header_event(0, T0, "deepseek-chat"),
                assistant_event(1, T0, 10, 5, 0),
                assistant_event(2, T0 + 5_000, 7, 3, 0),
            ],
        );
        assert!(advance_file(&p, &mut entry, true).unwrap());
        assert_eq!(entry.session.responses, 2, "only the new event is counted");
        assert_eq!(entry.session.all_time.total_tokens, 10 + 100 + 40 + 5 + 7 + 100 + 40 + 3);
        assert_eq!(entry.last_seq, 2);

        // Unchanged file → no growth.
        assert!(!advance_file(&p, &mut entry, true).unwrap());
    }

    #[test]
    fn raw_incremental_and_half_line() {
        let (_dir, home) = tmp_home();
        let p = write_raw(
            &home,
            "sess-abc",
            &[header_event(0, T0, "deepseek-chat"), assistant_event(1, T0, 10, 5, 0)],
        );
        let mut entry = DshFileEntry::default();
        assert!(advance_file(&p, &mut entry, false).unwrap());
        assert_eq!(entry.session.responses, 1);

        // Append HALF of a new event line — must not error, must not count.
        let full_line = serde_json::json!({
            "type": "assistant/message",
            "seq": 2,
            "time": T0 + 6_000,
            "data": { "usage": { "inputTokens": 1, "outputTokens": 2 } }
        })
        .to_string();
        let cut = 20.min(full_line.len());
        let mut text = std::fs::read_to_string(&p).unwrap();
        text.push_str(&full_line[..cut]);
        std::fs::write(&p, text).unwrap();
        assert!(!advance_file(&p, &mut entry, false).unwrap());
        assert_eq!(entry.session.responses, 1);

        // Complete the line — now it appears.
        let mut text = std::fs::read_to_string(&p).unwrap();
        text.push_str(&full_line[cut..]);
        text.push('\n');
        std::fs::write(&p, text).unwrap();
        assert!(advance_file(&p, &mut entry, false).unwrap());
        assert_eq!(entry.session.responses, 2);
        assert_eq!(entry.session.all_time.total_tokens, 10 + 100 + 40 + 5 + 1 + 2);
    }

    #[test]
    fn truncation_resets_state() {
        let (_dir, home) = tmp_home();
        let p = write_raw(
            &home,
            "sess-abc",
            &[header_event(0, T0, "deepseek-chat"), assistant_event(1, T0, 10, 5, 0)],
        );
        let mut entry = DshFileEntry::default();
        advance_file(&p, &mut entry, false).unwrap();
        assert_eq!(entry.session.responses, 1);

        std::fs::write(&p, "").unwrap();
        advance_file(&p, &mut entry, false).unwrap();
        assert_eq!(entry.session.responses, 0);
        assert_eq!(entry.offset, 0);
    }

    #[test]
    fn garbage_and_unknown_events_are_skipped() {
        let session = session_from_lines(
            "s",
            &[
                "not json".into(),
                serde_json::json!({"type":"turn/start","seq":0,"time":T0,"data":{}}).to_string(),
                serde_json::json!({"type":"assistant/message","seq":1,"time":T0,"data":{}}).to_string(),
                serde_json::json!({"noType":true}).to_string(),
                assistant_event(2, T0, 8, 4, 0),
            ],
        );
        assert_eq!(session.responses, 1);
        assert_eq!(session.all_time.total_tokens, 8 + 100 + 40 + 4);
    }

    #[test]
    fn missing_home_is_not_installed() {
        let mut p = DshProvider::new(None);
        p.with_home(PathBuf::from("/nonexistent/dsh-home"));
        let snap = p.poll(0);
        assert_eq!(snap.status, ProviderStatus::NotInstalled);
        assert!(snap.error.is_some());
    }

    #[test]
    fn empty_sessions_dir_is_not_configured() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join(".dsh");
        std::fs::create_dir_all(home.join("sessions")).unwrap();
        let mut p = DshProvider::new(None);
        p.with_home(home);
        let snap = p.poll(0);
        assert_eq!(snap.status, ProviderStatus::NotConfigured);
    }

    #[test]
    fn cache_persists_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("dsh-usage-cache.json");
        let (_h, home) = tmp_home();
        write_raw(
            &home,
            "sess-abc",
            &[header_event(0, T0, "deepseek-chat"), assistant_event(1, T0, 10, 5, 0)],
        );
        let mut p = DshProvider::new(Some(cache_path.clone()));
        p.with_home(home.clone());
        let snap = p.poll(T0 + 1_000);
        assert_eq!(snap.local_usage.unwrap().all_time.requests, 1);
        assert!(cache_path.exists());

        // A fresh provider reloads without re-counting.
        let mut q = DshProvider::new(Some(cache_path));
        q.with_home(home);
        let snap2 = q.poll(T0 + 2_000);
        assert_eq!(snap2.local_usage.unwrap().all_time.requests, 1);
    }

    #[test]
    fn session_id_prefers_parent_directory() {
        let (_dir, home) = tmp_home();
        let p = write_raw(&home, "sess-xyz", &[]);
        assert_eq!(session_id_from_path(&p), "sess-xyz");
        let framed = write_framed(&home, "sess-xyz", &[]);
        assert_eq!(session_id_from_path(&framed), "sess-xyz");
    }
}
