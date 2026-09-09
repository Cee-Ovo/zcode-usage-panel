//! Aggregation: totals, per-model stats, time buckets, session summaries.
//!
//! Statistical conventions (also shown as UI tooltips):
//! - **Total tokens** = per-record display totals
//!   (`UsageRecord::display_total_tokens`): a source-provided total wins;
//!   otherwise input + output + reasoning (when the schema reports it
//!   outside output) + cache_read + cache_write (when the schema's input
//!   excludes cache). Inclusive schemas (input already contains the cache
//!   tokens, e.g. ZCode `model_usage`, OpenAI-style `prompt_tokens`) must
//!   not add the cache again — otherwise heavily cached traffic doubles.
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
    /// Σ per-record display totals (`UsageRecord::display_total_tokens`).
    /// What `total_tokens()` reports — never a blind recombination of the
    /// field sums, which double counts on inclusive schemas.
    #[serde(default)]
    pub total_sum: u64,
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
        self.total_sum = self.total_sum.saturating_add(r.display_total_tokens());
    }

    /// Total tokens as displayed. Sum of per-record display totals so each
    /// record is counted under its own source schema.
    pub fn total_tokens(&self) -> u64 {
        self.total_sum
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
// Response-speed statistics (TTFT / tokens-per-second)
// ---------------------------------------------------------------------------

/// Speed-class metrics over a set of usage records. Honest-caliber rules:
/// - Only requests the source marks completed (or that carry no status at
///   all) contribute; `error` / `cancelled` / `running` rows are excluded.
/// - TTFT values are source-provided originals, never derived here.
/// - tok/s counts generated tokens (`output + reasoning`, unless the schema
///   already nests reasoning inside output) over the *generation* window
///   (duration − TTFT), so requests without a usable TTFT are excluded from
///   the speed aggregate instead of being averaged under a different
///   convention.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SpeedStats {
    pub ttft_avg_ms: Option<f64>,
    pub ttft_p50_ms: Option<f64>,
    pub ttft_p95_ms: Option<f64>,
    /// Records with a source-provided TTFT.
    pub ttft_samples: u64,
    /// Σ generated tokens ÷ Σ generation seconds (weighted aggregate).
    pub speed_tps: Option<f64>,
    /// Median per-request speed (tok/s), for context next to the weighted
    /// aggregate.
    pub speed_p50_tps: Option<f64>,
    /// Records with a usable TTFT + duration + generated tokens.
    pub speed_samples: u64,
    /// Completed (or status-unknown) requests in range — the denominator for
    /// coverage notes.
    pub completed_requests: u64,
    /// Σ generated tokens (output + reasoning unless nested in output).
    pub generated_tokens: u64,
    /// Σ (duration − TTFT) milliseconds behind `speed_tps`.
    pub generation_ms: u64,
}

fn status_is_completed(status: &Option<String>) -> bool {
    match status.as_deref() {
        None | Some("completed") => true,
        Some(_) => false,
    }
}

