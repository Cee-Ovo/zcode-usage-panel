//! Local anomaly detection.
//!
//! Five rules (each individually configurable, all optional):
//! 1. **spike**   — 10-minute token total ≥ multiplier × the trailing hour's
//!                  10-minute average (and ≥ `spike_min_tokens`).
//! 2. **session** — a single session's cumulative total crosses a threshold.
//! 3. **cache**   — recent (30 min) cache hit rate drops `cache_hit_drop`
//!                  below the trailing 24 h baseline (min. requests apply).
//! 4. **burst**   — one model issues ≥ N requests within 5 minutes.
//! 5. **stale**   — no new usage records for `staleness_minutes` while the
//!                  store previously saw activity within the last 24 h.
//!
//! Each rule has a 15-minute cooldown so one incident fires one toast.
//! Everything is local: Windows notifications only, nothing leaves the box.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::settings::AlertRuleState;
use crate::zcode::aggregate::group_by_model;
use crate::zcode::store::UsageStore;

const COOLDOWN_MS: i64 = 15 * 60_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertEvent {
    pub rule: String,
    pub severity: u8, // 1 = info, 2 = warning, 3 = critical
    pub title: String,
    pub body: String,
    pub ts_ms: i64,
}

pub struct AlertEngine {
    fired_at: HashMap<String, i64>,
}

impl AlertEngine {
    pub fn new() -> Self {
        Self {
            fired_at: HashMap::new(),
        }
    }

    fn ready(&self, key: &str, now_ms: i64) -> bool {
        self.fired_at.get(key).map(|t| now_ms - *t >= COOLDOWN_MS).unwrap_or(true)
    }

    fn fire(&mut self, key: &str, now_ms: i64) {
        self.fired_at.insert(key.to_string(), now_ms);
    }

    pub fn evaluate(
        &mut self,
        store: &mut UsageStore,
        rules: &AlertRuleState,
        now_ms: i64,
    ) -> Vec<AlertEvent> {
        if !rules.enabled || store.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let minute = 60_000i64;

        // ---- 1. 10-minute spike vs trailing hour average --------------------
        let last_10m = store.range(now_ms - 10 * minute, now_ms);
        // Trailing-hour baseline EXCLUDES the current 10-minute window, so a
        // surge is compared against a normal baseline rather than diluting it.
        let last_hour = store.range(now_ms - 60 * minute, now_ms - 10 * minute);
        let spike_now: u64 = last_10m
            .iter()
            .map(|r| {
                r.input_tokens + r.output_tokens
                    + r.reasoning_tokens.unwrap_or(0)
                    + r.cache_read_tokens.unwrap_or(0)
                    + r.cache_write_tokens.unwrap_or(0)
            })
            .sum();
        let hour_total: u64 = last_hour
            .iter()
            .map(|r| {
                r.input_tokens + r.output_tokens
                    + r.reasoning_tokens.unwrap_or(0)
                    + r.cache_read_tokens.unwrap_or(0)
                    + r.cache_write_tokens.unwrap_or(0)
            })
            .sum();
        let avg_10m = hour_total / 6;
        if spike_now >= rules.spike_min_tokens
            && avg_10m > 0
            && spike_now as f64 >= rules.spike_multiplier * avg_10m as f64
            && self.ready("spike", now_ms)
        {
            self.fire("spike", now_ms);
            out.push(AlertEvent {
                rule: "spike".into(),
                severity: 2,
                title: "Token 激增".into(),
                body: format!(
                    "最近 10 分钟消耗 {spike_now} tokens(过去一小时均值为每 10 分钟 {avg_10m})"
                ),
                ts_ms: now_ms,
            });
        }

        // ---- 2. Session total threshold --------------------------------------
        if rules.session_total_tokens > 0 {
            let offenders: Vec<(String, u64)> = store
                .session_summaries()
                .iter()
                .filter(|s| s.agg.total_tokens() >= rules.session_total_tokens)
                .map(|s| (s.id.clone(), s.agg.total_tokens()))
                .collect();
            if let Some((id, total)) = offenders.first() {
                let key = format!("session:{id}");
                if self.ready(&key, now_ms) {
                    self.fire(&key, now_ms);
                    out.push(AlertEvent {
                        rule: "session".into(),
                        severity: 2,
                        title: "Session 用量异常".into(),
                        body: format!(
                            "Session {} 累计 {total} tokens,超过阈值 {}",
                            short_id(id),
                            rules.session_total_tokens
                        ),
                        ts_ms: now_ms,
                    });
                }
            }
        }

        // ---- 3. Cache hit-rate drop -------------------------------------------
        let recent = store.range(now_ms - 30 * minute, now_ms);
        let baseline = store.range(now_ms - 24 * 60 * minute, now_ms - 30 * minute);
        let mut recent_agg = crate::zcode::aggregate::Agg::default();
        for r in recent {
            recent_agg.add(r);
        }
        let mut baseline_agg = crate::zcode::aggregate::Agg::default();
        for r in baseline {
            baseline_agg.add(r);
        }
        if recent_agg.requests >= rules.cache_min_requests {
            if let (Some(recent_rate), Some(base_rate)) =
                (recent_agg.cache_hit_rate(), baseline_agg.cache_hit_rate())
            {
                if base_rate - recent_rate >= rules.cache_hit_drop
                    && self.ready("cache", now_ms)
                {
                    self.fire("cache", now_ms);
                    out.push(AlertEvent {
                        rule: "cache".into(),
                        severity: 1,
                        title: "Cache Hit Rate 下降".into(),
                        body: format!(
                            "最近 30 分钟命中率 {:.0}%(基线 {:.0}%)",
                            recent_rate * 100.0,
                            base_rate * 100.0
                        ),
                        ts_ms: now_ms,
                    });
                }
            }
        }

        // ---- 4. Model burst -----------------------------------------------------
        let burst = store.range(now_ms - 5 * minute, now_ms);
        for m in group_by_model(burst) {
            if m.agg.requests >= rules.model_burst_per_5m
                && self.ready("burst", now_ms)
                && rules.model_burst_per_5m > 0
            {
                self.fire("burst", now_ms);
                out.push(AlertEvent {
                    rule: "burst".into(),
                    severity: 2,
                    title: "模型调用激增".into(),
                    body: format!(
                        "{} 最近 5 分钟 {} 次请求",
                        m.name, m.agg.requests
                    ),
                    ts_ms: now_ms,
                });
                break;
            }
        }

        // ---- 5. Data staleness ----------------------------------------------------
        if let Some(last) = store.last_record_ms {
            let stale_for = now_ms - last;
            let had_recent_activity = now_ms - last <= 24 * 60 * minute;
            if stale_for >= rules.staleness_minutes as i64 * minute
                && had_recent_activity
                && self.ready("stale", now_ms)
            {
                self.fire("stale", now_ms);
                out.push(AlertEvent {
                    rule: "stale".into(),
                    severity: 1,
                    title: "ZCode 数据停止更新".into(),
                    body: format!("已 {} 分钟没有新的 usage 记录", stale_for / minute),
                    ts_ms: now_ms,
                });
            }
        }

        out
    }
}

