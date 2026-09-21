//! Claude Code provider — 100% local transcript data.
//!
//! Data source: the Claude Code CLI's session transcripts under
//! `<config>/projects/<munged-project-dir>/<session-uuid>.jsonl`
//! (append-only JSONL; `<config>` is `~/.claude`, `$CLAUDE_CONFIG_DIR`
//! overrides that, and the panel's own setting wins over both — the same
//! precedence DSH/Codex use). The munged directory name encodes the
//! workspace path, but un-munging it is ambiguous (every separator and
//! special character becomes `-`), so the real project path is taken from
//! the `cwd` field the CLI writes on every line instead. Directory names are
//! compared with the crate-wide case-insensitive convention.
//!
//! Lines are Claude-Code style JSONL (`{"type":"assistant","message":{…
//! "usage":{…}}, "cwd":…, "sessionId":…}`) — exactly the shape the panel's
//! `zcode::usage` tolerant parser already supports (exclusive schema:
//! `input_tokens` excludes `cache_read_input_tokens` /
//! `cache_creation_input_tokens`). On top of that:
//!
//! - **Titles**: `{"type":"summary","summary":…}` lines (written on
//!   resume/compaction) provide real generated titles; otherwise the first
//!   user message is used — the same convention as the ZCode session table
//!   (generated summary or first user input). Nothing is fabricated.
//! - **Streaming duplicates**: Claude Code appends the same `message.id`
//!   several times while streaming / interleaving thinking, each carrying a
//!   partial usage snapshot. Entries are de-duplicated by `message.id`
//!   keeping the **last** occurrence (the final usage figure) — the strategy
//!   verified by the community tooling against real transcripts. Records
//!   without a message id fall back to the line's own `uuid` (unique per
//!   line, so the fallback never collapses distinct responses).
//! - **No quota windows**: Claude Code has no locally queryable official
//!   plan-quota interface, so this provider reports local token usage only
//!   and never invents an official quota.
//!
//! Parse state (byte watermark + de-duplicated entries) is persisted like
//! the other local providers, so restarts never re-count history.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::local_usage::{aggregate_local, record_delta, SessionUsage};
use super::session_index::SessionContrib;
use super::{ProviderSnapshot, ProviderStatus};

const CLAUDE_CACHE_SCHEMA_VERSION: u32 = 1;
/// Defensive cap for one transcript file (per-session append-only logs).
const MAX_TRANSCRIPT: u64 = 256 * 1024 * 1024;
/// Session ids are UUIDs the CLI writes into every line; the file name is
/// the same UUID, which we accept case-insensitively like directory names.
const SESSION_FILE_EXT: &str = "jsonl";

// ---------------------------------------------------------------------------
// Parse cache
// ---------------------------------------------------------------------------

/// Per-file parse state. `entries` is the de-duplication map keyed by
/// message id (last write wins); `session` is derived from it whenever the
/// file grows, so aggregates and the sorted record history can never drift
/// from the de-duplicated truth.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ClaudeFileEntry {
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub entries: HashMap<String, crate::zcode::usage::UsageRecord>,
    #[serde(default)]
    pub session: SessionUsage,
}

/// Persisted parse cache: per-file state. Saved atomically; corrupt files
/// fall back to empty.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ClaudeCache {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    files: HashMap<String, ClaudeFileEntry>,
    #[serde(default)]
    saved_at_ms: i64,
}

