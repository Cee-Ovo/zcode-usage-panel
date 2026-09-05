//! In-memory usage store.
//!
//! All parsed records live here, sorted by timestamp, together with an
//! incrementally maintained per-session accumulator map. Range queries are
//! served through binary-searched slices — no full-history rescans.

use std::collections::HashMap;

use super::aggregate::{Agg, SessionSummary};
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
                .map(|(id, acc)| SessionSummary {
                    id: id.clone(),
                    project: acc.project.clone(),
                    models: acc.models.clone(),
                    agg: acc.agg.clone(),
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
        self.sessions.get(id).map(|acc| SessionSummary {
            id: id.to_string(),
            project: acc.project.clone(),
            models: acc.models.clone(),
            agg: acc.agg.clone(),
        })
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
}
