//! Quota alert rules with per-event cooldowns (no notification storms).
//!
//! Rules (all configurable):
//! - `quota_threshold` — usage crosses 50 / 20 / 10 % remaining thresholds.
//!   Fires once per (provider, window, level) while the level holds;
//!   re-arms only after usage drops back below it (e.g. after a reset).
//! - `package_expiry` — an effective token package expires within N days.
//!   One notification per package per day.
//! - `provider_stale` — a provider's data is older than 3× its cadence.
//!   Cooldown 12 h.
//! - `cost_threshold` — daily ZCode API-equivalent cost crosses a CNY value
//!   (evaluated by the hub, one notification per day).
//!
//! State (last-fired timestamps + threshold level memory) persists to the
//! history DB so a restart never re-spams.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::history::QuotaHistory;
use super::ProviderSnapshot;

pub const ALERT_COOLDOWN_STALE_MS: i64 = 12 * 3600_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AlertEvent {
    pub rule: String,
    pub severity: u8, // 1 info, 2 warning, 3 critical
    pub title: String,
    pub body: String,
    pub ts_ms: i64,
}

/// Persisted notification bookkeeping: `key → (last_fired_ms, memory)`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AlertMemory {
    pub fired: HashMap<String, i64>,
    /// quota level memory: key "provider:window" → last level notified.
    pub quota_level: HashMap<String, u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct QuotaAlertRules {
    pub enabled: bool,
    /// Remaining-% thresholds that trigger (defaults 50/20/10).
    pub thresholds: Vec<f64>,
    /// Days before package expiry to warn.
    pub package_expiry_days: u32,
    /// Warn when API-equivalent daily cost crosses this (CNY). 0 = off.
    pub daily_cost_cny: f64,
}

impl Default for QuotaAlertRules {
    fn default() -> Self {
        Self {
            enabled: true,
            thresholds: vec![50.0, 20.0, 10.0],
            package_expiry_days: 7,
            daily_cost_cny: 0.0,
        }
    }
}

pub struct QuotaAlertEngine;

fn level_for(remaining_pct: f64, thresholds: &[f64]) -> Option<u8> {
    // Highest severity level whose threshold the remaining share is below.
    if remaining_pct < 10.0 && thresholds.contains(&10.0) {
        Some(3)
    } else if remaining_pct < 20.0 && thresholds.contains(&20.0) {
        Some(2)
    } else if remaining_pct < 50.0 && thresholds.contains(&50.0) {
        Some(1)
    } else {
        None
    }
}

