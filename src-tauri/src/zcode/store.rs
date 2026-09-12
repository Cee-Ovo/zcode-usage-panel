//! In-memory usage store.
//!
//! All parsed records live here, sorted by timestamp, together with an
//! incrementally maintained per-session accumulator map. Range queries are
//! served through binary-searched slices — no full-history rescans.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::aggregate::{Agg, SessionSummary};
use super::session_meta::SessionMetaEntry;
use super::usage::UsageRecord;

#[derive(Clone, Debug, Default)]
struct SessionAcc {
    project: Option<String>,
    models: Vec<String>,
    agg: Agg,
}

#[derive(Clone, Debug, Default)]
pub struct UsageStore {
    records: Vec<UsageRecord>,
    sessions: HashMap<String, SessionAcc>,
    /// Session metadata sidecar (titles / workspace dirs) keyed by session id.
    session_meta: HashMap<String, SessionMetaEntry>,
    sessions_cache: Option<Vec<SessionSummary>>,
    active_session: Option<(i64, String)>,
    pub total_ingested: u64,
    pub last_record_ms: Option<i64>,
    /// Set while the store is served from the persisted boot snapshot
    /// (before the first live ingest finishes).
    pub restored_from_cache: bool,
}

impl UsageStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn history_start_ms(&self) -> Option<i64> {
        self.records.first().map(|r| r.ts_ms)
    }

    /// Ingest a batch of freshly-read records.
    ///
    /// The common case (appended data, timestamps ≥ everything we have) is a
    /// cheap extend. Out-of-order batches (a record rewritten into the middle
    /// of history) trigger one re-sort.
    pub fn ingest(&mut self, mut batch: Vec<UsageRecord>) {
        if batch.is_empty() {
            return;
        }
        batch.sort_by_key(|r| r.ts_ms);
        let batch_min = batch[0].ts_ms;
        let batch_max = batch[batch.len() - 1].ts_ms;
        let batch_len = batch.len();
        for r in &batch {
            self.ingest_into_sessions(r);
        }
        let ok_to_extend = self
            .records
            .last()
            .map(|last| batch_min >= last.ts_ms)
            .unwrap_or(true);
        if ok_to_extend {
            self.records.extend(batch);
        } else {
            self.records.extend(batch);
            self.records.sort_by_key(|r| r.ts_ms);
        }
        self.sessions_cache = None;
        self.last_record_ms = self.last_record_ms.map_or(Some(batch_max), |m| Some(m.max(batch_max)));
        self.total_ingested += batch_len as u64;
    }

    /// Records in `[from_ms, to_ms]` via binary search.
    pub fn range(&self, from_ms: i64, to_ms: i64) -> &[UsageRecord] {
        if from_ms > to_ms {
            return &[];
        }
        let start = self.records.partition_point(|r| r.ts_ms < from_ms);
        let end = self.records.partition_point(|r| r.ts_ms <= to_ms);
        &self.records[start..end]
    }

    pub fn all(&self) -> &[UsageRecord] {
        &self.records
    }

    pub fn session_summaries(&mut self) -> &[SessionSummary] {
        if self.sessions_cache.is_none() {
            let mut list: Vec<SessionSummary> = self
                .sessions
                .iter()
                .map(|(id, acc)| {
                    let (title, project, project_path) = self.resolve_meta_fields(id, acc);
                    SessionSummary {
                        id: id.clone(),
                        title,
                        project,
                        project_path,
                        models: acc.models.clone(),
                        agg: acc.agg.clone(),
                    }
                })
                .collect();
            list.sort_by(|a, b| b.agg.last_ts_ms.cmp(&a.agg.last_ts_ms).then_with(|| a.id.cmp(&b.id)));
            self.sessions_cache = Some(list);
        }
        self.sessions_cache.as_deref().unwrap()
    }

    /// The most recently active session id, if any.
    pub fn active_session_id(&self) -> Option<String> {
        self.active_session.as_ref().map(|(_, id)| id.clone())
    }

    /// Direct lookup avoids rebuilding/sorting the entire sessions list for
    /// every live dashboard refresh or single-session detail request.
    pub fn session_summary(&self, id: &str) -> Option<SessionSummary> {
        self.sessions.get(id).map(|acc| {
            let (title, project, project_path) = self.resolve_meta_fields(id, acc);
            SessionSummary {
                id: id.to_string(),
                title,
                project,
                project_path,
                models: acc.models.clone(),
                agg: acc.agg.clone(),
            }
        })
    }

    /// Merge the session-metadata sidecar. Returns true when anything changed
    /// (cached summaries are invalidated so the new titles/projects surface).
    pub fn apply_session_meta(&mut self, meta: HashMap<String, SessionMetaEntry>) -> bool {
        if self.session_meta == meta {
            return false;
        }
        self.session_meta = meta;
        self.sessions_cache = None;
        true
    }

    /// Resolve a session's display fields: record-level project (JSONL
    /// sources) wins, then the metadata sidecar's real folder name.
    fn resolve_meta_fields(
        &self,
        id: &str,
        acc: &SessionAcc,
    ) -> (Option<String>, Option<String>, Option<String>) {
        let meta = self.session_meta.get(id);
        (
            meta.and_then(|m| m.title.clone()),
            acc.project
                .clone()
                .or_else(|| meta.and_then(|m| m.project_name.clone())),
            meta.and_then(|m| m.directory.clone()),
        )
    }

    /// Usage records of one session within a time window (used for the
    /// "current session" live card and its trend).
    pub fn session_records(&self, session_id: &str, from_ms: i64) -> Vec<UsageRecord> {
        let start = self.records.partition_point(|r| r.ts_ms < from_ms);
        self.records[start..]
            .iter()
            .filter(|r| r.session_id.as_deref() == Some(session_id))
            .cloned()
            .collect()
    }

    pub fn all_model_names(&self) -> Vec<String> {
        let mut seen = std::collections::BTreeSet::new();
        for r in &self.records {
            seen.insert(r.model.clone());
        }
        seen.into_iter().collect()
    }

    /// Drop every record that came from one of the `gone` source files and
    /// rebuild the derived session accumulators from what is left.
    ///
    /// Used when a source file disappears from the data root (ZCode session
    /// cleanup, a manually deleted log) so the deleted history stops counting
    /// right away instead of at the next process restart. `total_ingested` is
    /// deliberately left alone: it counts how much has been *read*, not what
    /// is currently held, and the UI treats it as a monotonic revision.
    pub fn drop_source_files(&mut self, gone: &HashSet<std::path::PathBuf>) -> usize {
        if gone.is_empty() {
            return 0;
        }
        let before = self.records.len();
        self.records
            .retain(|r| !gone.contains(Path::new(r.source_file.as_str())));
        let removed = before - self.records.len();
        if removed > 0 {
            self.rebuild_sessions();
        }
        removed
    }

    /// Clear all state. Used when the configured data root changes: records
    /// from the old root must not mix with (nor be double-counted alongside)
    /// the new one, and the per-file watermarks that go with them are dropped
    /// by the caller.
    pub fn reset(&mut self) {
        self.records.clear();
        self.sessions.clear();
        self.session_meta.clear();
        self.sessions_cache = None;
        self.active_session = None;
        self.last_record_ms = None;
        self.total_ingested = 0;
        self.restored_from_cache = false;
    }

    /// Rebuild `sessions` / `active_session` / `last_record_ms` from `records`.
    /// `records` is already ts-sorted, so the accumulators can be replayed in
    /// order and the running max is simply the last timestamp.
    fn rebuild_sessions(&mut self) {
        let records = std::mem::take(&mut self.records);
        self.sessions.clear();
        self.active_session = None;
        self.sessions_cache = None;
        for r in &records {
            self.ingest_into_sessions(r);
        }
        self.last_record_ms = records.last().map(|r| r.ts_ms);
        self.records = records;
    }

    pub(crate) fn ingest_into_sessions(&mut self, rec: &UsageRecord) {
        if let Some(sid) = &rec.session_id {
            let replace = self.active_session.as_ref().map_or(true, |(ts, id)| {
                rec.ts_ms > *ts || (rec.ts_ms == *ts && sid < id)
            });
            if replace {
                self.active_session = Some((rec.ts_ms, sid.clone()));
            }
            let acc = self.sessions.entry(sid.clone()).or_default();
            if acc.project.is_none() {
                acc.project = rec.project.clone();
            }
            if !acc.models.contains(&rec.model) {
                acc.models.push(rec.model.clone());
                acc.models.sort();
            }
            acc.agg.add(rec);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(ts: i64, model: &str, session: &str) -> UsageRecord {
        UsageRecord {
            ts_ms: ts,
            model: model.into(),
            session_id: Some(session.into()),
            project: Some("p".into()),
            input_tokens: 10,
            output_tokens: 5,
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            duration_ms: None,
            ttft_ms: None,
            status: None,
            total_override: None,
            reasoning_in_output: false,
            schema_exclusive: None,
            duration_derived: false,
            source_file: "t".into(),
        }
    }

    #[test]
    fn range_slice_is_correct() {
        let mut st = UsageStore::new();
        st.ingest((0..100).map(|i| rec(i * 1000, "m", "s")).collect());
        assert_eq!(st.range(0, 1000).len(), 2);
        assert_eq!(st.range(50_000, 60_000).len(), 11);
        assert_eq!(st.range(1_000_000, 2_000_000).len(), 0);
    }

    #[test]
    fn out_of_order_batch_is_sorted() {
        let mut st = UsageStore::new();
        st.ingest(vec![rec(5000, "m", "s"), rec(6000, "m", "s")]);
        st.ingest(vec![rec(1000, "m", "s")]);
        assert_eq!(st.all()[0].ts_ms, 1000);
    }

    #[test]
    fn direct_session_lookup_matches_sorted_list_without_building_cache() {
        let mut st = UsageStore::new();
        st.ingest(vec![rec(5000, "m", "z"), rec(5000, "m", "a"), rec(1000, "m", "old")]);
        assert_eq!(st.active_session_id().as_deref(), Some("a"));
        assert_eq!(st.session_summary("a").unwrap().agg.requests, 1);
        assert!(st.session_summary("missing").is_none());
        assert!(st.sessions_cache.is_none());
        assert_eq!(st.session_summaries()[0].id, "a");
        st.ingest(vec![rec(6000, "m", "old")]);
        assert_eq!(st.active_session_id().as_deref(), Some("old"));
        assert_eq!(st.session_summary("old").unwrap().agg.requests, 2);
        assert!(st.sessions_cache.is_none());
    }

    #[test]
    fn inverted_range_is_empty() {
        let mut st = UsageStore::new();
        st.ingest(vec![rec(1000, "m", "s")]);
        assert!(st.range(2000, 0).is_empty());
    }

    fn meta(id: &str, title: Option<&str>, dir: Option<&str>) -> (String, SessionMetaEntry) {
        (
            id.into(),
            SessionMetaEntry {
                title: title.map(str::to_string),
                directory: dir.map(str::to_string),
                project_name: dir.and_then(|d| {
                    let t = d.trim_end_matches('/');
                    t.rsplit('/').next().map(|s| s.to_string())
                }),
            },
        )
    }

    #[test]
    fn session_meta_enriches_summaries_and_invalidates_cache() {
        // sqlite-sourced sessions: records carry no project of their own.
        let bare = |ts: i64, model: &str, session: &str| UsageRecord {
            project: None,
            ..rec(ts, model, session)
        };
        let mut st = UsageStore::new();
        st.ingest(vec![bare(5000, "glm-5.3", "sess-1"), bare(6000, "m", "sess-2")]);

        let before = st.session_summaries().to_vec();
        assert!(before.iter().all(|s| s.title.is_none() && s.project.is_none()));

        let mut m = std::collections::HashMap::new();
        m.insert("sess-1".to_string(), meta("sess-1", Some("优化登录性能"), Some("/home/u/projects/panel")).1);
        assert!(st.apply_session_meta(m), "changed meta must report true");
        assert!(st.session_summary("sess-1").is_some());

        let s1 = st.session_summary("sess-1").unwrap();
        assert_eq!(s1.title.as_deref(), Some("优化登录性能"));
        assert_eq!(s1.project.as_deref(), Some("panel"));
        assert_eq!(s1.project_path.as_deref(), Some("/home/u/projects/panel"));
        // sess-2 has no metadata row → stays honestly empty.
        let s2 = st.session_summary("sess-2").unwrap();
        assert_eq!(s2.title, None);
        assert_eq!(s2.project_path, None);

        // Same map again → no change, cache stays warm.
        let mut same = std::collections::HashMap::new();
        same.insert("sess-1".to_string(), meta("sess-1", Some("优化登录性能"), Some("/home/u/projects/panel")).1);
        st.session_summaries(); // warm the cache
        assert!(!st.apply_session_meta(same), "identical meta must report false");
        assert!(st.sessions_cache.is_some(), "cache not invalidated by no-op");
    }

    #[test]
    fn record_level_project_wins_over_meta_folder() {
        // JSONL sources carry a real project string on the record itself.
        let mut st = UsageStore::new();
        let mut r = rec(5000, "m", "sess-1");
        r.project = Some("proj-A".into());
        st.ingest(vec![r]);
        let mut m = std::collections::HashMap::new();
        m.insert("sess-1".to_string(), meta("sess-1", Some("t"), Some("/x/panel")).1);
        st.apply_session_meta(m);
        let s = st.session_summary("sess-1").unwrap();
        assert_eq!(s.project.as_deref(), Some("proj-A"));
        assert_eq!(s.project_path.as_deref(), Some("/x/panel"));
    }

    fn rec_in(ts: i64, model: &str, session: &str, file: &str) -> UsageRecord {
        UsageRecord { source_file: file.into(), ..rec(ts, model, session) }
    }

    fn files(names: &[&str]) -> HashSet<std::path::PathBuf> {
        names.iter().map(std::path::PathBuf::from).collect()
    }

    /// 源文件消失后,来自它的记录必须立刻不再计入,且会话聚合要跟着重建。
    #[test]
    fn drop_source_files_removes_records_and_rebuilds_sessions() {
        let mut st = UsageStore::new();
        st.ingest(vec![
            rec_in(1000, "a", "s1", "f1.jsonl"),
            rec_in(2000, "b", "s2", "f2.jsonl"),
            rec_in(3000, "a", "s1", "f1.jsonl"),
        ]);
        assert_eq!(st.len(), 3);
        assert_eq!(st.session_summary("s1").unwrap().agg.requests, 2);

        assert_eq!(st.drop_source_files(&files(&["f1.jsonl"])), 2);
        assert_eq!(st.len(), 1);
        assert!(st.session_summary("s1").is_none(), "被删文件的会话必须消失");
        assert_eq!(st.session_summary("s2").unwrap().agg.requests, 1);
        assert_eq!(st.all_model_names(), vec!["b"]);
        assert_eq!(st.last_record_ms, Some(2000));
        // 累计读数语义是"读过多少",不回退。
        assert_eq!(st.total_ingested, 3);

        // 空集合与无匹配路径都是安全的 no-op。
        assert_eq!(st.drop_source_files(&HashSet::new()), 0);
        assert_eq!(st.drop_source_files(&files(&["nope.jsonl"])), 0);
        assert_eq!(st.len(), 1);
    }

    /// 数据根变更时的整体清空:记录、会话、元数据与所有派生状态都要归零,
    /// 否则切换目录后会拿旧目录的数字顶上。
    #[test]
    fn reset_clears_records_sessions_meta_and_derived_state() {
        let mut st = UsageStore::new();
        st.ingest(vec![rec_in(1000, "a", "s1", "f1.jsonl")]);
        let mut m = std::collections::HashMap::new();
        m.insert("s1".to_string(), meta("s1", Some("t"), Some("/x/panel")).1);
        st.apply_session_meta(m);
        st.restored_from_cache = true;
        st.session_summaries(); // 预热缓存

        st.reset();

        assert!(st.is_empty());
        assert_eq!(st.len(), 0);
        assert!(st.history_start_ms().is_none());
        assert_eq!(st.last_record_ms, None);
        assert_eq!(st.total_ingested, 0);
        assert!(!st.restored_from_cache);
        assert!(st.session_summaries().is_empty());
        assert!(st.active_session_id().is_none());
        assert!(st.all_model_names().is_empty());
    }
}
