//! Read-only SQLite reader with runtime schema discovery.
//!
//! ZCode versions differ in whether usage lives in SQLite at all and what the
//! tables look like. We therefore:
//! 1. open every `*.db` / `*.sqlite` in **read-only** mode (`SQLITE_OPEN_READ_ONLY`),
//! 2. enumerate tables/views via `sqlite_master`,
//! 3. score each table by how many recognizable token columns it has,
//! 4. map columns through alias sets,
//! 5. read incrementally using a rowid / integer-primary-key watermark.
//!
//! Busy databases (ZCode writing right now) surface as `SourceError::Busy`
//! after a short `busy_timeout`; the engine retries later. Opening a WAL
//! database read-only without its writer alive can fail; we retry with
//! `immutable=1` as a last resort (snapshot view, safe: we never write).

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};

use super::errors::SourceError;
use super::usage::UsageRecord;

const ALIAS_INPUT: &[&str] = &[
    "input_tokens", "inputtokens", "prompt_tokens", "prompttokens", "input_token_count",
    "inputtokenscount", "input",
];
const ALIAS_OUTPUT: &[&str] = &[
    "output_tokens", "outputtokens", "completion_tokens", "completiontokens",
    "output_token_count", "output", "completion",
];
const ALIAS_REASONING: &[&str] = &[
    "reasoning_tokens", "reasoningtokens", "thinking_tokens", "reasoning_output_tokens",
    "reasoning",
];
const ALIAS_CACHE_READ: &[&str] = &[
    "cache_read_input_tokens", "cachereadinputtokens", "cached_input_tokens",
    "cachedinputtokens", "cached_tokens", "cachedtokens", "cache_read", "cacheread",
    "cache_read_tokens",
];
const ALIAS_CACHE_WRITE: &[&str] = &[
    "cache_creation_input_tokens", "cachecreationinputtokens", "cache_write_input_tokens",
    "cachewriteinputtokens", "cache_creation", "cachecreation", "cache_write",
];
const ALIAS_TIME: &[&str] = &[
    "timestamp", "ts", "time", "created_at", "createdat", "request_timestamp",
    "requesttimestamp", "date", "created_at_ms", "timestamp_ms", "started_at",
];
const ALIAS_MODEL: &[&str] = &["model", "model_name", "modelname", "model_id", "modelid"];
const ALIAS_SESSION: &[&str] = &["session_id", "sessionid", "session", "conversation_id"];
const ALIAS_PROJECT: &[&str] = &["project", "project_path", "projectpath", "cwd", "workspace"];

fn norm(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

fn first_alias_match(col: &str, aliases: &[&str]) -> bool {
    let c = norm(col);
    aliases.iter().any(|a| c == *a)
}

#[derive(Clone, Debug, Default)]
pub struct ColumnMap {
    pub time: Option<String>,
    pub model: Option<String>,
    pub session: Option<String>,
    pub project: Option<String>,
    pub input: Option<String>,
    pub output: Option<String>,
    pub reasoning: Option<String>,
    pub cache_read: Option<String>,
    pub cache_write: Option<String>,
    /// rowid-style column used as the incremental watermark.
    pub watermark: Option<String>,
}

impl ColumnMap {
    fn token_column_count(&self) -> usize {
        [
            &self.input, &self.output, &self.reasoning, &self.cache_read, &self.cache_write,
        ]
        .iter()
        .filter(|c| c.is_some())
        .count()
    }
}

#[derive(Clone, Debug)]
pub struct MappedTable {
    pub name: String,
    pub map: ColumnMap,
    /// `true` when no monotonic watermark column exists — the reader falls
    /// back to a full scan with row-identity deduplication.
    pub full_scan: bool,
}

#[derive(Clone, Debug)]
pub struct SqliteSourceState {
    pub path: PathBuf,
    pub table: Option<MappedTable>,
    pub watermark: i64,
    pub records_read: u64,
    pub last_error: Option<String>,
    seen_row_ids: std::collections::HashSet<u64>,
}

impl SqliteSourceState {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            table: None,
            watermark: 0,
            records_read: 0,
            last_error: None,
            seen_row_ids: Default::default(),
        }
    }
}