impl ClaudeCache {
    fn fresh() -> Self {
        Self {
            schema_version: CLAUDE_CACHE_SCHEMA_VERSION,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

pub struct ClaudeCodeProvider {
    home: PathBuf,
    cache: ClaudeCache,
    cache_path: Option<PathBuf>,
    cache_needs_persist: bool,
}

impl ClaudeCodeProvider {
    pub fn new(cache_path: Option<PathBuf>) -> Self {
        let (cache, cache_needs_persist) = cache_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| {
                let parsed = serde_json::from_str::<ClaudeCache>(&t).ok()?;
                if parsed.schema_version == CLAUDE_CACHE_SCHEMA_VERSION {
                    Some((parsed, false))
                } else {
                    Some((ClaudeCache::fresh(), true))
                }
            })
            .unwrap_or_else(|| (ClaudeCache::fresh(), false));
        let mut provider = Self {
            home: default_home(),
            cache,
            cache_path,
            cache_needs_persist,
        };
        provider.rebuild_derived_sessions();
        provider
    }

    /// Point at a (possibly different) config dir; a changed root discards
    /// watermarks that belong to another tree.
    pub fn with_home(&mut self, home: PathBuf) -> &mut Self {
        if self.home != home {
            let belongs_to_home = !self.cache.files.is_empty()
                && self
                    .cache
                    .files
                    .keys()
                    .all(|key| Path::new(key).starts_with(&home));
            if !belongs_to_home {
                self.cache = ClaudeCache::fresh();
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
    pub fn contribs(&self) -> Vec<SessionContrib<'_>> {
        self.sessions().map(SessionContrib::from_usage).collect()
    }

    /// One poll cycle. Never panics; every failure degrades to a status.
    pub fn poll(&mut self, now_ms: i64) -> ProviderSnapshot {
        let mut snap = ProviderSnapshot::empty(super::PROVIDER_CLAUDE_CODE, ProviderStatus::Ok, now_ms);
        snap.source = "Claude Code 本地 session 转写(离线读取)".into();
        snap.source_url = Some("https://code.claude.com/docs/".into());

        let projects = self.home.join("projects");
        if !projects.is_dir() {
            snap.status = ProviderStatus::NotInstalled;
            snap.error = Some(
                "未检测到 Claude Code 数据目录(默认 ~/.claude/projects;可在「设置 → Claude Code」指定路径或设置 CLAUDE_CONFIG_DIR)".into(),
            );
            return snap;
        }

        let mut files = Vec::new();
        collect_transcripts(&projects, &mut files);
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
            // Transcripts without any sessionId line keep the file stem
            // (the session UUID the CLI named the file after) as the honest
            // id. Resolved before the entry borrow so a failed read can
            // still drop the key from the cache below.
            let needs_id = self
                .cache
                .files
                .get(&key)
                .map(|e| e.session.session_id.is_empty())
                .unwrap_or(true);
            let entry = self.cache.files.entry(key).or_default();
            if needs_id {
                entry.session.session_id = session_id_from_path(path);
            }
            match advance_file(path, entry) {
                Ok(grew) => changed |= grew,
                Err(why) => {
                    self.cache
                        .files
                        .remove(&path.to_string_lossy().into_owned());
                    snap.notes
                        .push(format!("跳过无法读取的 session 转写:{why}"));
                }
            }
        }
        self.cache.saved_at_ms = now_ms;

        let has_usage = self
            .cache
            .files
            .values()
            .any(|entry| entry.session.responses > 0);
        snap.local_usage = Some(aggregate_local(
            self.cache.files.values().map(|entry| &entry.session),
            now_ms,
        ));

        if !has_usage {
            if files.is_empty() {
                snap.status = ProviderStatus::NotConfigured;
                snap.error =
                    Some("projects 目录为空(在 Claude Code 里发起一次对话后自动出现)".into());
            } else {
                snap.notes
                    .push("已发现 session 转写,但尚未记录到任何 token usage 行".into());
            }
        } else {
            snap.notes.push(
                "来自 Claude Code 转写的 assistant message.usage;input 不含 cache(读/写单列),总量不重复累计".into(),
            );
        }
        snap.notes.push("session 转写统计 · 不计入 ZCode 总 Token".into());

        if changed {
            self.persist_cache();
            self.cache_needs_persist = false;
        }
        snap
    }

    /// Recompute every file's derived `SessionUsage` from its persisted
    /// entries (cache load path). Aggregates are never trusted from disk —
    /// they are cheap to fold and this guarantees one single definition.
    fn rebuild_derived_sessions(&mut self) {
        for entry in self.cache.files.values_mut() {
            recompute_session(entry);
        }
    }
}

pub fn default_home() -> PathBuf {
    if let Some(p) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        let p = PathBuf::from(p);
        if p.is_dir() {
            return p;
        }
    }
    dirs::home_dir()
        .map(|h| h.join(".claude"))
        .unwrap_or_else(|| PathBuf::from(".claude"))
}

fn collect_transcripts(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_transcripts(&p, out);
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case(SESSION_FILE_EXT))
        {
            out.push(p);
        }
    }
}

/// Session id from the transcript file name (`<session-uuid>.jsonl`). The
/// lines' own `sessionId` field wins when present; this is the fallback and
/// uses the same UUID the CLI chose.
fn session_id_from_path(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "claude-session".to_string())
}

