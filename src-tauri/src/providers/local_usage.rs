//! Shared "local harness token usage" machinery for offline session-log
//! providers (Codex rollouts today, DeepSeek Harness sessions next).
//!
//! Every provider keeps per-file parse state (watermarks + a `SessionUsage`
//! accumulator) and derives the same six rolling ranges from timestamped
//! usage events, so the UI can render every local-usage section with one
//! component. Anything provider-specific (wire formats, cumulative-counter
//! quirks, account claims) stays in the provider module.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::{LocalUsage, LocalUsageRange, ModelUsageRow, TokenBreakdown};

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

/// One timestamped usage delta retained for rolling-range aggregation.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UsageEvent {
    pub ts_ms: i64,
    pub model: String,
    pub delta: TotalTokenUsage,
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
    /// Timestamped deltas for rolling ranges; entries older than 30 days are
    /// pruned after each poll.
    #[serde(default)]
    pub recent: Vec<UsageEvent>,
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

/// Prune per-file rolling events past 30 days; reports whether anything went.
pub fn prune_recent<'a, I>(sessions: I, now_ms: i64) -> bool
where
    I: Iterator<Item = &'a mut SessionUsage>,
{
    let cutoff = now_ms.saturating_sub(30 * 24 * 60 * 60_000);
    let mut changed = false;
    for session in sessions {
        let before = session.recent.len();
        session.recent.retain(|event| event.ts_ms >= cutoff);
        changed |= before != session.recent.len();
    }
    changed
}

/// Derive the six rolling ranges plus legacy top-level fields from per-file
/// session accumulators. `all` sums the exact per-file totals; every other
/// range filters the timestamped events.
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
            for event in &s.recent {
                if !in_range(event.ts_ms, key, now_ms) {
                    continue;
                }
                in_session = true;
                breakdown.requests += 1;
                let delta = to_breakdown(&event.delta, 1);
                breakdown.input_tokens += delta.input_tokens;
                breakdown.cached_input_tokens += delta.cached_input_tokens;
                breakdown.cache_write_tokens += delta.cache_write_tokens;
                breakdown.output_tokens += delta.output_tokens;
                breakdown.reasoning_tokens += delta.reasoning_tokens;
                breakdown.total_tokens += delta.total_tokens;
                let row = models.entry(event.model.clone()).or_default();
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
        let recent = events
            .iter()
            .map(|(ts_ms, model, value)| UsageEvent {
                ts_ms: *ts_ms,
                model: (*model).into(),
                delta: total(*value),
            })
            .collect();
        let session = SessionUsage {
            session_id: "s1".into(),
            responses: 5,
            all_time: total(150),
            model_totals: HashMap::from([
                ("deepseek-chat".into(), total(130)),
                ("deepseek-reasoner".into(), total(20)),
            ]),
            model_requests: HashMap::from([("deepseek-chat".into(), 4), ("deepseek-reasoner".into(), 1)]),
            recent,
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
        assert_eq!(value("60m"), 10);
        assert_eq!(value("24h"), 30);
        assert_eq!(value("7d"), 60);
        assert_eq!(value("30d"), 100);
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
    fn prune_keeps_all_time_and_drops_old_events() {
        let now = 1_800_000_000_000i64;
        let mut session = SessionUsage {
            all_time: total(60),
            recent: vec![
                UsageEvent { ts_ms: now - 31 * 24 * 60 * 60_000, model: "m".into(), delta: total(20) },
                UsageEvent { ts_ms: now - 2 * 24 * 60 * 60_000, model: "m".into(), delta: total(40) },
            ],
            ..Default::default()
        };
        assert!(prune_recent(std::iter::once(&mut session), now));
        assert_eq!(session.recent.len(), 1);
        assert_eq!(session.all_time.total_tokens, 60);
    }
}
