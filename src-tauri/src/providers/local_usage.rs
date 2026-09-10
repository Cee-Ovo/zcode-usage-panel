//! Shared "local harness token usage" machinery for offline session-log
//! providers (Codex rollouts, DeepSeek Harness sessions, Claude Code
//! transcripts).
//!
//! Every provider keeps per-file parse state (watermarks + a `SessionUsage`
//! accumulator) and derives the same six rolling ranges from timestamped
//! usage events, so the UI can render every local-usage section with one
//! component. Anything provider-specific (wire formats, cumulative-counter
//! quirks, account claims) stays in the provider module.
//!
//! On top of the rollups, each provider stores its full event history as
//! [`UsageRecord`]s — the same honest-caliber record model the ZCode engine
//! uses. That single representation powers the multi-source Sessions page,
//! per-session detail views, range dashboards and cost estimates through the
//! exact same aggregation code paths (`Agg` / `bucketize` / pricing), so no
//! source ever gets its own (drifted) math. History is retained unpruned:
//! personal-scale harness logs are small, and pruning would silently blank
//! old sessions' detail views.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::{LocalUsage, LocalUsageRange, ModelUsageRow, TokenBreakdown};
use crate::zcode::usage::UsageRecord;

pub const RANGE_KEYS: [&str; 6] = ["today", "60m", "24h", "7d", "30d", "all"];

/// Raw per-message token counters as recorded by a harness. Field semantics
/// belong to the provider; `total_tokens` is whatever the source reports (or
/// the provider's documented sum) and is never re-derived here.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TotalTokenUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    #[serde(default)]
    pub cache_write_input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub reasoning_output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
}

impl TotalTokenUsage {
    pub fn add(&mut self, delta: &TotalTokenUsage) {
        self.input_tokens += delta.input_tokens;
        self.cached_input_tokens += delta.cached_input_tokens;
        self.cache_write_input_tokens += delta.cache_write_input_tokens;
        self.output_tokens += delta.output_tokens;
        self.reasoning_output_tokens += delta.reasoning_output_tokens;
        self.total_tokens += delta.total_tokens;
    }
}

/// How one provider's event deltas map onto the shared [`UsageRecord`]
/// schema. These flags are the *documented* semantics of the source format —
/// they keep totals/hit-rates caliber-aware instead of guessed per record.
#[derive(Clone, Copy, Debug)]
pub struct DeltaSchema {
    /// `true` when the source counts reasoning inside `output_tokens`
    /// (DSH) — display totals and generated-token math must not add it again.
    pub reasoning_in_output: bool,
    /// `true` when the source (or its documented disjoint-field sum)
    /// provides a trustworthy per-event total — recorded as
    /// `total_override` so the record's own caliber always wins.
    pub total_override: bool,
    /// Whether `input_tokens` excludes the cache tokens (provider-
    /// documented). Known schemas must not rely on the numeric heuristic —
    /// a genuinely exclusive source with a large fresh input would be
    /// misread as inclusive and its cache silently dropped.
    pub input_exclusive: bool,
}

/// Codex rollouts: OpenAI-style inclusive input (contains cached tokens) and
/// a source-provided `total_tokens` per cumulative counter delta.
pub const CODEX_DELTA: DeltaSchema = DeltaSchema {
    reasoning_in_output: false,
    total_override: true,
    input_exclusive: false,
};

/// DSH session logs: disjoint fields (`inputTokens` excludes cache,
/// `outputTokens` already contains reasoning); the total is the documented
/// sum of the disjoint fields.
pub const DSH_DELTA: DeltaSchema = DeltaSchema {
    reasoning_in_output: true,
    total_override: true,
    input_exclusive: true,
};

/// Map one parsed event delta onto the shared record schema. The cache /
/// reasoning fields of these harness schemas are always present (possibly
/// zero), so they become `Some(...)` — that is what feeds hit-rate coverage.
pub fn delta_record(
    delta: &TotalTokenUsage,
    schema: DeltaSchema,
    ts_ms: i64,
    model: &str,
    session_id: &str,
    project: Option<&str>,
    source_file: &str,
    // Approximate request wall time derived from event timestamps
    // (`usage_ts − last input item ts`), when the source provides it.
    // `ttft_ms` stays `None` — that moment is genuinely unrecorded.
    duration_ms: Option<u64>,
) -> UsageRecord {
    UsageRecord {
        ts_ms,
        model: model.to_string(),
        session_id: (!session_id.is_empty()).then(|| session_id.to_string()),
        project: project.map(str::to_string),
        input_tokens: delta.input_tokens,
        output_tokens: delta.output_tokens,
        reasoning_tokens: Some(delta.reasoning_output_tokens),
        cache_read_tokens: Some(delta.cached_input_tokens),
        cache_write_tokens: Some(delta.cache_write_input_tokens),
        duration_ms,
        duration_derived: duration_ms.is_some(),
        ttft_ms: None,
        status: None,
        total_override: schema.total_override.then(|| delta.total_tokens),
        reasoning_in_output: schema.reasoning_in_output,
        schema_exclusive: Some(schema.input_exclusive),
        source_file: source_file.to_string(),
    }
}

