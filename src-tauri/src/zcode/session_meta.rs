//! Session metadata sidecar: real session titles & workspace directories.
//!
//! The usage tables only record `session_id`; the human-facing session title
//! and the project directory live in the ZCode CLI `session` table. This
//! sidecar reads them (read-only, best-effort) so the Sessions page can show
//! real names without coupling them to usage ingestion. Nothing is fabricated:
//! sessions without a metadata row simply keep no title / project, and the UI
//! degrades honestly.

use std::collections::HashMap;
use std::path::Path;

use super::sqlite;

/// One session's metadata from the `session` table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionMetaEntry {
    /// Real title (generated summary or first user input), trimmed; empty
    /// titles normalize to `None`.
    pub title: Option<String>,
    /// Workspace directory (full path), when recorded.
    pub directory: Option<String>,
    /// Last path segment of `directory` — the project folder name.
    pub project_name: Option<String>,
}

/// Read `session(id, title, directory|path)` from a ZCode CLI database.
///
/// Best-effort by design: returns `None` when the file cannot be opened, the
/// database is busy, or no compatible `session` table exists (non-ZCode
/// sqlite sources) — enrichment must never break or block usage ingestion.
/// The engine retries on a later refresh cycle anyway.
pub fn read_session_meta(path: &Path) -> Option<HashMap<String, SessionMetaEntry>> {
    let conn = open_quietly(path)?;
    let cols = sqlite::columns_of(&conn, "session");
    if cols.is_empty() {
        return None;
    }
    let has = |name: &str| cols.iter().any(|c| c.eq_ignore_ascii_case(name));
    if !has("id") || !has("title") || !(has("directory") || has("path")) {
        return None;
    }
    let dir_col = if has("directory") { "directory" } else { "path" };
    let sql = format!(r#"SELECT "id", "title", "{dir_col}" FROM "session""#);
    let mut stmt = conn.prepare(&sql).ok()?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .ok()?;
    let mut map = HashMap::new();
    for row in rows.flatten() {
        let (id, title, directory) = row;
        let Some(id) = non_empty(id) else { continue };
        let directory = non_empty(directory);
        map.insert(
            id,
            SessionMetaEntry {
                title: non_empty(title),
                project_name: directory.as_deref().and_then(folder_name),
                directory,
            },
        );
    }
    Some(map)
}

fn open_quietly(path: &Path) -> Option<rusqlite::Connection> {
    match sqlite::open_readonly(path) {
        Ok(conn) => Some(conn),
        // Busy / locked / unreadable: skip this cycle, retry later.
        Err(_) => None,
    }
}

fn non_empty(s: Option<String>) -> Option<String> {
    let s = s?.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Last path segment of a directory string, tolerating both `/` and `\`
/// separators regardless of the OS this build runs on.
fn folder_name(dir: &str) -> Option<String> {
    let trimmed = dir.trim_end_matches(['/', '\\']);
    let last = trimmed.rsplit(['/', '\\']).next()?;
    let name = last.trim();
    (!name.is_empty()).then(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zcode_like_db(path: &Path) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                directory TEXT NOT NULL,
                path TEXT,
                time_updated INTEGER NOT NULL
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (id, title, directory, time_updated) VALUES (?1, ?2, ?3, 0)",
            rusqlite::params!["sess-1", "  优化登录性能  ", "/home/u/projects/panel"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (id, title, directory, time_updated) VALUES (?1, ?2, ?3, 0)",
            rusqlite::params!["sess-2", "", "D:\\work\\crawler_Xianyu\\"],
        )
        .unwrap();
        // No directory value → project_name stays None.
        conn.execute(
            "INSERT INTO session (id, title, directory, time_updated) VALUES (?1, ?2, ?3, 0)",
            rusqlite::params!["sess-3", "调研", ""],
        )
        .unwrap();
    }

    #[test]
    fn reads_real_titles_and_project_folders() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("db.sqlite");
        zcode_like_db(&db);

        let meta = read_session_meta(&db).expect("session table present");
        let s1 = &meta["sess-1"];
        assert_eq!(s1.title.as_deref(), Some("优化登录性能"));
        assert_eq!(s1.directory.as_deref(), Some("/home/u/projects/panel"));
        assert_eq!(s1.project_name.as_deref(), Some("panel"));
        // Windows separators + trailing slash are handled on any OS.
        let s2 = &meta["sess-2"];
        assert_eq!(s2.title, None, "empty title normalizes to None");
        assert_eq!(s2.project_name.as_deref(), Some("crawler_Xianyu"));
        let s3 = &meta["sess-3"];
        assert_eq!(s3.directory, None);
        assert_eq!(s3.project_name, None);
    }

    #[test]
    fn db_without_session_table_or_wrong_shape_yields_none() {
        // No session table at all.
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("other.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE model_usage (input_tokens INTEGER);").unwrap();
        assert!(read_session_meta(&db).is_none());

        // Session-like table missing the title column → not compatible.
        let db2 = dir.path().join("partial.db");
        let conn2 = rusqlite::Connection::open(&db2).unwrap();
        conn2.execute_batch("CREATE TABLE session (id TEXT, directory TEXT);").unwrap();
        assert!(read_session_meta(&db2).is_none());

        // Missing file → best-effort None.
        assert!(read_session_meta(&dir.path().join("nope.db")).is_none());
    }

    #[test]
    fn folder_name_edge_cases() {
        assert_eq!(folder_name("/a/b/zcode-usage-panel"), Some("zcode-usage-panel".into()));
        assert_eq!(folder_name("D:\\linux_project\\panel\\"), Some("panel".into()));
        assert_eq!(folder_name("/"), None);
        assert_eq!(folder_name(""), None);
    }
}
