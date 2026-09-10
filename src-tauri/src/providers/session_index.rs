//! Unified multi-source session index.
//!
//! The Sessions page shows one merged list where ZCode sessions keep their
//! raw ids and every other source gets a stable display prefix on the id
//! (`cx-` Codex, `cc-` Claude Code, `dsh-` DSH) so users can tell sources
//! apart at a glance while sorting, searching and detail views behave
//! identically for all of them.
//!
//! Every source contributes per-file accumulators whose `records` are
//! already in the shared [`UsageRecord`] schema, so summaries, detail
//! buckets and per-model stats are folded with the *same* honest-caliber
//! aggregation the ZCode engine uses — there is no per-source math to
//! drift. Fields a source does not record stay `None` and the UI degrades
//! to "—" honestly.

use std::collections::{BTreeSet, HashMap};

use crate::commands::{ActiveSession, DashboardDto, ModelSwitch, UsageViewDto};
use crate::zcode::aggregate::{
    bucketize, compute_speed_stats, group_by_model, resolve_span, Agg, ModelStat, SessionSummary,
    TrendRange,
};
use crate::zcode::pricing::PricingManager;
use crate::zcode::session_meta::folder_name;
use crate::zcode::usage::UsageRecord;

use super::local_usage::SessionUsage;
use super::{PROVIDER_CLAUDE_CODE, PROVIDER_CODEX, PROVIDER_DSH};

/// Display prefixes added to non-ZCode session ids (never stored in the
/// sources' own caches — applied at the index boundary).
pub const PREFIX_CODEX: &str = "cx-";
pub const PREFIX_CLAUDE_CODE: &str = "cc-";
pub const PREFIX_DSH: &str = "dsh-";

/// Split a prefixed session id back into `(provider id, raw session id)`.
/// ZCode ids carry no prefix and are returned as `None`.
pub fn split_prefixed(session_id: &str) -> Option<(&'static str, &str)> {
    if let Some(raw) = session_id.strip_prefix(PREFIX_CODEX) {
        Some((PROVIDER_CODEX, raw))
    } else if let Some(raw) = session_id.strip_prefix(PREFIX_CLAUDE_CODE) {
        Some((PROVIDER_CLAUDE_CODE, raw))
    } else if let Some(raw) = session_id.strip_prefix(PREFIX_DSH) {
        Some((PROVIDER_DSH, raw))
    } else {
        None
    }
}

/// One session's worth of contributions from a provider's files. Several
/// files may reference the same session id (resumed / split rollouts); the
/// index merges them.
#[derive(Clone, Copy)]
pub struct SessionContrib<'a> {
    pub session_id: &'a str,
    pub title: Option<&'a str>,
    pub project_path: Option<&'a str>,
    pub records: &'a [UsageRecord],
}

impl<'a> SessionContrib<'a> {
    pub fn from_usage(s: &'a SessionUsage) -> Self {
        Self {
            session_id: &s.session_id,
            title: s.title.as_deref(),
            project_path: s.project_path.as_deref(),
            records: &s.records,
        }
    }

    fn usable(&self) -> bool {
        !self.session_id.is_empty() || !self.records.is_empty()
    }
}

#[derive(Default)]
struct MergedSession {
    title: Option<String>,
    project_path: Option<String>,
    models: BTreeSet<String>,
    agg: Agg,
}