// ---------------------------------------------------------------------------
// File advancing
// ---------------------------------------------------------------------------

/// Advance one transcript past its byte watermark. Only complete lines are
/// consumed (a half-written trailing line waits for the next poll).
fn advance_file(path: &Path, entry: &mut ClaudeFileEntry) -> Result<bool, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    if size > MAX_TRANSCRIPT {
        return Err(format!("transcript exceeds {MAX_TRANSCRIPT} bytes (not a session log?)"));
    }
    if size < entry.offset {
        // Truncated/rewritten → re-read from the start.
        entry.size = 0;
        entry.offset = 0;
        entry.entries.clear();
        entry.session = SessionUsage::default();
    }
    if size == entry.offset {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(entry.offset))
        .map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(file);
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
        if apply_line(&line, entry, &source_file) {
            grew = true;
        }
    }
    entry.offset = offset;
    entry.size = size;
    if grew {
        recompute_session(entry);
    }
    Ok(grew)
}

/// Fold the de-duplicated entries into the file's derived accumulator:
/// totals, model splits, timestamps, and the ts-sorted record history.
fn recompute_session(entry: &mut ClaudeFileEntry) {
    let mut session = SessionUsage::default();
    session.session_id = entry
        .session
        .session_id
        .clone();
    session.title = entry.session.title.clone();
    session.project_path = entry.session.project_path.clone();

    let mut records: Vec<crate::zcode::usage::UsageRecord> =
        entry.entries.values().cloned().collect();
    records.sort_by_key(|r| r.ts_ms);
    for r in &records {
        if session.session_id.is_empty() {
            if let Some(sid) = &r.session_id {
                session.session_id = sid.clone();
            }
        }
        if session.project_path.is_none() {
            if let Some(cwd) = &r.project {
                session.project_path = Some(cwd.clone());
            }
        }
        session.responses += 1;
        let delta = record_delta(r);
        session.all_time.add(&delta);
        session
            .model_totals
            .entry(r.model.clone())
            .or_default()
            .add(&delta);
        *session.model_requests.entry(r.model.clone()).or_insert(0) += 1;
        if session.first_ts_ms == 0 || (r.ts_ms > 0 && r.ts_ms < session.first_ts_ms) {
            session.first_ts_ms = r.ts_ms;
        }
        if r.ts_ms > session.last_ts_ms {
            session.last_ts_ms = r.ts_ms;
        }
        if session.model.is_empty() {
            session.model = r.model.clone();
        }
    }
    session.records = records;
    entry.session = session;
}

/// Apply one transcript line. Returns true when the line produced (or
/// replaced) a usage entry. Non-usage lines still contribute metadata
/// (session id, cwd, summary title, first user message).
fn apply_line(line: &str, entry: &mut ClaudeFileEntry, source_file: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return false;
    };
    let line_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

    // Metadata-only lines (no usage object) feed the session meta fields.
    let session = &mut entry.session;
    if session.session_id.is_empty() {
        if let Some(sid) = v.get("sessionId").and_then(|s| s.as_str()) {
            session.session_id = sid.to_string();
        }
    }
    if session.project_path.is_none() {
        if let Some(cwd) = v.get("cwd").and_then(|c| c.as_str()) {
            session.project_path = Some(cwd.to_string());
        }
    }
    match line_type {
        "summary" => {
            if session.title.is_none() {
                session.title = v
                    .get("summary")
                    .and_then(|s| s.as_str())
                    .and_then(normalize_title);
            }
            false
        }
        "user" => {
            if session.title.is_none() {
                session.title = first_user_text(&v);
            }
            false
        }
        "assistant" => {
            let Some(record) = parse_assistant_record(&v, session, source_file) else {
                return false;
            };
            // Streaming duplicates: the same message.id appears several
            // times; the LAST occurrence carries the final usage snapshot.
            // Lines without a message id fall back to their own uuid, then
            // to a content-derived key — distinct responses never collapse.
            let key = v
                .pointer("/message/id")
                .and_then(|m| m.as_str())
                .map(str::to_string)
                .or_else(|| v.get("uuid").and_then(|u| u.as_str()).map(str::to_string))
                .unwrap_or_else(|| {
                    format!(
                        "anon-{}-{}-{}",
                        record.ts_ms, record.input_tokens, record.output_tokens
                    )
                });
            entry.entries.insert(key, record);
            true
        }
        _ => false,
    }
}