impl QuotaAlertEngine {
    /// Evaluate one snapshot. `interval_ms` is the provider's cadence, used
    /// for staleness. Mutations to `memory` are persisted by the caller.
    pub fn evaluate(
        memory: &mut AlertMemory,
        history: &QuotaHistory,
        snap: &ProviderSnapshot,
        rules: &QuotaAlertRules,
        interval_ms: u64,
        now_ms: i64,
    ) -> Vec<AlertEvent> {
        let mut out = Vec::new();
        if !rules.enabled {
            return out;
        }
        let cooldown = |memory: &AlertMemory, key: &str, cd_ms: i64| -> bool {
            memory.fired.get(key).map(|t| now_ms - *t >= cd_ms).unwrap_or(true)
        };

        // 1) usage thresholds per window
        for w in &snap.windows {
            let Some(used) = w.used_percent else { continue };
            let remaining = 100.0 - used;
            let Some(level) = level_for(remaining, &rules.thresholds) else {
                // Usage dropped back below all thresholds (e.g. after a
                // reset) → re-arm: forget the level memory AND the cooldown
                // keys so a genuinely new crossing notifies again.
                let mkey = format!("{}:{}", snap.provider, w.key);
                memory.quota_level.remove(&mkey);
                for l in 1..=3u8 {
                    memory.fired.remove(&format!("quota:{mkey}:{l}"));
                }
                continue;
            };
            let mkey = format!("{}:{}", snap.provider, w.key);
            let should_fire = memory.quota_level.get(&mkey).map(|l| level > *l).unwrap_or(true);
            if should_fire {
                let akey = format!("quota:{mkey}:{level}");
                if cooldown(memory, &akey, 6 * 3600_000) {
                    memory.fired.insert(akey.to_string(), now_ms);
                    memory.quota_level.insert(mkey, level);
                    let (sev, label) = match level {
                        3 => (3u8, "即将耗尽"),
                        2 => (2u8, "额度紧张"),
                        _ => (1u8, "额度过半"),
                    };
                    let body = match (w.remaining_quota, &w.unit) {
                        (Some(r), Some(u)) => format!("{} 剩余 {:.1}%({r:.0} {u})", w.label, remaining),
                        _ => format!("{} 已用 {used:.0}%,剩余 {:.0}%", w.label, remaining),
                    };
                    out.push(AlertEvent {
                        rule: "quota_threshold".into(),
                        severity: sev,
                        title: format!("{} · {}", provider_display(&snap.provider), label),
                        body,
                        ts_ms: now_ms,
                    });
                }
            }
        }

        // 2) package expiry
        for p in &snap.packages {
            let Some(expiry) = p.expiry_ms else { continue };
            if p.status != "Effective" {
                continue;
            }
            let days_left = (expiry - now_ms) / 86_400_000;
            if days_left >= 0 && days_left <= rules.package_expiry_days as i64 {
                let akey = format!("expiry:{}:{}", snap.provider, p.instance_no);
                let day_bucket = now_ms / 86_400_000;
                let last = memory.fired.get(&akey).copied().unwrap_or(0);
                if last / 86_400_000 < day_bucket {
                    memory.fired.insert(akey.to_string(), now_ms);
                    let pct = p
                        .usage_percent
                        .map(|u| format!("剩余 {:.0}%", 100.0 - u))
                        .unwrap_or_default();
                    out.push(AlertEvent {
                        rule: "package_expiry".into(),
                        severity: if days_left <= 2 { 3 } else { 2 },
                        title: format!("{} · Token 包即将到期", provider_display(&snap.provider)),
                        body: format!(
                            "「{}」{} 天后到期,{}",
                            p.name,
                            if days_left == 0 { "今天".into() } else { days_left.to_string() },
                            pct
                        ),
                        ts_ms: now_ms,
                    });
                }
            }
        }

        // 3) provider stale
        let stale_after = (interval_ms as i64 * 3).max(10 * 60_000);
        let age = now_ms - snap.updated_at_ms;
        let meaningful = snap.status == super::ProviderStatus::Stale
            || (snap.status == super::ProviderStatus::Error && age > stale_after);
        if meaningful {
            let akey = format!("stale:{}", snap.provider);
            if cooldown(memory, &akey, ALERT_COOLDOWN_STALE_MS) {
                memory.fired.insert(akey.to_string(), now_ms);
                let hours = age / 3600_000;
                out.push(AlertEvent {
                    rule: "provider_stale".into(),
                    severity: 1,
                    title: format!("{} · 数据未更新", provider_display(&snap.provider)),
                    body: format!("最近一次成功更新于 {hours} 小时前({})", snap.error.clone().unwrap_or_default()),
                    ts_ms: now_ms,
                });
            }
        }

        // 4) daily cost (hub passes the day's cost via a pseudo-snapshot
        //    provider= "zcode" window key "daily_cost"; kept here so all
        //    quota alerts share one engine).
        if rules.daily_cost_cny > 0.0 {
            let from = crate::zcode::aggregate::local_day_start_ms(now_ms);
            if let Some(cost) = history.points("zcode", "daily_cost", from, now_ms).last().and_then(|p| p.used) {
                if cost >= rules.daily_cost_cny {
                    let akey = "cost:daily";
                    let day_bucket = now_ms / 86_400_000;
                    let last = memory.fired.get(akey).copied().unwrap_or(0);
                    if last / 86_400_000 < day_bucket {
                        memory.fired.insert(akey.to_string(), now_ms);
                        out.push(AlertEvent {
                            rule: "cost_threshold".into(),
                            severity: 2,
                            title: "ZCode · 今日 API 等价成本达标".into(),
                            body: format!("今日估算成本 ¥{cost:.2}(阈值 ¥{:.2})", rules.daily_cost_cny),
                            ts_ms: now_ms,
                        });
                    }
                }
            }
        }

        out
    }
}

