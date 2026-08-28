//! Incremental JSONL transcript reader.
//!
//! Each file keeps a byte-offset watermark. Only bytes appended since the
//! last read are parsed. A trailing partial line (ZCode is mid-write) stays
//! buffered: we only ever advance the offset past complete `\n`-terminated
//! lines, so half-written lines never produce errors — they are picked up on
//! the next refresh once completed.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::errors::{classify_io_error, SourceError};
use super::usage::{extract_record, LineContext, UsageRecord};

/// Cap on bytes parsed per file per refresh tick, so a huge backlog cannot
/// stall the UI thread group. The remainder is read on the next tick.
const CHUNK_BUDGET: u64 = 8 * 1024 * 1024;
/// Files larger than this are skipped with a note (protection against
/// accidentally watching a data dir that contains unrelated giants).
pub const MAX_TRACKED_JSONL: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct JsonlSourceState {
    pub path: PathBuf,
    pub offset: u64,
    pub session_hint: Option<String>,
    pub project_hint: Option<String>,
    pub records_read: u64,
    pub lines_skipped: u64,
    pub last_error: Option<String>,
}

impl JsonlSourceState {
    pub fn new(path: PathBuf) -> Self {
        let session_hint = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| s.len() >= 30) // looks like a uuid session id
            .map(|s| s.to_string());
        let project_hint = project_hint_from_path(&path);
        Self {
            path,
            offset: 0,
            session_hint,
            project_hint,
            records_read: 0,
            lines_skipped: 0,
            last_error: None,
        }
    }
}

/// `…/projects/<munged-project-dir>/<session>.jsonl` ⇒ project dir name.
/// ZCode munges path separators into `-`; we cannot un-munge reliably, so we
/// display the directory name as-is.
fn project_hint_from_path(path: &Path) -> Option<String> {
    let comps: Vec<_> = path.components().collect();
    for (i, c) in comps.iter().enumerate() {
        if c.as_os_str().to_string_lossy().eq_ignore_ascii_case("projects") {
            return comps.get(i + 1).map(|c| c.as_os_str().to_string_lossy().into_owned());
        }
    }
    None
}

