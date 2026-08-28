//! Data-directory discovery.
//!
//! Resolution order for the ZCode data root:
//! 1. the user-configured `dataDir` (Settings page),
//! 2. `ZCODE_HOME` environment variable,
//! 3. `<home>/.zcode` (the standard location; on Windows
//!    `%USERPROFILE%\.zcode`).
//!
//! Inside the root we look for:
//! - `*.jsonl` files (session transcripts / usage streams) — typically under
//!   a `projects/` tree,
//! - `*.db` / `*.sqlite` / `*.sqlite3` (SQLite usage stores, if the installed
//!   ZCode version uses one).
//!
//! The scan is a bounded manual walk (it must survive weird trees and
//! never descend into `node_modules` and friends).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

const MAX_DEPTH: usize = 8;
const MAX_ENTRIES: usize = 200_000;
const SKIP_DIRS: &[&str] = &[
    "node_modules", ".git", ".hg", ".svn", "__pycache__", ".venv", "venv",
    "target", "dist", "build", "Cache", "cache", "tmp", "temp",
];

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataLayout {
    pub root: PathBuf,
    pub jsonl_files: Vec<PathBuf>,
    pub sqlite_files: Vec<PathBuf>,
    pub scan_ms: u64,
    pub notes: Vec<String>,
}

/// Resolve the effective data root from explicit setting / env / default.
pub fn resolve_root(configured: Option<&str>) -> Option<PathBuf> {
    if let Some(dir) = configured {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return Some(p);
        }
        return None; // configured but missing — caller surfaces this loudly
    }
    if let Ok(env_dir) = std::env::var("ZCODE_HOME") {
        let p = PathBuf::from(env_dir);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Some(home) = dirs::home_dir() {
        let p = home.join(".zcode");
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

pub fn discover(root: &Path) -> io::Result<DataLayout> {
    let started = Instant::now();
    let mut jsonl_files = Vec::new();
    let mut sqlite_files = Vec::new();
    let mut notes = Vec::new();
    let mut entries_seen: usize = 0;

    // Stack of (path, depth). DFS keeps memory bounded by tree depth.
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            continue;
        }
        let Ok(read_dir) = fs::read_dir(&dir) else {
            continue; // unreadable subdir — skip, don't fail the scan
        };
        for entry in read_dir.flatten() {
            entries_seen += 1;
            if entries_seen > MAX_ENTRIES {
                notes.push(format!("scan stopped at {MAX_ENTRIES} entries in {}", dir.display()));
                stack.clear();
                break;
            }
            let path = entry.path();
            // cheap metadata via file_type first; fall back to fs::metadata
            // only when needed (symlinks)
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push((path, depth + 1));
                }
            } else if ft.is_file() {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_lowercase())
                    .unwrap_or_default();
                match ext.as_str() {
                    "jsonl" => jsonl_files.push(path),
                    "db" | "sqlite" | "sqlite3" => {
                        // -wal / -shm sidecars have compound extensions, so they
                        // never end in .db here.
                        sqlite_files.push(path);
                    }
                    _ => {}
                }
            }
        }
    }

    jsonl_files.sort();
    sqlite_files.sort();
    Ok(DataLayout {
        root: root.to_path_buf(),
        jsonl_files,
        sqlite_files,
        scan_ms: started.elapsed().as_millis() as u64,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_jsonl_and_sqlite_recursively() {
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("cli").join("projects").join("proj-A");
        fs::create_dir_all(&projects).unwrap();
        fs::write(
            projects.join("1111111111111111111111111111111.jsonl"),
            "{}",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("cli")).unwrap();
        fs::write(dir.path().join("cli").join("usage.db"), "").unwrap();
        // noise that must be ignored
        fs::create_dir_all(dir.path().join("node_modules").join("x")).unwrap();
        fs::write(dir.path().join("node_modules").join("x").join("junk.jsonl"), "").unwrap();

        let layout = discover(dir.path()).unwrap();
        assert_eq!(layout.jsonl_files.len(), 1);
        assert_eq!(layout.sqlite_files.len(), 1);
    }

    #[test]
    fn configured_root_missing_returns_none() {
        assert!(resolve_root(Some("/definitely/not/here")).is_none());
    }
}