/// Inverse mapping for the rolling-range rollups: one record's contribution
/// as raw counters. `total_tokens` mirrors the record's display total so the
/// LocalUsage numbers and the record-based dashboards can never disagree.
pub fn record_delta(r: &UsageRecord) -> TotalTokenUsage {
    TotalTokenUsage {
        input_tokens: r.input_tokens,
        cached_input_tokens: r.cache_read_tokens.unwrap_or(0),
        cache_write_input_tokens: r.cache_write_tokens.unwrap_or(0),
        output_tokens: r.output_tokens,
        reasoning_output_tokens: r.reasoning_tokens.unwrap_or(0),
        total_tokens: r.display_total_tokens(),
    }
}

/// Accumulated usage for one session log file.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SessionUsage {
    pub session_id: String,
    pub model: String,
    pub first_ts_ms: i64,
    pub last_ts_ms: i64,
    /// Number of usage events (= model responses) in this session.
    pub responses: u64,
    /// Latest cumulative counters (providers with cumulative sources use this
    /// as the incremental baseline; per-event sources leave it at default).
    #[serde(default)]
    pub totals: TotalTokenUsage,
    /// Exact accumulated deltas for this file (works across counter resets).
    #[serde(default)]
    pub all_time: TotalTokenUsage,
    /// Exact accumulated deltas by the model active at each usage event.
    #[serde(default)]
    pub model_totals: HashMap<String, TotalTokenUsage>,
    #[serde(default)]
    pub model_requests: HashMap<String, u64>,
    /// Full per-event history in the shared record schema (sorted by
    /// arrival; providers with out-of-order appends keep them as written).
    /// Kept unpruned — see the module docs.
    #[serde(default)]
    pub records: Vec<UsageRecord>,
    /// Real session title when the source provides one (summary line / first
    /// user message); `None` = honest degradation, never fabricated.
    #[serde(default)]
    pub title: Option<String>,
    /// Workspace directory (the source's recorded `cwd`) when available.
    #[serde(default)]
    pub project_path: Option<String>,
    /// Timestamp of the request-input item that started the current request
    /// (Codex rollouts). Anchors the approximate per-request duration: usage
    /// events are flushed *after* the turn's tool calls finish, so they can
    /// never bound the request on their own.
    #[serde(default)]
    pub pending_input_ms: Option<i64>,
    /// Timestamp of the last response-output item seen (Codex rollouts) —
    /// the moment the model stopped streaming this request. Paired with
    /// `pending_input_ms` it brackets the generation window.
    #[serde(default)]
    pub pending_output_ms: Option<i64>,
}

/// Byte-watermark parse state for one append-only log file.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FileEntry {
    /// Byte watermark — append-only logs never re-parse unchanged prefixes.
    pub offset: u64,
    pub complete: bool,
    pub session: SessionUsage,
}

impl FileEntry {
    /// Creates an entry capturing an already-parsed prefix.
    pub fn with_session(session: SessionUsage) -> Self {
        Self { offset: 0, complete: false, session }
    }
}

pub fn add_breakdown(target: &TokenBreakdown, delta: &TokenBreakdown) -> TokenBreakdown {
    TokenBreakdown {
        requests: target.requests + delta.requests,
        input_tokens: target.input_tokens + delta.input_tokens,
        cached_input_tokens: target.cached_input_tokens + delta.cached_input_tokens,
        cache_write_tokens: target.cache_write_tokens + delta.cache_write_tokens,
        output_tokens: target.output_tokens + delta.output_tokens,
        reasoning_tokens: target.reasoning_tokens + delta.reasoning_tokens,
        total_tokens: target.total_tokens + delta.total_tokens,
    }
}

pub fn to_breakdown(total: &TotalTokenUsage, requests: u64) -> TokenBreakdown {
    TokenBreakdown {
        requests,
        input_tokens: total.input_tokens,
        cached_input_tokens: total.cached_input_tokens,
        cache_write_tokens: total.cache_write_input_tokens,
        output_tokens: total.output_tokens,
        reasoning_tokens: total.reasoning_output_tokens,
        total_tokens: total.total_tokens,
    }
}