/// Parse one assistant line into a shared-schema record via the tolerant
/// zcode parser (Claude style: exclusive input + separate cache fields).
fn parse_assistant_record(
    v: &serde_json::Value,
    session: &SessionUsage,
    source_file: &str,
) -> Option<crate::zcode::usage::UsageRecord> {
    let ctx = crate::zcode::usage::LineContext {
        session_hint: (!session.session_id.is_empty()).then(|| session.session_id.clone()),
        project_hint: session.project_path.clone(),
        source_file: source_file.to_string(),
    };
    // Sidechain (subagent) lines are real API usage and carry their own
    // usage objects — they count like any other response.
    let mut record = crate::zcode::usage::extract_record(v, &ctx)
        .ok()
        .flatten()
        .filter(|r| r.input_tokens > 0 || r.output_tokens > 0)?;
    // Claude-style usage is documented exclusive (input excludes the cache
    // tokens) — pin the caliber so the numeric heuristic can never
    // misread a large fresh input as "cache already included".
    record.schema_exclusive = Some(true);
    Some(record)
}

/// First user message text as a title candidate (same convention as the
/// ZCode session table: generated summary or first user input). Synthetic
/// command bodies (XML-ish wrappers, tool results) are skipped.
fn first_user_text(v: &serde_json::Value) -> Option<String> {
    let content = v.pointer("/message/content")?;
    let text: Option<&str> = match content {
        serde_json::Value::String(s) => Some(s.as_str()),
        serde_json::Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                (part.get("type").and_then(|t| t.as_str()) == Some("text"))
                    .then(|| part.get("text").and_then(|x| x.as_str()))
                    .flatten()
            })
            .find(|s| !s.trim().is_empty()),
        _ => None,
    };
    normalize_title(text?)
}

/// Trim / sanity-cap a candidate title. Truncation is presentation, not
/// fabrication — the full text stays in the source file.
fn normalize_title(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() || s.starts_with('<') || s.starts_with("[Request interrupted") {
        return None;
    }
    let mut out = String::from(s);
    if out.chars().count() > 300 {
        out = out.chars().take(300).collect();
    }
    Some(out)
}

