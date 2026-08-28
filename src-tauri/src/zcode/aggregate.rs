//! Aggregation: totals, per-model stats, time buckets, session summaries.
//!
//! Statistical conventions (also shown as UI tooltips):
//! - **Total tokens** = input + output + reasoning + cache_read + cache_write,
//!   summing only the fields the source actually provides.
//! - **Cache Hit Rate** = cached input / total input, where a record's total
//!   input is auto-classified per source schema:
//!     * inclusive schemas (input_tokens already contains cached tokens,
//!       e.g. OpenAI-style `prompt_tokens`): total = input_tokens,
//!       hit = cached / input.
//!     * exclusive schemas (input_tokens excludes cache, e.g. Claude-style
//!       `input_tokens` + separate `cache_read_input_tokens`):
//!       total = input + cache_read + cache_write, hit = cache_read / total.
//!   Records without cache fields contribute to neither numerator nor
//!   denominator. If no record in a group reports cache fields, the hit rate
//!   is `None` ⇒ displayed as "unavailable".
//! - Optional fields (reasoning / cache) carry a **coverage** ratio
//!   (`present / requests`); below 100 % the UI annotates the value with the
//!   number of contributing records so nothing is silently extrapolated.

use std::collections::HashMap;

use chrono::TimeZone;
use serde::{Deserialize, Serialize};

use super::usage::UsageRecord;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FieldStat {
    pub sum: u64,
    /// How many records actually provided this field.
    pub present: u64,
}

impl FieldStat {
    fn add(&mut self, v: Option<u64>) {
        if let Some(x) = v {
            self.sum += x;
            self.present += 1;
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Agg {
    pub requests: u64,
    pub input: u64,
    pub output: u64,
    pub reasoning: FieldStat,
    pub cache_read: FieldStat,
    pub cache_write: FieldStat,
    /// Σ cached input tokens (numerator of the hit rate).
    pub hit_cached: u64,
    /// Σ total input tokens under the auto-classified schema (denominator).
    pub hit_input_total: u64,
    pub first_ts_ms: Option<i64>,
    pub last_ts_ms: Option<i64>,
}

impl Agg {
    pub fn add(&mut self, r: &UsageRecord) {
        self.requests += 1;
        self.input += r.input_tokens;
        self.output += r.output_tokens;
        self.reasoning.add(r.reasoning_tokens);
        self.cache_read.add(r.cache_read_tokens);
        self.cache_write.add(r.cache_write_tokens);

        if let Some(cr) = r.cache_read_tokens {
            let cw = r.cache_write_tokens.unwrap_or(0);
            let inclusive = r.input_tokens >= cr + cw && r.input_tokens > 0;
            let total = if inclusive {
                r.input_tokens.max(cr)
            } else {
                r.input_tokens + cr + cw
            };
            self.hit_cached += cr;
            self.hit_input_total += total;
        }

        self.first_ts_ms = Some(self.first_ts_ms.map_or(r.ts_ms, |t| t.min(r.ts_ms)));
        self.last_ts_ms = Some(self.last_ts_ms.map_or(r.ts_ms, |t| t.max(r.ts_ms)));
    }

    /// Total tokens as displayed. Only counts fields the source provides.
    pub fn total_tokens(&self) -> u64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.reasoning.sum)
            .saturating_add(self.cache_read.sum)
            .saturating_add(self.cache_write.sum)
    }

    pub fn cache_hit_rate(&self) -> Option<f64> {
        if self.hit_input_total > 0 {
            Some(self.hit_cached as f64 / self.hit_input_total as f64)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStat {
    pub name: String,
    pub agg: Agg,
}

pub fn group_by_model(records: &[UsageRecord]) -> Vec<ModelStat> {
    let mut map: HashMap<String, Agg> = HashMap::new();
    for r in records {
        map.entry(r.model.clone()).or_default().add(r);
    }
    let mut out: Vec<ModelStat> = map
        .into_iter()
        .map(|(name, agg)| ModelStat { name, agg })
        .collect();
    out.sort_by(|a, b| b.agg.total_tokens().cmp(&a.agg.total_tokens()));
    out
}

// ---------------------------------------------------------------------------
// Time ranges and buckets
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrendRange {
    Last60Min,
    TodayHourly,
    Last24h,
    Last7d,
    Last30d,
    All,
}

impl TrendRange {
    pub fn key(&self) -> &'static str {
        match self {
            TrendRange::Last60Min => "60m",
            TrendRange::TodayHourly => "today",
            TrendRange::Last24h => "24h",
            TrendRange::Last7d => "7d",
            TrendRange::Last30d => "30d",
            TrendRange::All => "all",
        }
    }
    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "60m" => TrendRange::Last60Min,
            "today" => TrendRange::TodayHourly,
            "24h" => TrendRange::Last24h,
            "7d" => TrendRange::Last7d,
            "30d" => TrendRange::Last30d,
            "all" => TrendRange::All,
            _ => return None,
        })
    }
}