pub fn in_range(ts_ms: i64, key: &str, now_ms: i64) -> bool {
    match key {
        "today" => ts_ms >= crate::zcode::aggregate::local_day_start_ms(now_ms) && ts_ms <= now_ms,
        "60m" => ts_ms >= now_ms.saturating_sub(60 * 60_000) && ts_ms <= now_ms,
        "24h" => ts_ms >= now_ms.saturating_sub(24 * 60 * 60_000) && ts_ms <= now_ms,
        "7d" => ts_ms >= now_ms.saturating_sub(7 * 24 * 60 * 60_000) && ts_ms <= now_ms,
        "30d" => ts_ms >= now_ms.saturating_sub(30 * 24 * 60 * 60_000) && ts_ms <= now_ms,
        "all" => true,
        _ => false,
    }
}

fn model_rows(models: HashMap<String, TokenBreakdown>) -> Vec<ModelUsageRow> {
    let mut rows: Vec<_> = models
        .into_iter()
        .map(|(model, breakdown)| ModelUsageRow { model, breakdown })
        .collect();
    rows.sort_by(|a, b| {
        b.breakdown
            .total_tokens
            .cmp(&a.breakdown.total_tokens)
            .then_with(|| a.model.cmp(&b.model))
    });
    rows
}

/// Derive the six rolling ranges plus legacy top-level fields from per-file
/// session accumulators. `all` sums the exact per-file totals; every other
/// range filters the timestamped records.
pub fn aggregate_local<'a, I>(sessions: I, now_ms: i64) -> LocalUsage
where
    I: Iterator<Item = &'a SessionUsage>,
{
    // Materialize once: every range below re-iterates the same sessions.
    let sessions: Vec<&SessionUsage> = sessions.collect();
    let mut usage = LocalUsage::default();
    for key in RANGE_KEYS {
        let mut breakdown = TokenBreakdown::default();
        let mut models: HashMap<String, TokenBreakdown> = HashMap::new();
        let mut sessions_in_range = 0u64;
        for s in &sessions {
            if s.session_id.is_empty() && s.responses == 0 {
                continue;
            }
            if key == "all" {
                sessions_in_range += 1;
                breakdown.requests += s.responses;
                let all = to_breakdown(&s.all_time, 0);
                breakdown.input_tokens += all.input_tokens;
                breakdown.cached_input_tokens += all.cached_input_tokens;
                breakdown.cache_write_tokens += all.cache_write_tokens;
                breakdown.output_tokens += all.output_tokens;
                breakdown.reasoning_tokens += all.reasoning_tokens;
                breakdown.total_tokens += all.total_tokens;
                for (model, total) in &s.model_totals {
                    let row = models.entry(model.clone()).or_default();
                    *row = add_breakdown(
                        row,
                        &to_breakdown(total, s.model_requests.get(model).copied().unwrap_or(0)),
                    );
                }
                continue;
            }
            let mut in_session = false;
            for r in &s.records {
                if !in_range(r.ts_ms, key, now_ms) {
                    continue;
                }
                in_session = true;
                breakdown.requests += 1;
                let delta = to_breakdown(&record_delta(r), 1);
                breakdown.input_tokens += delta.input_tokens;
                breakdown.cached_input_tokens += delta.cached_input_tokens;
                breakdown.cache_write_tokens += delta.cache_write_tokens;
                breakdown.output_tokens += delta.output_tokens;
                breakdown.reasoning_tokens += delta.reasoning_tokens;
                breakdown.total_tokens += delta.total_tokens;
                let row = models.entry(r.model.clone()).or_default();
                *row = add_breakdown(row, &delta);
            }
            if in_session {
                sessions_in_range += 1;
            }
        }
        usage.ranges.push(LocalUsageRange {
            key: key.into(),
            breakdown,
            sessions: sessions_in_range,
            models: model_rows(models),
        });
    }
    usage.today = usage.ranges[0].breakdown.clone();
    usage.last_7d = usage.ranges[3].breakdown.clone();
    usage.all_time = usage.ranges[5].breakdown.clone();
    usage.sessions = usage.ranges[5].sessions;
    usage.models = usage.ranges[5].models.clone();
    usage
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(ts: i64, model: &str, value: u64) -> UsageRecord {
        // Exclusive schema fixture: input excludes the cache fields.
        UsageRecord {
            ts_ms: ts,
            model: model.into(),
            session_id: Some("s1".into()),
            project: None,
            input_tokens: value,
            output_tokens: value,
            reasoning_tokens: Some(0),
            cache_read_tokens: Some(0),
            cache_write_tokens: Some(0),
            duration_ms: None,
            ttft_ms: None,
            status: None,
            total_override: Some(value * 2),
            reasoning_in_output: false,
            schema_exclusive: Some(false),
            duration_derived: false,
            source_file: "t".into(),
        }
    }

    fn total(value: u64) -> TotalTokenUsage {
        TotalTokenUsage {
            input_tokens: value,
            total_tokens: value,
            ..Default::default()
        }
    }

    #[test]
    fn six_ranges_use_rolling_boundaries_and_keep_all_time_exact() {
        let now = 1_800_000_000_000i64;
        let events = [
            (now - 30 * 60_000, "deepseek-chat", 10),
            (now - 2 * 60 * 60_000, "deepseek-reasoner", 20),
            (now - 2 * 24 * 60 * 60_000, "deepseek-chat", 30),
            (now - 10 * 24 * 60 * 60_000, "deepseek-chat", 40),
            (now - 31 * 24 * 60 * 60_000, "deepseek-chat", 50),
        ];
        // Records carry the display total as override; the "all" branch uses
        // the exact accumulated totals (50 included even though it is older
        // than the 30-day rolling window).
        let session = SessionUsage {
            session_id: "s1".into(),
            responses: 5,
            all_time: total(150),
            model_totals: HashMap::from([
                ("deepseek-chat".into(), total(130)),
                ("deepseek-reasoner".into(), total(20)),
            ]),
            model_requests: HashMap::from([("deepseek-chat".into(), 4), ("deepseek-reasoner".into(), 1)]),
            records: events
                .iter()
                .map(|(ts_ms, model, value)| record(*ts_ms, model, *value))
                .collect(),
            ..Default::default()
        };

        let usage = aggregate_local(std::iter::once(&session), now);
        assert_eq!(
            usage
                .ranges
                .iter()
                .map(|r| r.key.as_str())
                .collect::<Vec<_>>(),
            RANGE_KEYS
        );
        let value = |key: &str| {
            usage
                .ranges
                .iter()
                .find(|range| range.key == key)
                .unwrap()
                .breakdown
                .total_tokens
        };
        assert_eq!(value("60m"), 20);
        assert_eq!(value("24h"), 60);
        assert_eq!(value("7d"), 120);
        assert_eq!(value("30d"), 200);
        assert_eq!(value("all"), 150);
        assert_eq!(usage.all_time.requests, 5);
        assert_eq!(usage.ranges[1].sessions, 1);
        assert_eq!(usage.ranges[1].models[0].model, "deepseek-chat");
        assert_eq!(
            usage.ranges[5]
                .models
                .iter()
                .map(|m| m.breakdown.requests)
                .sum::<u64>(),
            5
        );
    }

    #[test]
    fn delta_record_and_record_delta_roundtrip_calibers() {
        // Codex-style: inclusive input + source total; reasoning separate.
        let delta = TotalTokenUsage {
            input_tokens: 1000,
            cached_input_tokens: 10,
            cache_write_input_tokens: 0,
            output_tokens: 100,
            reasoning_output_tokens: 14,
            total_tokens: 1114,
        };
        let rec = delta_record(&delta, CODEX_DELTA, 5, "gpt-5.6-sol", "s1", None, "f.jsonl", Some(3_600));
        assert_eq!(rec.display_total_tokens(), 1114, "source total wins");
        assert_eq!(rec.cache_read_tokens, Some(10));
        let back = record_delta(&rec);
        assert_eq!(back, delta);

        // DSH-style: exclusive input, reasoning nested in output.
        let dsh = TotalTokenUsage {
            input_tokens: 1000,
            cached_input_tokens: 100,
            cache_write_input_tokens: 40,
            output_tokens: 500,
            reasoning_output_tokens: 200,
            total_tokens: 1640,
        };
        let rec = delta_record(&dsh, DSH_DELTA, 5, "deepseek-chat", "s1", None, "f.jsonl", None);
        assert_eq!(rec.display_total_tokens(), 1640, "documented disjoint sum wins");
        assert_eq!(rec.generated_tokens(), 500, "reasoning nested in output");
        assert_eq!(record_delta(&rec), dsh);
    }

    #[test]
    fn old_cache_snapshots_without_records_still_load() {
        // Pre-`records` persisted entries deserialize with an empty history;
        // rolling ranges degrade to zero while "all" stays exact.
        let old = serde_json::json!({
            "session_id": "s1",
            "model": "m",
            "first_ts_ms": 0,
            "last_ts_ms": 0,
            "responses": 3,
            "all_time": { "input_tokens": 30, "total_tokens": 30 }
        });
        let s: SessionUsage = serde_json::from_value(old).unwrap();
        assert!(s.records.is_empty());
        assert_eq!(s.title, None);
        assert_eq!(s.project_path, None);
        let usage = aggregate_local(std::iter::once(&s), 1_800_000_000_000);
        assert_eq!(usage.all_time.total_tokens, 30);
        assert_eq!(usage.today.total_tokens, 0);
    }
}