pub fn provider_display(id: &str) -> &'static str {
    match id {
        super::PROVIDER_ZCODE => "ZCode",
        super::PROVIDER_CODEX => "Codex",
        super::PROVIDER_ANTIGRAVITY => "Antigravity",
        super::PROVIDER_VOLCENGINE => "火山引擎",
        _ => "服务",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{PackageInfo, ProviderStatus, QuotaWindow};

    fn snap(provider: &str, windows: Vec<QuotaWindow>, packages: Vec<PackageInfo>, now: i64) -> ProviderSnapshot {
        let mut s = ProviderSnapshot::empty(provider, ProviderStatus::Ok, now);
        s.windows = windows;
        s.packages = packages;
        s
    }

    #[test]
    fn threshold_fires_once_and_rearms_after_drop() {
        let mut mem = AlertMemory::default();
        let history = QuotaHistory::open_in_memory();
        let rules = QuotaAlertRules::default();
        let mk = |used: f64| {
            snap(
                "codex",
                vec![QuotaWindow { key: "5h".into(), label: "5 小时".into(), used_percent: Some(used), ..Default::default() }],
                vec![],
                1_000_000,
            )
        };

        // 40% used → below-50 threshold fires level 1
        let evs = QuotaAlertEngine::evaluate(&mut mem, &history, &mk(60.0), &rules, 60_000, 2_000_000);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].severity, 1);

        // 75% used, same level → no repeat
        let evs = QuotaAlertEngine::evaluate(&mut mem, &history, &mk(75.0), &rules, 60_000, 2_000_001);
        assert!(evs.is_empty());

        // 85% used → level 2 fires
        let evs = QuotaAlertEngine::evaluate(&mut mem, &history, &mk(85.0), &rules, 60_000, 2_000_002);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].severity, 2);

        // 95% used → level 3
        let evs = QuotaAlertEngine::evaluate(&mut mem, &history, &mk(95.0), &rules, 60_000, 2_000_003);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].severity, 3);

        // reset happens → 10% used: nothing fires, memory re-arms
        let evs = QuotaAlertEngine::evaluate(&mut mem, &history, &mk(10.0), &rules, 60_000, 2_000_004);
        assert!(evs.is_empty());

        // climbs again → level 1 fires again (re-armed)
        let evs = QuotaAlertEngine::evaluate(&mut mem, &history, &mk(60.0), &rules, 60_000, 3_000_000);
        assert_eq!(evs.len(), 1);
    }

    #[test]
    fn cooldown_suppresses_repeated_levels() {
        let mut mem = AlertMemory::default();
        let history = QuotaHistory::open_in_memory();
        let rules = QuotaAlertRules::default();
        let s = snap(
            "volcengine",
            vec![QuotaWindow { key: "packages_total".into(), used_percent: Some(95.0), ..Default::default() }],
            vec![],
            1_000_000,
        );
        assert_eq!(QuotaAlertEngine::evaluate(&mut mem, &history, &s, &rules, 60_000, 1_000_000).len(), 1);
        // A new window (fresh key) at same level within cooldown → suppressed
        let s2 = ProviderSnapshot::empty("codex", ProviderStatus::Ok, 1_000_000);
        let _ = s2;
        // simulate level re-fire attempt after re-arm within 6h cooldown
        mem.quota_level.remove("volcengine:packages_total");
        assert!(QuotaAlertEngine::evaluate(&mut mem, &history, &s, &rules, 60_000, 1_000_001).is_empty());
    }

    #[test]
    fn package_expiry_once_per_day() {
        let mut mem = AlertMemory::default();
        let history = QuotaHistory::open_in_memory();
        let rules = QuotaAlertRules::default();
        let now = 1_788_000_000_000i64;
        let pkg = PackageInfo {
            instance_no: "i-9".into(),
            name: "百万Token包".into(),
            expiry_ms: Some(now + 3 * 86_400_000),
            status: "Effective".into(),
            usage_percent: Some(68.0),
            ..Default::default()
        };
        let s = snap("volcengine", vec![], vec![pkg.clone()], now);
        let evs = QuotaAlertEngine::evaluate(&mut mem, &history, &s, &rules, 1_800_000, now);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].rule, "package_expiry");
        assert!(evs[0].body.contains("3 天后到期"));
        assert!(evs[0].body.contains("剩余 32%"));
        // same day again → nothing
        let evs2 = QuotaAlertEngine::evaluate(&mut mem, &history, &s, &rules, 1_800_000, now + 3600_000);
        assert!(evs2.is_empty());
        // next day → fires again
        let evs3 = QuotaAlertEngine::evaluate(&mut mem, &history, &s, &rules, 1_800_000, now + 86_400_001);
        assert_eq!(evs3.len(), 1);
        // used-up package never alerts
        let mut pkg2 = pkg.clone();
        pkg2.status = "UsedUp".into();
        let s2 = snap("volcengine", vec![], vec![pkg2.clone()], now);
        mem = AlertMemory::default();
        assert!(QuotaAlertEngine::evaluate(&mut mem, &history, &s2, &rules, 1_800_000, now).is_empty());
    }

    #[test]
    fn stale_provider_alerts_with_cooldown() {
        let mut mem = AlertMemory::default();
        let history = QuotaHistory::open_in_memory();
        let rules = QuotaAlertRules::default();
        let mut s = ProviderSnapshot::empty("antigravity", ProviderStatus::Error, 0);
        s.error = Some("rpc 失败".into());
        let now = 100 * 60_000; // age 100 min > 3×2min cadence, > 10 min floor
        let evs = QuotaAlertEngine::evaluate(&mut mem, &history, &s, &rules, 120_000, now);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].rule, "provider_stale");
        // within 12h → suppressed
        assert!(QuotaAlertEngine::evaluate(&mut mem, &history, &s, &rules, 120_000, now + 3600_000).is_empty());
    }

    #[test]
    fn disabled_rules_fire_nothing() {
        let mut mem = AlertMemory::default();
        let history = QuotaHistory::open_in_memory();
        let mut rules = QuotaAlertRules::default();
        rules.enabled = false;
        let s = snap(
            "codex",
            vec![QuotaWindow { key: "5h".into(), used_percent: Some(99.0), ..Default::default() }],
            vec![],
            0,
        );
        assert!(QuotaAlertEngine::evaluate(&mut mem, &history, &s, &rules, 60_000, 5_000_000).is_empty());
    }
}