/// Nearest-rank percentile of a non-empty sorted ascending slice.
fn percentile(sorted: &[u64], pct: f64) -> u64 {
    debug_assert!(!sorted.is_empty());
    let rank = ((sorted.len() as f64) * pct).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// Compute TTFT / tok-s statistics over `records` (usually one range slice).
/// Generic over the iterator so per-model groups (`&[&UsageRecord]`) reuse the
/// exact same caliber as the whole-range dashboard card.
pub fn compute_speed_stats<'a, I>(records: I) -> SpeedStats
where
    I: IntoIterator<Item = &'a UsageRecord>,
{
    let mut stats = SpeedStats::default();
    let mut ttfts: Vec<u64> = Vec::new();
    let mut per_request_tps: Vec<f64> = Vec::new();

    for r in records {
        if !status_is_completed(&r.status) {
            continue;
        }
        stats.completed_requests += 1;

        if let Some(ttft) = r.ttft_ms.filter(|t| r.duration_ms.map_or(true, |d| d >= *t)) {
            ttfts.push(ttft);
        }

        let generated = r.generated_tokens();
        if let (Some(ttft), Some(duration)) = (r.ttft_ms, r.duration_ms) {
            if generated > 0 && duration > ttft {
                let gen_ms = duration - ttft;
                stats.generated_tokens += generated;
                stats.generation_ms += gen_ms;
                per_request_tps.push(generated as f64 * 1000.0 / gen_ms as f64);
            }
        }
    }

    if !ttfts.is_empty() {
        let sum: u128 = ttfts.iter().map(|t| *t as u128).sum();
        stats.ttft_avg_ms = Some(sum as f64 / ttfts.len() as f64);
        ttfts.sort_unstable();
        stats.ttft_p50_ms = Some(percentile(&ttfts, 0.50) as f64);
        stats.ttft_p95_ms = Some(percentile(&ttfts, 0.95) as f64);
        stats.ttft_samples = ttfts.len() as u64;
    }
    if !per_request_tps.is_empty() {
        stats.speed_tps = Some(stats.generated_tokens as f64 * 1000.0 / stats.generation_ms as f64);
        per_request_tps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = per_request_tps.len() / 2;
        stats.speed_p50_tps = Some(if per_request_tps.len() % 2 == 0 {
            (per_request_tps[mid - 1] + per_request_tps[mid]) / 2.0
        } else {
            per_request_tps[mid]
        });
        stats.speed_samples = per_request_tps.len() as u64;
    }
    stats
}

/// Per-model speed stats over the same record set. Delegates to
/// `compute_speed_stats` per group so the caliber (completed-only, TTFT
/// validity, weighted TPS) matches the whole-range dashboard card exactly.
pub fn speed_by_model(records: &[UsageRecord]) -> HashMap<String, SpeedStats> {
    let mut groups: HashMap<String, Vec<&UsageRecord>> = HashMap::new();
    for r in records {
        groups.entry(r.model.clone()).or_default().push(r);
    }
    groups
        .into_iter()
        .map(|(name, group)| (name, compute_speed_stats(group)))
        .collect()
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
            duration_ms: None,
            ttft_ms: None,
            status: None,
            total_override: None,
            reasoning_in_output: false,
            source_file: "t".into(),
        }
    }

    fn speed_rec(ts: i64, output: u64, reasoning: Option<u64>, ttft: Option<u64>, duration: Option<u64>, status: Option<&str>) -> UsageRecord {
        UsageRecord {
            ts_ms: ts,
            model: "m".into(),
            session_id: None,
            project: None,
            input_tokens: 1000,
            output_tokens: output,
            reasoning_tokens: reasoning,
            cache_read_tokens: None,
            cache_write_tokens: None,
            duration_ms: duration,
            ttft_ms: ttft,
            status: status.map(str::to_string),
            total_override: None,
            reasoning_in_output: false,
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
    fn total_tokens_inclusive_schema_does_not_add_cache_twice() {
        // ZCode / OpenAI style: input_tokens already contains the cache.
        let mut agg = Agg::default();
        agg.add(&rec(1, "m", 54518, 286, Some(54016), None));
        assert_eq!(agg.total_tokens(), 54518 + 286);

        // Inclusive with cache_write too: input = cache_read + cache_write + rest.
        let mut agg = Agg::default();
        agg.add(&rec(1, "m", 19975, 18, Some(0), Some(19973)));
        assert_eq!(agg.total_tokens(), 19975 + 18);
    }

    #[test]
    fn total_tokens_source_override_wins() {
        // ZCode computed_total_tokens: input+output, reasoning nested in output.
        let mut r = rec(1, "m", 54518, 286, Some(54016), None);
        r.total_override = Some(54804);
        r.reasoning_in_output = true;
        let mut agg = Agg::default();
        agg.add(&r);
        assert_eq!(agg.total_tokens(), 54804);
    }

    #[test]
    fn speed_generated_tokens_exclude_nested_reasoning() {
        // reasoning_tokens ⊆ output_tokens on the ZCode SQLite schema:
        // adding it again inflated tok/s (deepseek rows: 132 real → 213 shown).
        let mut r = speed_rec(1, 73, Some(36), Some(1_000), Some(2_000), None);
        r.reasoning_in_output = true;
        let s = compute_speed_stats(&[r]);
        assert_eq!(s.generated_tokens, 73);
        assert!((s.speed_tps.unwrap() - 73.0).abs() < 1e-9);

        // Same record under a Claude-style schema (reasoning separate) keeps
        // the old caliber.
        let r = speed_rec(1, 73, Some(36), Some(1_000), Some(2_000), None);
        let s = compute_speed_stats(&[r]);
        assert_eq!(s.generated_tokens, 109);
        assert!((s.speed_tps.unwrap() - 109.0).abs() < 1e-9);
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

    #[test]
    fn speed_stats_weighted_aggregate_and_percentiles() {
        // Three usable samples with hand-computable speeds:
        //   100 tok over 1s → 100 tps; 300 tok over 3s → 100 tps;
        //   200 tok over 1s → 200 tps.
        let recs = vec![
            speed_rec(1, 100, None, Some(1_000), Some(2_000), None),
            speed_rec(2, 300, None, Some(1_000), Some(4_000), Some("completed")),
            speed_rec(3, 200, None, Some(500), Some(1_500), Some("completed")),
        ];
        let s = compute_speed_stats(&recs);
        assert_eq!(s.completed_requests, 3);
        assert_eq!(s.ttft_samples, 3);
        // avg ttft = (1000+1000+500)/3; p50 = 1000 (sorted [500,1000,1000]); p95 = 1000
        assert!((s.ttft_avg_ms.unwrap() - 833.333).abs() < 0.1);
        assert_eq!(s.ttft_p50_ms, Some(1000.0));
        assert_eq!(s.ttft_p95_ms, Some(1000.0));
        // weighted: 600 tokens / 5s = 120 tps (not the mean of 100/100/200 ≈ 133)
        assert!((s.speed_tps.unwrap() - 120.0).abs() < 1e-9);
        // median per-request speed = 100
        assert!((s.speed_p50_tps.unwrap() - 100.0).abs() < 1e-9);
        assert_eq!(s.speed_samples, 3);
        assert_eq!(s.generated_tokens, 600);
        assert_eq!(s.generation_ms, 5_000);
    }

    #[test]
    fn speed_by_model_matches_whole_range_caliber() {
        let mut a1 = speed_rec(1, 100, None, Some(1_000), Some(2_000), None);
        a1.model = "alpha".into();
        let mut a2 = speed_rec(2, 300, None, Some(3_000), Some(4_000), Some("completed"));
        a2.model = "alpha".into();
        let mut b1 = speed_rec(3, 500, None, Some(500), Some(1_500), Some("completed"));
        b1.model = "beta".into();
        // error rows never reach any model's stats
        let mut err = speed_rec(4, 999, None, Some(100), Some(9_000), Some("error"));
        err.model = "beta".into();

        let by_model = speed_by_model(&[a1, a2, b1, err]);
        assert_eq!(by_model.len(), 2);

        let alpha = &by_model["alpha"];
        assert_eq!(alpha.completed_requests, 2);
        assert!((alpha.ttft_avg_ms.unwrap() - 2_000.0).abs() < 1e-9);
        // weighted: 400 tokens over (1s + 1s) generation = 200 tps
        assert!((alpha.speed_tps.unwrap() - 200.0).abs() < 1e-9);

        let beta = &by_model["beta"];
        assert_eq!(beta.completed_requests, 1);
        assert_eq!(beta.ttft_samples, 1);
        // 500 tokens over (1500-500)ms = 500 tps
        assert!((beta.speed_tps.unwrap() - 500.0).abs() < 1e-9);
    }

    #[test]
    fn speed_stats_exclusions() {
        let recs = vec![
            // error + cancelled rows: never counted anywhere
            speed_rec(1, 500, None, Some(100), Some(2_000), Some("error")),
            speed_rec(2, 500, None, Some(100), Some(2_000), Some("cancelled")),
            // running: excluded
            speed_rec(3, 500, None, Some(100), Some(2_000), Some("running")),
            // no generated tokens: not a speed sample (but completed + ttft)
            speed_rec(4, 0, None, Some(700), Some(2_000), None),
            // duration == ttft (nothing streamed): ttft kept, no speed sample
            speed_rec(5, 50, None, Some(2_000), Some(2_000), None),
            // duration < ttft (bad row): both excluded
            speed_rec(6, 50, None, Some(3_000), Some(2_000), None),
            // usable: reasoning counts as generated
            speed_rec(7, 80, Some(20), Some(1_000), Some(3_000), None),
        ];
        let s = compute_speed_stats(&recs);
        assert_eq!(s.completed_requests, 4); // records 4,5,6,7
        assert_eq!(s.ttft_samples, 3); // 700, 2000, 1000 (3000 excluded)
        assert_eq!(s.speed_samples, 1);
        assert_eq!(s.generated_tokens, 100); // 80 + 20 reasoning
        assert_eq!(s.generation_ms, 2_000);
        assert!((s.speed_tps.unwrap() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn speed_stats_empty_when_no_timing_data() {
        let recs = vec![rec(1, "m", 10, 10, None, None)];
        let s = compute_speed_stats(&recs);
        assert_eq!(s.completed_requests, 1);
        assert_eq!(s.ttft_samples, 0);
        assert_eq!(s.speed_samples, 0);
        assert!(s.ttft_avg_ms.is_none());
        assert!(s.speed_tps.is_none());
    }

    #[test]
    fn percentile_nearest_rank() {
        // 1..=100: p95 = 95, p50 = 50 under nearest-rank.
        let sorted: Vec<u64> = (1..=100).collect();
        assert_eq!(percentile(&sorted, 0.95), 95);
        assert_eq!(percentile(&sorted, 0.50), 50);
    }
}