/// Fold one provider's contributions into prefixed session summaries,
/// sorted most-recently-active first (the same ordering the ZCode store
/// emits, so the merged page mixes sources consistently).
pub fn build_summaries(prefix: &str, contribs: &[SessionContrib<'_>]) -> Vec<SessionSummary> {
    let mut merged: HashMap<&str, MergedSession> = HashMap::new();
    for c in contribs.iter().filter(|c| c.usable()) {
        let entry = merged.entry(c.session_id).or_default();
        if entry.title.is_none() {
            entry.title = c.title.map(str::to_string);
        }
        if entry.project_path.is_none() {
            entry.project_path = c.project_path.map(str::to_string);
        }
        for r in c.records {
            entry.models.insert(r.model.clone());
            entry.agg.add(r);
        }
    }
    let mut out: Vec<SessionSummary> = merged
        .into_iter()
        .map(|(raw_id, m)| {
            let project_path = m.project_path;
            SessionSummary {
                id: format!("{prefix}{raw_id}"),
                title: m.title,
                project: project_path.as_deref().and_then(folder_name),
                project_path,
                models: m.models.into_iter().collect(),
                agg: m.agg,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.agg.last_ts_ms
            .cmp(&a.agg.last_ts_ms)
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

/// Detail-view parts for one (already resolved) session: time buckets over
/// the session's own span plus per-model stats — the same shapes as the
/// ZCode session detail.
pub struct SessionDetailParts {
    pub summary: SessionSummary,
    pub buckets: Vec<crate::zcode::aggregate::Bucket>,
    pub models: Vec<ModelStat>,
}

/// Build the detail for one raw session id from its contributions. The
/// `prefix` is re-attached to the summary id so the frontend can keep using
/// the id it selected.
pub fn build_detail(prefix: &str, raw_id: &str, contribs: &[SessionContrib<'_>]) -> Option<SessionDetailParts> {
    let mine: Vec<SessionContrib<'_>> = contribs
        .iter()
        .filter(|c| c.session_id == raw_id)
        .copied()
        .collect();
    if mine.is_empty() {
        return None;
    }
    let summary = build_summaries(prefix, &mine).into_iter().next()?;

    let mut records: Vec<&UsageRecord> = mine.iter().flat_map(|c| c.records.iter()).collect();
    records.sort_by_key(|r| r.ts_ms);
    let from = summary.agg.first_ts_ms?;
    let to = summary.agg.last_ts_ms?;
    let owned: Vec<UsageRecord> = records.into_iter().cloned().collect();
    let buckets = bucketize(&owned, from, to + 1, 48.min(owned.len().max(1)));
    let models = group_by_model(&owned);
    Some(SessionDetailParts { summary, buckets, models })
}

// ---------------------------------------------------------------------------
// ZCode-density dashboard view for one local source
// ---------------------------------------------------------------------------

/// Filter a ts-sorted record history to a named range, resolving the span
/// against the records' own first timestamp (so "all" means *this source's*
/// full history, never another store's).
pub fn records_for_range(records: &[UsageRecord], range_key: &str, now_ms: i64) -> Vec<UsageRecord> {
    let range = TrendRange::from_key(range_key).unwrap_or(TrendRange::TodayHourly);
    let history_start_ms = records.first().map(|r| r.ts_ms);
    let (from, to, _) = resolve_span(range, now_ms, history_start_ms);
    records
        .iter()
        .filter(|r| r.ts_ms >= from && r.ts_ms <= to)
        .cloned()
        .collect()
}

/// Build the dashboard/trend/cost view for a local source from its complete
/// record history. Reuses the exact ZCode section shapes (and its model-row
/// helper) so all four dashboard sections share one presentation structure;
/// metrics the source cannot know (TTFT / tok-per-sec) come back as honest
/// all-`None` speed stats instead of being invented.
pub fn build_usage_view(
    records: &[UsageRecord],
    latest_session: Option<SessionSummary>,
    latest_records: &[UsageRecord],
    range_key: &str,
    include_trend: bool,
    pricing: &PricingManager,
    now_ms: i64,
) -> UsageViewDto {
    let range = TrendRange::from_key(range_key).unwrap_or(TrendRange::TodayHourly);
    let history_start_ms = records.first().map(|r| r.ts_ms);
    let (from, to, buckets) = resolve_span(range, now_ms, history_start_ms);
    let in_range: Vec<UsageRecord> = records
        .iter()
        .filter(|r| r.ts_ms >= from && r.ts_ms <= to)
        .cloned()
        .collect();

    let agg = in_range.iter().fold(Agg::default(), |mut a, r| {
        a.add(r);
        a
    });
    let models = crate::commands::model_rows(&in_range);
    let speed = compute_speed_stats(&in_range);
    let active = latest_session
        .as_ref()
        .map(|s| local_active_session(s, latest_records, now_ms));

    let dash = DashboardDto {
        range_key: range.key().to_string(),
        from_ms: from,
        to_ms: to,
        agg,
        models,
        active_session: active,
        speed,
        restored: false,
        data_error: None,
    };
    let trend = include_trend.then(|| crate::commands::TrendDto {
        range_key: range.key().to_string(),
        from_ms: from,
        to_ms: to,
        buckets: bucketize(&in_range, from, to, buckets),
        restored: false,
    });
    let cost_summary = pricing.cost_summary(range.key(), &in_range);
    UsageViewDto {
        dash,
        trend,
        cost_summary,
        revision: in_range.len() as u64,
    }
}

/// "Current session" card for a local source: the most recently active
/// session, its 5-minute growth, and model switches — all derived from
/// recorded events only.
fn local_active_session(latest: &SessionSummary, latest_records: &[UsageRecord], now: i64) -> ActiveSession {
    let last_5m: u64 = latest_records
        .iter()
        .filter(|r| r.ts_ms >= now - 5 * 60_000)
        .map(|r| r.display_total_tokens())
        .sum();

    // Model switch log: unique-model transitions in chronological order.
    let mut switches: Vec<ModelSwitch> = Vec::new();
    for r in latest_records
        .iter()
        .filter(|r| r.ts_ms >= now - 30 * 60_000)
    {
        if switches.last().map(|s| s.model != r.model).unwrap_or(true) {
            switches.push(ModelSwitch { ts_ms: r.ts_ms, model: r.model.clone() });
        }
        if switches.len() >= 50 {
            break;
        }
    }
    ActiveSession {
        session_id: latest.id.clone(),
        project: latest.project.clone(),
        session_total_tokens: latest.agg.total_tokens(),
        session_agg: latest.agg.clone(),
        tokens_last_5m: last_5m,
        tokens_per_min: (last_5m as f64) / 5.0,
        last_request_ms: latest.agg.last_ts_ms,
        active_model: latest_records
            .last()
            .map(|r| r.model.clone())
            .or_else(|| latest.models.last().cloned()),
        model_switches: switches,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{PROVIDER_CLAUDE_CODE, PROVIDER_CODEX, PROVIDER_DSH};

    fn rec(ts: i64, model: &str, input: u64, output: u64, cr: Option<u64>) -> UsageRecord {
        UsageRecord {
            ts_ms: ts,
            model: model.into(),
            session_id: Some("s".into()),
            project: None,
            input_tokens: input,
            output_tokens: output,
            reasoning_tokens: None,
            cache_read_tokens: cr,
            cache_write_tokens: None,
            duration_ms: None,
            ttft_ms: None,
            status: None,
            total_override: None,
            reasoning_in_output: false,
            schema_exclusive: Some(true),
            duration_derived: false,
            source_file: "t".into(),
        }
    }

    #[test]
    fn summaries_carry_prefix_merge_files_and_derive_project_folder() {
        let a = vec![
            rec(10, "gpt-5.6-sol", 100, 20, Some(5)),
            rec(20, "gpt-5.6-sol", 30, 40, None),
        ];
        // Same session id in a second (resumed) file — merged, not duplicated.
        let b = vec![rec(30, "gpt-5.6-luna", 50, 10, Some(0))];
        let contribs = [
            SessionContrib {
                session_id: "abc",
                title: None,
                project_path: Some("C:\\Users\\27632\\Desktop\\zcode-panel"),
                records: &a,
            },
            SessionContrib {
                session_id: "abc",
                title: Some("重构额度卡"),
                project_path: None,
                records: &b,
            },
            SessionContrib {
                session_id: "",
                title: None,
                project_path: None,
                records: &[],
            },
        ];
        let out = build_summaries(PREFIX_CODEX, &contribs);
        assert_eq!(out.len(), 1);
        let s = &out[0];
        assert_eq!(s.id, "cx-abc");
        assert_eq!(s.title.as_deref(), Some("重构额度卡"));
        // Windows separators: the folder name is the real last segment.
        assert_eq!(s.project.as_deref(), Some("zcode-panel"));
        assert_eq!(
            s.project_path.as_deref(),
            Some("C:\\Users\\27632\\Desktop\\zcode-panel")
        );
        assert_eq!(s.models, vec!["gpt-5.6-luna", "gpt-5.6-sol"]);
        assert_eq!(s.agg.requests, 3);
        // Exclusive schema: cache added on top; second record has no cache.
        assert_eq!(s.agg.total_tokens(), 100 + 20 + 5 + 30 + 40 + 50 + 10);
    }

    #[test]
    fn summaries_sort_by_recency_and_split_prefixes_back() {
        let older = vec![rec(10, "m", 1, 1, None)];
        let newer = vec![rec(99, "m", 2, 2, None)];
        let contribs = [
            SessionContrib { session_id: "old", title: None, project_path: None, records: &older },
            SessionContrib { session_id: "new", title: None, project_path: None, records: &newer },
        ];
        let out = build_summaries(PREFIX_DSH, &contribs);
        assert_eq!(out[0].id, "dsh-new");
        assert_eq!(out[1].id, "dsh-old");

        assert_eq!(split_prefixed("dsh-new"), Some((PROVIDER_DSH, "new")));
        assert_eq!(split_prefixed("cx-a"), Some((PROVIDER_CODEX, "a")));
        assert_eq!(split_prefixed("cc-a"), Some((PROVIDER_CLAUDE_CODE, "a")));
        assert_eq!(split_prefixed("sess_zcode"), None);
        // Raw ids that themselves start with a prefix-like token are not
        // double-prefixed; splitting always strips exactly one level.
        let tricky = vec![rec(5, "m", 1, 1, None)];
        let t = [SessionContrib { session_id: "cc-real", title: None, project_path: None, records: &tricky }];
        let out = build_summaries(PREFIX_CLAUDE_CODE, &t);
        assert_eq!(out[0].id, "cc-cc-real");
        assert_eq!(split_prefixed("cc-cc-real"), Some((PROVIDER_CLAUDE_CODE, "cc-real")));
    }

    #[test]
    fn detail_buckets_models_and_unknown_id() {
        let records = vec![
            rec(1_000, "deepseek-chat", 10, 5, Some(3)),
            rec(60_000, "deepseek-reasoner", 20, 8, None),
        ];
        let contribs = [SessionContrib {
            session_id: "sess-1",
            title: Some("调研"),
            project_path: Some("/home/u/projects/panel"),
            records: &records,
        }];
        let detail = build_detail(PREFIX_DSH, "sess-1", &contribs).expect("known session");
        assert_eq!(detail.summary.id, "dsh-sess-1");
        assert_eq!(detail.summary.agg.requests, 2);
        assert_eq!(detail.models.len(), 2);
        assert_eq!(detail.buckets.len(), 2);
        assert!(detail.buckets.iter().all(|b| b.agg.requests <= 1));
        assert!(build_detail(PREFIX_DSH, "missing", &contribs).is_none());
    }
}