pub fn local_day_start_ms(ms: i64) -> i64 {
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .and_then(|dt| dt.date_naive().and_hms_opt(0, 0, 0))
        .and_then(|naive| chrono::Local.from_local_datetime(&naive).single())
        .map(|d| d.timestamp_millis())
        .unwrap_or(ms)
}

/// Resolve a range to `(from_ms, to_ms, bucket_count)` against "now".
/// `history_start_ms` is the oldest record timestamp (used by `All`).
pub fn resolve_span(range: TrendRange, now_ms: i64, history_start_ms: Option<i64>) -> (i64, i64, usize) {
    match range {
        TrendRange::Last60Min => (now_ms - 60 * 60_000, now_ms, 60),
        TrendRange::TodayHourly => {
            let from = local_day_start_ms(now_ms);
            (from, now_ms, 24)
        }
        TrendRange::Last24h => (now_ms - 24 * 3600_000, now_ms, 24),
        TrendRange::Last7d => (now_ms - 7 * 24 * 3600_000, now_ms, 28),
        TrendRange::Last30d => (now_ms - 30 * 24 * 3600_000, now_ms, 30),
        TrendRange::All => {
            let from = history_start_ms.unwrap_or(now_ms - 30 * 24 * 3600_000);
            (from, now_ms, 90)
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    pub start_ms: i64,
    pub end_ms: i64,
    pub agg: Agg,
    pub by_model: HashMap<String, Agg>,
}

pub fn bucketize(records: &[UsageRecord], from_ms: i64, to_ms: i64, buckets: usize) -> Vec<Bucket> {
    let buckets = buckets.max(1);
    let span = (to_ms - from_ms).max(1) as u128;
    let mut out: Vec<Bucket> = (0..buckets)
        .map(|i| {
            let start = from_ms + (span * i as u128 / buckets as u128) as i64;
            let end = from_ms + (span * (i + 1) as u128 / buckets as u128) as i64;
            Bucket {
                start_ms: start,
                end_ms: end,
                agg: Agg::default(),
                by_model: HashMap::new(),
            }
        })
        .collect();
    for r in records {
        if r.ts_ms < from_ms || r.ts_ms > to_ms {
            continue;
        }
        let idx = (((r.ts_ms - from_ms) as u128 * buckets as u128) / span) as usize;
        let idx = idx.min(buckets - 1);
        let b = &mut out[idx];
        b.agg.add(r);
        b.by_model.entry(r.model.clone()).or_default().add(r);
    }
    out
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub project: Option<String>,
    pub models: Vec<String>,
    pub agg: Agg,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(ts: i64, model: &str, input: u64, output: u64, cr: Option<u64>, cw: Option<u64>) -> UsageRecord {
        UsageRecord {
            ts_ms: ts,
            model: model.into(),
            session_id: Some("s".into()),
            project: Some("p".into()),
            input_tokens: input,
            output_tokens: output,
            reasoning_tokens: None,
            cache_read_tokens: cr,
            cache_write_tokens: cw,
            source_file: "t".into(),
        }
    }

    #[test]
    fn hit_rate_exclusive_schema() {
        // Claude-style: input excludes cache.
        let mut agg = Agg::default();
        agg.add(&rec(1, "m", 1000, 500, Some(39_000), Some(5_000)));
        let rate = agg.cache_hit_rate().unwrap();
        // total input = 1000 + 39000 + 5000 = 45000; cached = 39000
        assert!((rate - 39_000.0 / 45_000.0).abs() < 1e-9);
    }

    #[test]
    fn hit_rate_inclusive_schema() {
        let mut agg = Agg::default();
        // OpenAI-style: prompt_tokens includes cached_tokens.
        agg.add(&rec(1, "m", 900, 100, Some(800), None));
        let rate = agg.cache_hit_rate().unwrap();
        assert!((rate - 800.0 / 900.0).abs() < 1e-9);
    }

    #[test]
    fn hit_rate_unavailable_without_cache_fields() {
        let mut agg = Agg::default();
        agg.add(&rec(1, "m", 10, 10, None, None));
        assert!(agg.cache_hit_rate().is_none());
    }

    #[test]
    fn total_tokens_and_coverage() {
        let mut agg = Agg::default();
        agg.add(&rec(1, "m", 10, 20, Some(100), None));
        agg.add(&rec(2, "m", 1, 2, None, None));
        assert_eq!(agg.requests, 2);
        assert_eq!(agg.cache_read.sum, 100);
        assert_eq!(agg.cache_read.present, 1); // 1 of 2 records
        assert_eq!(agg.total_tokens(), 10 + 20 + 100 + 1 + 2);
    }

    #[test]
    fn bucketize_assigns_correctly() {
        let recs = vec![
            rec(0, "a", 1, 0, None, None),
            rec(50_000, "a", 1, 0, None, None),
            rec(150_000, "b", 1, 0, None, None),
        ];
        let buckets = bucketize(&recs, 0, 200_000, 2);
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].agg.requests, 2);
        assert_eq!(buckets[1].agg.requests, 1);
        assert_eq!(buckets[1].by_model["b"].requests, 1);
    }
}