/// Public helper for tests: build one file entry from raw lines.
pub fn entry_from_lines(lines: &[String]) -> ClaudeFileEntry {
    let mut entry = ClaudeFileEntry::default();
    for line in lines {
        apply_line(line, &mut entry, "test.jsonl");
    }
    recompute_session(&mut entry);
    entry
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_790_000_000_000;

    fn ts(ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(ms)
            .unwrap()
            .to_rfc3339()
    }

    fn assistant_line(msg_id: &str, input: u64, output: u64, cache_read: u64, cache_write: u64) -> String {
        serde_json::json!({
            "parentUuid": null,
            "isSidechain": false,
            "userType": "external",
            "cwd": "C:\\Users\\27632\\Desktop\\zcode-panel",
            "sessionId": "9f3a2b71-1111-2222-3333-444455556666",
            "version": "2.1.103",
            "type": "assistant",
            "uuid": format!("line-{msg_id}"),
            "message": {
                "id": msg_id,
                "model": "claude-sonnet-5",
                "role": "assistant",
                "content": [{ "type": "text", "text": "ok" }],
                "usage": {
                    "input_tokens": input,
                    "cache_creation_input_tokens": cache_write,
                    "cache_read_input_tokens": cache_read,
                    "output_tokens": output
                }
            },
            "timestamp": ts(T0),
            "requestId": "req_1"
        })
        .to_string()
    }

    fn user_line(text: &str) -> String {
        serde_json::json!({
            "type": "user",
            "cwd": "C:\\Users\\27632\\Desktop\\zcode-panel",
            "sessionId": "9f3a2b71-1111-2222-3333-444455556666",
            "uuid": "u1",
            "timestamp": ts(T0 - 5_000),
            "message": { "role": "user", "content": text }
        })
        .to_string()
    }

    fn summary_line(text: &str) -> String {
        serde_json::json!({
            "type": "summary",
            "summary": text,
            "leafUuid": "some-uuid"
        })
        .to_string()
    }

    fn tmp_home() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join(".claude");
        std::fs::create_dir_all(
            home.join("projects")
                .join("C--Users-27632-Desktop-zcode-panel"),
        )
        .unwrap();
        (dir, home)
    }

    fn write_transcript(home: &Path, munged_dir: &str, name: &str, lines: &[String]) -> PathBuf {
        let p = home.join("projects").join(munged_dir).join(name);
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(&p, lines.join("\n") + "\n").unwrap();
        p
    }

    #[test]
    fn parses_claude_style_usage_with_exclusive_cache() {
        let entry = entry_from_lines(&[
            user_line("帮我重构 Sessions 页"),
            assistant_line("msg_1", 1200, 340, 40_000, 5_000),
        ]);
        let s = &entry.session;
        assert_eq!(s.session_id, "9f3a2b71-1111-2222-3333-444455556666");
        assert_eq!(s.title.as_deref(), Some("帮我重构 Sessions 页"));
        assert_eq!(
            s.project_path.as_deref(),
            Some("C:\\Users\\27632\\Desktop\\zcode-panel")
        );
        assert_eq!(s.responses, 1);
        assert_eq!(s.records.len(), 1);
        // Exclusive schema: total = in + out + cache_read + cache_write.
        assert_eq!(s.records[0].display_total_tokens(), 1200 + 340 + 40_000 + 5_000);
        assert_eq!(s.records[0].cache_read_tokens, Some(40_000));
        assert_eq!(s.records[0].cache_write_tokens, Some(5_000));
        assert_eq!(s.records[0].reasoning_tokens, None);
        assert_eq!(s.model_requests["claude-sonnet-5"], 1);
    }

    #[test]
    fn summary_line_title_wins_over_first_user_message() {
        let entry = entry_from_lines(&[
            summary_line("会话压缩摘要标题"),
            user_line("第一条用户消息"),
            assistant_line("msg_1", 10, 5, 0, 0),
        ]);
        assert_eq!(entry.session.title.as_deref(), Some("会话压缩摘要标题"));
    }

    #[test]
    fn streaming_duplicates_keep_the_last_usage_snapshot() {
        // Same message.id written three times while streaming: usage grows
        // 100 → 200 → 350. Only the final snapshot may count.
        let entry = entry_from_lines(&[
            user_line("问点东西"),
            assistant_line("msg_1", 100, 10, 0, 0),
            assistant_line("msg_1", 200, 20, 50, 0),
            assistant_line("msg_1", 350, 35, 50, 10),
            assistant_line("msg_2", 40, 8, 350, 0),
        ]);
        let s = &entry.session;
        assert_eq!(s.responses, 2, "one response per message id");
        assert_eq!(s.all_time.total_tokens, 350 + 35 + 50 + 10 + 40 + 8 + 350);
        assert_eq!(s.records.len(), 2);
    }

    #[test]
    fn poll_reads_munged_project_dirs_and_reports_local_usage() {
        let (_dir, home) = tmp_home();
        write_transcript(
            &home,
            "C--Users-27632-Desktop-zcode-panel",
            "9f3a2b71-1111-2222-3333-444455556666.jsonl",
            &[user_line("标题"), assistant_line("msg_1", 100, 50, 700, 30)],
        );
        let mut p = ClaudeCodeProvider::new(None);
        p.with_home(home);
        let snap = p.poll(T0 + 60_000);
        assert_eq!(snap.status, ProviderStatus::Ok);
        let lu = snap.local_usage.unwrap();
        assert_eq!(lu.all_time.requests, 1);
        assert_eq!(lu.all_time.total_tokens, 100 + 50 + 700 + 30);
        assert_eq!(lu.sessions, 1);
        assert_eq!(lu.models[0].model, "claude-sonnet-5");
    }

    #[test]
    fn incremental_append_and_half_line_tolerance() {
        let (_dir, home) = tmp_home();
        let path = write_transcript(
            &home,
            "C--Users-27632-Desktop-zcode-panel",
            "aaa.jsonl",
            &[user_line("t"), assistant_line("msg_1", 10, 5, 0, 0)],
        );
        let mut entry = ClaudeFileEntry::default();
        assert!(advance_file(&path, &mut entry).unwrap());
        assert_eq!(entry.session.responses, 1);

        // Append HALF of a new assistant line — no error, no count.
        let next = assistant_line("msg_2", 7, 3, 0, 0);
        let cut = 40.min(next.len());
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str(&next[..cut]);
        std::fs::write(&path, text).unwrap();
        assert!(!advance_file(&path, &mut entry).unwrap());
        assert_eq!(entry.session.responses, 1);

        // Complete the line — it appears exactly once.
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str(&next[cut..]);
        text.push('\n');
        std::fs::write(&path, text).unwrap();
        assert!(advance_file(&path, &mut entry).unwrap());
        assert_eq!(entry.session.responses, 2);
        assert_eq!(entry.session.all_time.total_tokens, 10 + 5 + 7 + 3);

        // Unchanged file → no growth.
        assert!(!advance_file(&path, &mut entry).unwrap());
    }

    #[test]
    fn truncation_resets_state() {
        let (_dir, home) = tmp_home();
        let path = write_transcript(
            &home,
            "C--Users-27632-Desktop-zcode-panel",
            "bbb.jsonl",
            &[assistant_line("msg_1", 10, 5, 0, 0)],
        );
        let mut entry = ClaudeFileEntry::default();
        advance_file(&path, &mut entry).unwrap();
        assert_eq!(entry.session.responses, 1);

        std::fs::write(&path, "").unwrap();
        advance_file(&path, &mut entry).unwrap();
        assert_eq!(entry.session.responses, 0);
        assert!(entry.entries.is_empty());
        assert_eq!(entry.offset, 0);
    }

    #[test]
    fn garbage_and_non_usage_lines_are_skipped() {
        let entry = entry_from_lines(&[
            "not json".into(),
            serde_json::json!({"type":"system","subtype":"init","sessionId":"s1"}).to_string(),
            serde_json::json!({"noType":true}).to_string(),
            // Assistant line without usage → no record, but no error either.
            serde_json::json!({
                "type":"assistant","sessionId":"s1","cwd":"/home/u/p",
                "timestamp": ts(T0),
                "message":{"id":"m0","model":"claude-sonnet-5","content":[]}
            }).to_string(),
            assistant_line("msg_1", 8, 4, 0, 0),
        ]);
        assert_eq!(entry.session.responses, 1);
        assert_eq!(entry.session.all_time.total_tokens, 12);
    }

    #[test]
    fn missing_home_is_not_installed_and_empty_projects_is_not_configured() {
        let mut p = ClaudeCodeProvider::new(None);
        p.with_home(PathBuf::from("/nonexistent/claude-home"));
        assert_eq!(p.poll(0).status, ProviderStatus::NotInstalled);

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join(".claude");
        std::fs::create_dir_all(home.join("projects")).unwrap();
        let mut q = ClaudeCodeProvider::new(None);
        q.with_home(home);
        assert_eq!(q.poll(0).status, ProviderStatus::NotConfigured);
    }

    #[test]
    fn session_id_falls_back_to_file_name_and_cache_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("claude-usage-cache.json");
        let (_h, home) = tmp_home();
        // No sessionId field on any line → the file stem is the honest id.
        let no_sid = serde_json::json!({
            "type": "assistant",
            "cwd": "/home/u/panel",
            "timestamp": ts(T0),
            "message": {
                "id": "m1",
                "model": "claude-opus-5",
                "usage": { "input_tokens": 5, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0, "output_tokens": 6 }
            }
        })
        .to_string();
        write_transcript(&home, "home-u-panel", "0d0e0f-uuid.jsonl", &[no_sid]);
        let mut p = ClaudeCodeProvider::new(Some(cache_path.clone()));
        p.with_home(home.clone());
        let snap = p.poll(T0 + 1_000);
        assert_eq!(snap.local_usage.unwrap().all_time.requests, 1);
        assert!(cache_path.exists());
        let ids: Vec<&str> = p.sessions().map(|s| s.session_id.as_str()).collect();
        assert!(ids.contains(&"0d0e0f-uuid"));

        // A fresh provider reloads the cache without re-counting history.
        let mut q = ClaudeCodeProvider::new(Some(cache_path));
        q.with_home(home);
        let snap2 = q.poll(T0 + 2_000);
        assert_eq!(snap2.local_usage.unwrap().all_time.requests, 1);
        let session = q.sessions().next().unwrap();
        assert_eq!(session.project_path.as_deref(), Some("/home/u/panel"));
        assert_eq!(session.all_time.total_tokens, 11);
    }
}