fn short_id(id: &str) -> String {
    if id.len() > 12 {
        format!("{}…", &id[..8])
    } else {
        id.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zcode::usage::UsageRecord;

    fn rec(ts: i64, model: &str, input: u64) -> UsageRecord {
        UsageRecord {
            ts_ms: ts,
            model: model.into(),
            session_id: Some("s1".into()),
            project: Some("p".into()),
            input_tokens: input,
            output_tokens: 0,
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            source_file: "t".into(),
        }
    }

    #[test]
    fn spike_rule_fires_with_cooldown() {
        let now = 1_756_300_000_000i64;
        let mut store = UsageStore::new();
        // steady baseline over the past hour, then a huge last-10-min burst
        let mut batch = Vec::new();
        for i in 0..60 {
            batch.push(rec(now - 60 * 60_000 + i * 60_000, "m", 10_000));
        }
        for i in 0..10 {
            batch.push(rec(now - 10 * 60_000 + i * 60_000, "m", 500_000));
        }
        store.ingest(batch);

        let mut engine = AlertEngine::new();
        let rules = AlertRuleState::default();
        let events = engine.evaluate(&mut store, &rules, now);
        assert!(events.iter().any(|e| e.rule == "spike"));

        // Cooldown: immediate re-evaluation must not re-fire.
        let again = engine.evaluate(&mut store, &rules, now + 1000);
        assert!(!again.iter().any(|e| e.rule == "spike"));
    }

    #[test]
    fn disabled_rules_fire_nothing() {
        let mut store = UsageStore::new();
        store.ingest(vec![rec(1, "m", 1_000_000_000)]);
        let mut engine = AlertEngine::new();
        let mut rules = AlertRuleState::default();
        rules.enabled = false;
        assert!(engine.evaluate(&mut store, &rules, 2).is_empty());
    }
}