/// Read all *new* complete lines from `state.path`, updating the watermark.
pub fn read_new(state: &mut JsonlSourceState) -> Result<Vec<UsageRecord>, SourceError> {
    state.last_error = None;
    let file = File::open(&state.path).map_err(|e| {
        let classified = classify_io_error(&e);
        state.last_error = Some(e.to_string());
        classified
    })?;
    let len = file.metadata().map_err(|e| classify_io_error(&e))?.len();
    if len > MAX_TRACKED_JSONL {
        return Err(SourceError::Fatal(format!(
            "file larger than tracking limit ({} bytes)",
            len
        )));
    }
    if len < state.offset {
        // File was truncated or replaced (log rotation, session cleanup):
        // restart from the beginning to stay consistent.
        state.offset = 0;
    }
    if len == state.offset {
        return Ok(Vec::new());
    }

    let mut reader = BufReader::new(file);
    reader
        .seek(SeekFrom::Start(state.offset))
        .map_err(|e| classify_io_error(&e))?;

    let budget = (len - state.offset).min(CHUNK_BUDGET);
    let mut raw = Vec::with_capacity(budget.min(4 * 1024 * 1024) as usize);
    reader
        .take(budget)
        .read_to_end(&mut raw)
        .map_err(|e| classify_io_error(&e))?;

    let ctx = LineContext {
        session_hint: state.session_hint.clone(),
        project_hint: state.project_hint.clone(),
        source_file: state.path.to_string_lossy().into_owned(),
    };

    let mut records = Vec::new();
    let mut consumed: u64 = 0;
    let mut start = 0usize;
    for (i, byte) in raw.iter().enumerate() {
        if *byte == b'\n' {
            let line = &raw[start..i];
            // Blank lines are fine to skip silently.
            if !line.iter().all(|b| b.is_ascii_whitespace()) {
                match serde_json::from_slice::<Value>(line) {
                    Ok(v) => {
                        if !v.is_object() {
                            // Valid JSON but not an object — a usage line is
                            // always an object, so treat it as garbage.
                            state.lines_skipped += 1;
                        } else {
                            match extract_record(&v, &ctx) {
                                Ok(Some(rec)) => records.push(rec),
                                Ok(None) => {}
                                Err(why) => {
                                    state.lines_skipped += 1;
                                    if state.lines_skipped <= 3 {
                                        state.last_error =
                                            Some(format!("skipped malformed line: {why}"));
                                    }
                                }
                            }
                        }
                    }
                    Err(_) => {
                        // Malformed JSON (or binary garbage). Count and move on —
                        // a reader must survive arbitrary trailing bytes.
                        state.lines_skipped += 1;
                    }
                }
            }
            consumed += (i - start + 1) as u64;
            start = i + 1;
        }
    }
    // `raw[start..]` (if any) is an incomplete trailing line: leave it for the
    // next refresh by not advancing the offset past `consumed` + start.
    state.offset += consumed;
    state.records_read += records.len() as u64;
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_jsonl(content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a4b3c2d1e0f4a5b6c7d8e9f0a1b2c3d4.jsonl");
        std::fs::write(&p, content).unwrap();
        (dir, p)
    }

    fn line(model: &str, input: u64, output: u64, ts: i64) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":{},"message":{{"model":"{}","usage":{{"input_tokens":{},"output_tokens":{}}}}}}}"#,
            ts, model, input, output
        )
    }

    #[test]
    fn incremental_reads_and_half_line_tolerance() {
        let (_dir, path) = tmp_jsonl("");
        let mut st = JsonlSourceState::new(path.clone());

        // 1. two complete lines
        std::fs::write(&path, format!("{}\n{}\n", line("m", 1, 1, 1756300800), line("m", 2, 2, 1756300860))).unwrap();
        assert_eq!(read_new(&mut st).unwrap().len(), 2);

        // 2. append a HALF line — must not error, must not yield a record
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        write!(f, r#"{{"type":"assistant","timestamp":1756300920,"mes"#).unwrap();
        assert_eq!(read_new(&mut st).unwrap().len(), 0);

        // 3. complete the line — now it appears
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        write!(f, r#"sage":{{"model":"m","usage":{{"input_tokens":3,"output_tokens":3}}}}}}"#).unwrap();
        write!(f, "\n").unwrap();
        let recs = read_new(&mut st).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].input_tokens, 3);

        // 4. no new data → empty
        assert!(read_new(&mut st).unwrap().is_empty());
    }

    #[test]
    fn truncation_resets_offset() {
        let (_dir, path) = tmp_jsonl(&format!("{}\n{}\n", line("m", 1, 1, 1756300800), line("m", 2, 2, 1756300860)));
        let mut st = JsonlSourceState::new(path.clone());
        assert_eq!(read_new(&mut st).unwrap().len(), 2);

        // Simulate rotation: file replaced by a shorter one.
        std::fs::write(&path, format!("{}\n", line("x", 9, 9, 1756301000))).unwrap();
        let recs = read_new(&mut st).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].model, "x");
    }

    #[test]
    fn garbage_lines_are_counted_not_fatal() {
        let (_dir, path) = tmp_jsonl(&format!(
            "not json at all\n{}\n\"just a string\"\n{}\n",
            line("m", 1, 1, 1756300800),
            line("m", 2, 2, 1756300860)
        ));
        let mut st = JsonlSourceState::new(path);
        let recs = read_new(&mut st).unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(st.lines_skipped, 2);
    }

    #[test]
    fn session_hint_from_uuid_filename() {
        let (_dir, path) = tmp_jsonl("");
        let st = JsonlSourceState::new(path);
        assert!(st.session_hint.is_some());
    }

    #[test]
    fn missing_file_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = JsonlSourceState::new(dir.path().join("nope-1111111111111111111111111111.jsonl"));
        assert!(matches!(read_new(&mut st), Err(SourceError::Gone)));
    }
}