fn open_readonly(path: &Path) -> Result<Connection, SourceError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI;
    let attempt = |uri: Option<String>| -> rusqlite::Result<Connection> {
        let conn = match uri {
            Some(u) => Connection::open_with_flags(u, flags)?,
            None => Connection::open_with_flags(path, flags)?,
        };
        conn.busy_timeout(Duration::from_millis(800))?;
        Ok(conn)
    };
    match attempt(None) {
        Ok(conn) => Ok(conn),
        Err(first_err) => {
            // WAL database whose writer is not running: try immutable snapshot.
            // Percent-encode non-ASCII bytes so Unicode paths stay valid URIs.
            let encoded: String = path
                .to_string_lossy()
                .replace('\\', "/")
                .bytes()
                .map(|b| {
                    match b {
                        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
                        | b'-' | b'_' | b'.' | b'/' | b':' | b'~' => {
                            String::from_utf8_lossy(&[b]).into_owned()
                        }
                        _ => format!("%{b:02X}"),
                    }
                })
                .collect();
            let uri = format!("file:{encoded}?immutable=1");
            attempt(Some(uri)).map_err(|_| SourceError::RetryLater(first_err.to_string()))
        }
    }
}

/// Classify a rusqlite error: a busy/locked database must surface as
/// `SourceError::Busy` (the engine retries later) rather than `Fatal`.
fn classify_sqlite_err(e: rusqlite::Error) -> SourceError {
    if matches!(
        e,
        rusqlite::Error::SqliteFailure(ref se, _)
            if se.code == rusqlite::ErrorCode::DatabaseBusy
                || se.code == rusqlite::ErrorCode::DatabaseLocked
    ) {
        SourceError::Busy
    } else {
        SourceError::Fatal(e.to_string())
    }
}

fn list_tables(conn: &Connection) -> Result<Vec<String>, SourceError> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%'")
        .map_err(classify_sqlite_err)?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(classify_sqlite_err)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(names)
}

fn columns_of(conn: &Connection, table: &str) -> Vec<String> {
    // Table names come from sqlite_master, but still quote to be safe.
    let sql = format!("PRAGMA table_info(\"{}\")", table.replace('"', "\"\""));
    let Ok(mut stmt) = conn.prepare(&sql) else { return Vec::new() };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(1)) else { return Vec::new() };
    rows.filter_map(|r| r.ok()).collect()
}

fn has_rowid(conn: &Connection, table: &str) -> bool {
    let sql = format!("SELECT rowid FROM \"{}\" LIMIT 1", table.replace('"', "\"\""));
    conn.prepare(&sql)
        .map(|mut s| s.query([]).is_ok())
        .unwrap_or(false)
}

fn map_table(conn: &Connection, table: &str) -> Option<MappedTable> {
    let cols = columns_of(conn, table);
    if cols.is_empty() {
        return None;
    }
    let mut map = ColumnMap::default();
    for col in &cols {
        if map.time.is_none() && first_alias_match(col, ALIAS_TIME) { map.time = Some(col.clone()); }
        if map.model.is_none() && first_alias_match(col, ALIAS_MODEL) { map.model = Some(col.clone()); }
        if map.session.is_none() && first_alias_match(col, ALIAS_SESSION) { map.session = Some(col.clone()); }
        if map.project.is_none() && first_alias_match(col, ALIAS_PROJECT) { map.project = Some(col.clone()); }
        if map.input.is_none() && first_alias_match(col, ALIAS_INPUT) { map.input = Some(col.clone()); }
        if map.output.is_none() && first_alias_match(col, ALIAS_OUTPUT) { map.output = Some(col.clone()); }
        if map.reasoning.is_none() && first_alias_match(col, ALIAS_REASONING) { map.reasoning = Some(col.clone()); }
        if map.cache_read.is_none() && first_alias_match(col, ALIAS_CACHE_READ) { map.cache_read = Some(col.clone()); }
        if map.cache_write.is_none() && first_alias_match(col, ALIAS_CACHE_WRITE) { map.cache_write = Some(col.clone()); }
    }
    if map.token_column_count() == 0 {
        return None;
    }
    let rowid = has_rowid(conn, table);
    map.watermark = if rowid { Some("rowid".to_string()) } else { None };
    Some(MappedTable {
        name: table.to_string(),
        map,
        full_scan: !rowid,
    })
}

/// Pick the most promising usage-like table in the database.
///
/// Discovery errors propagate so a busy/locked database surfaces as
/// `SourceError::Busy` instead of "no usable table".
pub fn discover_table(conn: &Connection) -> Result<Option<MappedTable>, SourceError> {
    let tables = list_tables(conn)?;
    let mut best: Option<(usize, MappedTable)> = None;
    for t in tables {
        if let Some(mt) = map_table(conn, &t) {
            let score = mt.map.token_column_count() * 4
                + if mt.map.time.is_some() { 2 } else { 0 }
                + if mt.map.model.is_some() { 1 } else { 0 };
            let better = best.as_ref().map(|(s, _)| score > *s).unwrap_or(true);
            if better {
                best = Some((score, mt));
            }
        }
    }
    Ok(best.map(|(_, mt)| mt))
}

fn vref_to_i64(v: ValueRef<'_>) -> Option<i64> {
    match v {
        ValueRef::Integer(i) => Some(i),
        ValueRef::Real(f) => Some(f as i64),
        ValueRef::Text(t) => std::str::from_utf8(t).ok()?.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn vref_to_string(v: ValueRef<'_>) -> Option<String> {
    match v {
        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Integer(i) => Some(i.to_string()),
        _ => None,
    }
}

/// Row identity for full-scan dedup fallback: hash of the mapped values.
fn row_identity(row: &[Option<String>]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for cell in row {
        cell.hash(&mut h);
    }
    h.finish()
}

/// Read records with a watermark greater than `state.watermark`.
pub fn read_new(state: &mut SqliteSourceState) -> Result<Vec<UsageRecord>, SourceError> {
    state.last_error = None;
    let conn = open_readonly(&state.path).map_err(|e| {
        state.last_error = Some(e.to_string());
        e
    })?;

    if state.table.is_none() {
        state.table = match discover_table(&conn) {
            Ok(t) => t,
            Err(SourceError::Busy) => return Err(SourceError::Busy),
            Err(SourceError::RetryLater(m)) => return Err(SourceError::RetryLater(m)),
            Err(_) => None,
        };
        if state.table.is_none() {
            return Err(SourceError::Fatal(
                "no table with recognizable token columns found".into(),
            ));
        }
    }
    let table = state.table.clone().unwrap();

    let wm_col = table.map.watermark.clone().unwrap_or_default();
    let mut select_cols: Vec<String> = Vec::new();
    if !table.full_scan {
        select_cols.push(format!("\"{wm_col}\""));
    }
    // Selection order MUST match the `raws` indexing below:
    // [time, model, session, project, input, output, reasoning, cache_read, cache_write]
    // Every slot occupies a fixed result column — absent mapped columns are
    // selected as NULL so the fixed 9-slot `raws` layout stays aligned.
    for col in [
        &table.map.time,
        &table.map.model,
        &table.map.session,
        &table.map.project,
        &table.map.input,
        &table.map.output,
        &table.map.reasoning,
        &table.map.cache_read,
        &table.map.cache_write,
    ] {
        match col {
            Some(c) => select_cols.push(format!("\"{c}\"")),
            None => select_cols.push("NULL".to_string()),
        }
    }

    let sql = if table.full_scan {
        format!(
            "SELECT {} FROM \"{}\"",
            select_cols.join(", "),
            table.name.replace('"', "\"\"")
        )
    } else {
        format!(
            "SELECT {} FROM \"{}\" WHERE \"{}\" > ?1 ORDER BY \"{}\" LIMIT 50000",
            select_cols.join(", "),
            table.name.replace('"', "\"\""),
            wm_col,
            wm_col
        )
    };

    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::DatabaseBusy || e.code == rusqlite::ErrorCode::DatabaseLocked =>
        {
            return Err(SourceError::Busy);
        }
        Err(e) => {
            // Schema changed underneath us → rediscover once.
            state.table = match discover_table(&conn) {
                Ok(t) => t,
                Err(_) => None,
            };
            let msg = e.to_string();
            state.last_error = Some(msg.clone());
            return Err(SourceError::RetryLater(msg));
        }
    };

    let params: Vec<&dyn rusqlite::ToSql> = if table.full_scan {
        vec![]
    } else {
        vec![&state.watermark]
    };

    let mut records = Vec::new();
    let mut max_wm = state.watermark;
    let mut new_seen: Vec<u64> = Vec::new();
    let mut rows = match stmt.query(params.as_slice()) {
        Ok(r) => r,
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::DatabaseBusy || e.code == rusqlite::ErrorCode::DatabaseLocked =>
        {
            return Err(SourceError::Busy);
        }
        Err(e) => return Err(SourceError::RetryLater(e.to_string())),
    };
    let source_file = state.path.to_string_lossy().into_owned();
    loop {
        match rows.next() {
            Ok(Some(row)) => {
                let mut idx = 0usize;
                let wm = if table.full_scan {
                    0i64
                } else {
                    let v = row.get_ref(idx).map_err(|e| SourceError::Fatal(e.to_string()))?;
                    idx += 1;
                    vref_to_i64(v).unwrap_or(0)
                };
                // Read the mapped cells (order fixed by `select_cols` above).
                let mut raws: Vec<Option<String>> = Vec::with_capacity(9);
                for _ in 0..9 {
                    let v = row.get_ref(idx).ok();
                    idx += 1;
                    raws.push(v.and_then(vref_to_string));
                }
                let identity = row_identity(&raws);
                if table.full_scan && state.seen_row_ids.contains(&identity) {
                    continue;
                }
                new_seen.push(identity);

                let ts = raws[0].as_deref().and_then(|s| ts_from_cell(&s));
                let (Some(ts_ms), Some(model)) = (ts, raws[1].clone()) else {
                    continue;
                };
                let num = |s: &Option<String>| s.as_deref().and_then(|x| x.parse::<u64>().ok());
                let input = num(&raws[4]);
                let output = num(&raws[5]);
                if input.is_none() && output.is_none() {
                    continue;
                }
                records.push(UsageRecord {
                    ts_ms,
                    model,
                    session_id: raws[2].clone(),
                    project: raws[3].clone(),
                    input_tokens: input.unwrap_or(0),
                    output_tokens: output.unwrap_or(0),
                    reasoning_tokens: num(&raws[6]),
                    cache_read_tokens: num(&raws[7]),
                    cache_write_tokens: num(&raws[8]),
                    source_file: source_file.clone(),
                });
                max_wm = max_wm.max(wm);
            }
            Ok(None) => break,
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::DatabaseBusy || e.code == rusqlite::ErrorCode::DatabaseLocked =>
            {
                return Err(SourceError::Busy);
            }
            Err(e) => return Err(SourceError::RetryLater(e.to_string())),
        }
    }

    if !table.full_scan {
        state.watermark = max_wm;
    } else {
        state.seen_row_ids.extend(new_seen);
        // Keep the dedup set bounded.
        if state.seen_row_ids.len() > 500_000 {
            state.seen_row_ids.clear(); // worst case: brief double count until next rebuild
            state.last_error = Some("full-scan dedup set overflow; cache reset".into());
        }
    }
    state.records_read += records.len() as u64;
    Ok(records)
}

fn ts_from_cell(s: &str) -> Option<i64> {
    // SQLite cells arrive stringified; parse_ts handles epoch numbers
    // (seconds or milliseconds) and ISO-8601 strings alike.
    super::usage::parse_ts(&serde_json::Value::String(s.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE usage_events (
                request_timestamp INTEGER,
                model_name TEXT,
                session_id TEXT,
                prompt_tokens INTEGER,
                completion_tokens INTEGER,
                cached_tokens INTEGER,
                reasoning_tokens INTEGER
            );",
        )
        .unwrap();
    }

    fn insert(path: &Path, ts: i64, model: &str, input: u64, output: u64, cached: u64) {
        let conn = Connection::open(path).unwrap();
        conn.execute(
            "INSERT INTO usage_events (request_timestamp, model_name, session_id, prompt_tokens, completion_tokens, cached_tokens)
             VALUES (?1, ?2, 's1', ?3, ?4, ?5)",
            rusqlite::params![ts, model, input, output, cached],
        )
        .unwrap();
    }

    #[test]
    fn discovers_schema_and_reads_incrementally() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("usage.db");
        fixture_db(&db);
        insert(&db, 1756300800, "GLM-5.3", 100, 200, 50);
        insert(&db, 1756300900, "GLM-5.3-Flash", 300, 400, 300);

        let mut st = SqliteSourceState::new(db.clone());
        let recs = read_new(&mut st).unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].input_tokens, 100);
        assert_eq!(recs[0].cache_read_tokens, Some(50));
        assert_eq!(recs[0].reasoning_tokens, None);
        assert_eq!(recs[0].ts_ms, 1756300800_000);

        // Incremental: only the new row comes back.
        insert(&db, 1756301000, "GLM-5.3", 1, 1, 1);
        let recs = read_new(&mut st).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].ts_ms, 1756301000_000);

        // No changes → empty.
        assert!(read_new(&mut st).unwrap().is_empty());
    }

    #[test]
    fn busy_database_surfaces_as_busy() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("usage.db");
        fixture_db(&db);
        insert(&db, 1756300800, "m", 1, 1, 0);

        let writer = Connection::open(&db).unwrap();
        writer
            .execute_batch("BEGIN EXCLUSIVE;")
            .unwrap();

        let mut st = SqliteSourceState::new(db);
        let outcome = read_new(&mut st);
        assert!(
            matches!(outcome, Err(SourceError::Busy) | Err(SourceError::RetryLater(_))),
            "expected busy/retry, got {outcome:?}"
        );

        writer.execute_batch("COMMIT;").unwrap();
        assert!(!read_new(&mut st).unwrap().is_empty());
    }

    #[test]
    fn non_usage_db_reports_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("other.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE notes (body TEXT);").unwrap();
        let mut st = SqliteSourceState::new(db);
        assert!(matches!(read_new(&mut st), Err(SourceError::Fatal(_))));
    }

    #[test]
    fn zcode_cli_model_usage_schema_reads_real_records() {
        // Mirrors the real ZCode CLI `model_usage` table: TEXT primary key
        // (hidden rowid), `started_at` ms timestamps, `model_id` as the model
        // name, NO project column, cache_creation/cache_read columns. This is
        // the exact shape that previously returned 0 records because
        // `started_at` was not a recognized time alias.
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("usage.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE model_usage (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                model_id TEXT NOT NULL,
                status TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                reasoning_tokens INTEGER NOT NULL DEFAULT 0,
                cache_creation_input_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_input_tokens INTEGER NOT NULL DEFAULT 0
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO model_usage (id, session_id, model_id, status, started_at, input_tokens, output_tokens, reasoning_tokens, cache_creation_input_tokens, cache_read_input_tokens)
             VALUES ('r1', 's1', 'glm-5.2', 'completed', 1786278274308, 8423, 78, 0, 0, 0)",
            [],
        )
        .unwrap();

        let mut st = SqliteSourceState::new(db.clone());
        let recs = read_new(&mut st).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].ts_ms, 1786278274308);
        assert_eq!(recs[0].model, "glm-5.2");
        assert_eq!(recs[0].session_id.as_deref(), Some("s1"));
        assert_eq!(recs[0].project, None);
        // Column alignment: input/output must not be swapped (no project col).
        assert_eq!(recs[0].input_tokens, 8423);
        assert_eq!(recs[0].output_tokens, 78);
        assert_eq!(recs[0].reasoning_tokens, Some(0));
        assert_eq!(recs[0].cache_write_tokens, Some(0));
        assert_eq!(recs[0].cache_read_tokens, Some(0));

        // Incremental: second read returns nothing new.
        assert!(read_new(&mut st).unwrap().is_empty());
    }
}
