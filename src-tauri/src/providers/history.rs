//! Quota snapshot history: SQLite persistence, retention, forecasting.
//!
//! One row per (provider, window, value-change-or-time-quantum) — the insert
//! path dedups so an idle app writes nothing. Retention keeps 400 days of
//! raw points (at ≤ ~6 rows/day/window this stays a few hundred KB for
//! years). The DB lives in the OS cache dir, fully separate from user data
//! sources; it is created with a `user_version` migration framework so
//! future schema changes are additive (never drop-and-recreate).

use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::{Forecast, PackageInfo, ProviderSnapshot, QuotaWindow};

const SCHEMA_VERSION: i64 = 1;
/// Insert a point if the last one for the key is older than this, even when
/// the value is unchanged (so flat lines are still visible).
const FORCE_INTERVAL_MS: i64 = 30 * 60_000;
/// Consider a value change worth recording beyond this relative delta.
const MIN_PERCENT_DELTA: f64 = 0.25;

#[derive(Clone, Debug)]
pub struct HistoryPoint {
    pub ts_ms: i64,
    pub used_percent: Option<f64>,
    pub used: Option<f64>,
    pub remaining: Option<f64>,
}

/// Health of the history store.  Errors are deliberately generic: paths,
/// SQLite diagnostics, and any credential-like data must not cross the IPC
/// boundary.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryHealth {
    pub persistent: bool,
    pub error: Option<String>,
    pub last_success_ms: Option<i64>,
}

pub struct QuotaHistory {
    conn: Connection,
    health: HistoryHealth,
}

const ERR_OPEN: &str = "history database unavailable; using in-memory history";
const ERR_MIGRATION: &str = "history database migration failed; using in-memory history";
const ERR_WRITE: &str = "history persistence failed; changes rolled back";

impl QuotaHistory {
    /// Open (creating if needed) with migrations applied. Errors degrade to
    /// an in-memory DB so history is a nice-to-have, never a crash path.
    pub fn open(path: Option<&Path>) -> QuotaHistory {
        let memory = |error: Option<&str>| {
            let mut h = QuotaHistory {
                conn: Connection::open_in_memory().expect("sqlite in-memory fallback always succeeds"),
                health: HistoryHealth {
                    persistent: false,
                    error: None,
                    last_success_ms: None,
                },
            };
            // Fallback history must remain usable; initialize the same schema
            // in memory before returning it.
            if h.migrate().is_err() {
                h.health.error = Some(ERR_MIGRATION.into());
            } else {
                h.health.error = error.map(str::to_string);
            }
            h
        };
        let Some(path) = path else {
            return memory(None);
        };

        // At first launch the cache dir may not exist yet.  A failure here is
        // a failed disk backend, distinct from the deliberate None/memory
        // mode; do not delete or reset an existing user database.
        if path
            .parent()
            .map(std::fs::create_dir_all)
            .transpose()
            .is_err()
        {
            return memory(Some(ERR_OPEN));
        }
        let conn = match Connection::open(path) {
            Ok(conn) => conn,
            Err(_) => return memory(Some(ERR_OPEN)),
        };
        let mut h = QuotaHistory {
            conn,
            health: HistoryHealth {
                persistent: true,
                error: None,
                last_success_ms: None,
            },
        };
        if h.migrate().is_err() {
            return memory(Some(ERR_MIGRATION));
        }
        h
    }

    pub fn open_in_memory() -> QuotaHistory {
        Self::open(None)
    }

    pub fn health(&self) -> HistoryHealth {
        self.health.clone()
    }

    fn mark_success(&mut self, ts_ms: i64) {
        // A successful in-memory fallback write does not repair the failed
        // disk backend; retain its open/migration diagnostic so callers can
        // distinguish it from deliberate memory mode.  Transient write
        // errors, however, may clear after a later successful write.
        if self.health.persistent || self.health.error.as_deref() == Some(ERR_WRITE) {
            self.health.error = None;
        }
        self.health.last_success_ms = Some(ts_ms);
    }

    /// Additive migrations keyed by `PRAGMA user_version`. Each step runs in
    /// a transaction; unknown future versions (older binary, newer DB) are
    /// left untouched.
    fn migrate(&mut self) -> rusqlite::Result<()> {
        let version: i64 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < SCHEMA_VERSION {
            self.conn.execute_batch(
                "BEGIN;
                 CREATE TABLE snapshots (
                     id INTEGER PRIMARY KEY,
                     ts_ms INTEGER NOT NULL,
                     provider TEXT NOT NULL,
                     window_key TEXT NOT NULL,
                     used_percent REAL,
                     total REAL,
                     used REAL,
                     remaining REAL
                 );
                 CREATE INDEX idx_snap_key_ts ON snapshots(provider, window_key, ts_ms);
                 PRAGMA user_version = 1;
                 COMMIT;",
            )?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> i64 {
        self.conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0)
    }

    /// Record a snapshot's windows (and package "windows") with change-based
    /// dedup. A failed provider still records nothing — errors don't pollute
    /// trends.
    pub fn record(&mut self, snap: &ProviderSnapshot) {
        if snap.status != super::ProviderStatus::Ok {
            return;
        }
        let mut rows: Vec<(String, QuotaWindow)> = snap
            .windows
            .iter()
            .map(|w| (w.key.clone(), w.clone()))
            .collect();
        for p in &snap.packages {
            rows.push((
                format!("package:{}", p.instance_no),
                QuotaWindow {
                    key: format!("package:{}", p.instance_no),
                    label: p.name.clone(),
                    used_percent: p.usage_percent,
                    total_quota: Some(p.total_amount),
                    used_quota: Some(p.used_amount),
                    remaining_quota: Some(p.available_amount),
                    unit: Some(p.unit.clone()),
                    ..Default::default()
                },
            ));
        }
        let tx = match self.conn.transaction() {
            Ok(tx) => tx,
            Err(_) => {
                self.health.error = Some(ERR_WRITE.into());
                return;
            }
        };
        let mut wrote = false;
        for (key, w) in rows {
            let effective = w.effective_percent();
            if effective.is_none()
                && w.remaining_quota.is_none()
                && w.used_quota.is_none()
            {
                continue; // nothing quantifiable to track
            }
            if !Self::should_insert(&tx, &snap.provider, &key, effective, w.remaining_quota, snap.updated_at_ms) {
                continue;
            }
            if tx.execute(
                "INSERT INTO snapshots (ts_ms, provider, window_key, used_percent, total, used, remaining)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    snap.updated_at_ms,
                    snap.provider,
                    key,
                    effective,
                    w.total_quota,
                    w.used_quota,
                    w.remaining_quota,
                ],
            ).is_err() {
                self.health.error = Some(ERR_WRITE.into());
                return;
            }
            wrote = true;
        }
        let cutoff = snap.updated_at_ms - 400 * 24 * 3600_000;
        if tx.execute("DELETE FROM snapshots WHERE ts_ms < ?1", [cutoff]).is_err()
            || tx.commit().is_err()
        {
            self.health.error = Some(ERR_WRITE.into());
            return;
        }
        if wrote {
            self.mark_success(snap.updated_at_ms);
        }
    }

    fn should_insert(
        conn: &Connection,
        provider: &str,
        key: &str,
        percent: Option<f64>,
        remaining: Option<f64>,
        now_ms: i64,
    ) -> bool {
        let Ok((last_ts, last_pct, last_rem)) = conn.query_row(
            "SELECT ts_ms, used_percent, remaining FROM snapshots
             WHERE provider = ?1 AND window_key = ?2 ORDER BY ts_ms DESC, id DESC LIMIT 1",
            [provider, key],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<f64>>(1)?, r.get::<_, Option<f64>>(2)?)),
        ) else {
            return true;
        };
        if now_ms - last_ts >= FORCE_INTERVAL_MS {
            return true;
        }
        let pct_changed = match (percent, last_pct) {
            (Some(a), Some(b)) => (a - b).abs() >= MIN_PERCENT_DELTA,
            (Some(_), None) | (None, Some(_)) => true,
            (None, None) => false,
        };
        let rem_changed = match (remaining, last_rem) {
            (Some(a), Some(b)) => (a - b).abs() >= (b.abs() * 0.001).max(1.0),
            (Some(_), None) | (None, Some(_)) => true,
            (None, None) => false,
        };
        pct_changed || rem_changed
    }

    /// Trend points for one provider window over a time range.
    pub fn points(&self, provider: &str, window_key: &str, from_ms: i64, to_ms: i64) -> Vec<HistoryPoint> {
        let mut stmt = match self.conn.prepare(
            "SELECT ts_ms, used_percent, used, remaining FROM snapshots
             WHERE provider = ?1 AND window_key = ?2 AND ts_ms BETWEEN ?3 AND ?4
             ORDER BY ts_ms ASC",
        ) {            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map(
            rusqlite::params![provider, window_key, from_ms, to_ms],
            |r| {
                Ok(HistoryPoint {
                    ts_ms: r.get(0)?,
                    used_percent: r.get(1)?,
                    used: r.get(2)?,
                    remaining: r.get(3)?,
                })
            },
        );
        match rows {
            Ok(it) => it.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn known_windows(&self, provider: &str) -> Vec<String> {
        match self
            .conn
            .prepare("SELECT DISTINCT window_key FROM snapshots WHERE provider = ?1")
        {
            Ok(mut stmt) => match stmt.query_map([provider], |r| r.get::<_, String>(0)) {
                Ok(it) => it.filter_map(|r| r.ok()).collect(),
                Err(_) => Vec::new(),
            },
            Err(_) => Vec::new(),
        }
    }

    /// Linear-regression depletion forecast over the recent history of a
    /// window. Requires enough spread — insufficient samples → `None`, we
    /// never extrapolate from two points.
    pub fn forecast(&self, provider: &str, window_key: &str, now_ms: i64) -> Option<Forecast> {
        let pts = self.points(provider, window_key, now_ms - 72 * 3600_000, now_ms);
        if pts.len() < 8 {
            return None;
        }
        let first = pts.first()?.ts_ms;
        let last = pts.last()?.ts_ms;
        if last - first < 2 * 3600_000 {
            return None; // need ≥ 2 h of observation
        }
        // Regress remaining(t) — only on points that actually carry one.
        let series: Vec<(f64, f64)> = pts
            .iter()
            .filter_map(|p| p.remaining.map(|r| (p.ts_ms as f64, r)))
            .collect();
        let (slope_per_ms, _intercept) = linear_regression(&series)?;
        if slope_per_ms >= -1e-12 {
            return None; // not consuming (flat/refill) — no meaningful ETA
        }
        let remaining_now = series.last().map(|(_, r)| *r)?;
        if remaining_now <= 0.0 {
            return None;
        }
        let eta_ms = (remaining_now / -slope_per_ms) as i64;
        let span_h = (last - first) as f64 / 3600_000.0;
        let confidence = if series.len() >= 24 && span_h >= 12.0 {
            "high"
        } else if series.len() >= 12 && span_h >= 6.0 {
            "medium"
        } else {
            "low"
        };
        Some(Forecast {
            eta_ms,
            rate_per_day: -slope_per_ms * 86_400_000.0,
            samples: series.len(),
            confidence: confidence.into(),
        })
    }

    /// Daily usage totals for one window over the last N days: sum of the
    /// per-day (max−min remaining) deltas. Used by the history chart.
    pub fn daily_consumption(&self, provider: &str, window_key: &str, days: u32, now_ms: i64) -> Vec<(i64, f64)> {
        let from = now_ms - days as i64 * 24 * 3600_000;
        let pts = self.points(provider, window_key, from, now_ms);
        let mut days_map: Vec<(i64, Vec<f64>)> = Vec::new();
        for p in &pts {
            if let Some(r) = p.remaining {
                let day = p.ts_ms / 86_400_000;
                match days_map.iter_mut().find(|(d, _)| *d == day) {
                    Some((_, v)) => v.push(r),
                    None => days_map.push((day, vec![r])),
                }
            }
        }
        days_map
            .into_iter()
            .map(|(d, vals)| {
                let max = vals.iter().cloned().fold(f64::MIN, f64::max);
                let min = vals.iter().cloned().fold(f64::MAX, f64::min);
                (d, (max - min).max(0.0))
            })
            .collect()
    }

    /// Record the ZCode daily API-equivalent cost (CNY) for threshold
    /// alerts. One row per material change within the day.
    pub fn record_daily_cost(&mut self, day_start_ms: i64, cost_cny: f64, now_ms: i64) {
        if cost_cny <= 0.0 {
            return;
        }
        let changed = self
            .points("zcode", "daily_cost", day_start_ms, now_ms)
            .last()
            .map(|p| (cost_cny - p.used.unwrap_or(0.0)).abs() >= 0.5)
            .unwrap_or(true);
        if !changed {
            return;
        }
        let tx = match self.conn.transaction() {
            Ok(tx) => tx,
            Err(_) => {
                self.health.error = Some(ERR_WRITE.into());
                return;
            }
        };
        if tx.execute(
            "INSERT INTO snapshots (ts_ms, provider, window_key, used_percent, total, used, remaining)
             VALUES (?1, 'zcode', 'daily_cost', NULL, NULL, ?2, NULL)",
            rusqlite::params![now_ms, cost_cny],
        ).is_err() || tx.commit().is_err() {
            self.health.error = Some(ERR_WRITE.into());
            return;
        }
        self.mark_success(now_ms);
    }

    /// Attach forecasts (computed from history) onto a snapshot's windows.
    pub fn enrich_with_forecasts(&self, snap: &mut ProviderSnapshot, now_ms: i64) {
        for w in snap.windows.iter_mut() {
            w.forecast = self.forecast(&snap.provider, &w.key, now_ms);
        }
    }
}

fn linear_regression(series: &[(f64, f64)]) -> Option<(f64, f64)> {
    let n = series.len() as f64;
    if n < 2.0 {
        return None;
    }
    let sx: f64 = series.iter().map(|(x, _)| x).sum();
    let sy: f64 = series.iter().map(|(_, y)| y).sum();
    let sxx: f64 = series.iter().map(|(x, _)| x * x).sum();
    let sxy: f64 = series.iter().map(|(x, y)| x * y).sum();
    let denom = n * sxx - sx * sx;
    if denom.abs() < 1e-9 {
        return None;
    }
    let slope = (n * sxy - sx * sy) / denom;
    let intercept = (sy - slope * sx) / n;
    Some((slope, intercept))
}

/// Helper for package aggregation UIs.
pub fn aggregate_packages(packages: &[PackageInfo]) -> Option<QuotaWindow> {
    let effective: Vec<&PackageInfo> = packages
        .iter()
        .filter(|p| p.status == "Effective" || p.status.is_empty())
        .collect();
    if effective.is_empty() {
        return None;
    }
    let total: f64 = effective.iter().map(|p| p.total_amount * p.unit_multiplier).sum();
    let avail: f64 = effective.iter().map(|p| p.available_amount * p.unit_multiplier).sum();
    let nearest_expiry = effective
        .iter()
        .filter_map(|p| p.expiry_ms)
        .min();
    let unit_mult = effective
        .iter()
        .map(|p| p.unit_multiplier)
        .fold(0.0_f64, f64::max);
    let unit = if unit_mult >= 1000.0 { "tokens".to_string() } else { effective[0].unit.clone() };
    let percent = if total > 0.0 { Some((1.0 - avail / total) * 100.0) } else { None };
    Some(QuotaWindow {
        key: "packages_total".into(),
        label: format!("{} 个有效 Token 包", effective.len()),
        used_percent: percent,
        total_quota: Some(total),
        used_quota: Some(total - avail),
        remaining_quota: Some(avail),
        unit: Some(unit),
        reset_at_ms: nearest_expiry,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{PackageInfo, ProviderStatus, QuotaWindow};

    fn snap_with(provider: &str, key: &str, pct: f64, remaining: f64, now: i64) -> ProviderSnapshot {
        let mut s = ProviderSnapshot::empty(provider, ProviderStatus::Ok, now);
        s.windows.push(QuotaWindow {
            key: key.into(),
            label: "w".into(),
            used_percent: Some(pct),
            remaining_quota: Some(remaining),
            total_quota: Some(1000.0),
            used_quota: Some(1000.0 - remaining),
            ..Default::default()
        });
        s
    }

    #[test]
    fn schema_migrates_to_v1() {
        let h = QuotaHistory::open_in_memory();
        assert_eq!(h.schema_version(), 1);
        assert_eq!(h.health(), HistoryHealth { persistent: false, error: None, last_success_ms: None });
        // reopening is idempotent
        let h2 = QuotaHistory::open_in_memory();
        assert_eq!(h2.schema_version(), 1);
    }

    #[test]
    fn failed_disk_creation_is_reported_without_path_details() {
        let dir = tempfile::tempdir().unwrap();
        let parent_file = dir.path().join("not-a-directory");
        std::fs::write(&parent_file, b"occupied").unwrap();
        let mut h = QuotaHistory::open(Some(&parent_file.join("history.sqlite")));
        let health = h.health();
        assert!(!health.persistent);
        assert!(health.error.is_some());
        assert!(!health.error.unwrap().contains("not-a-directory"));
        h.record(&snap_with("p", "w", 10.0, 900.0, 1234));
        assert_eq!(h.points("p", "w", 0, 2000).len(), 1);
        assert!(!h.health().persistent);
    }

    #[test]
    fn failed_migration_falls_back_without_mutating_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute("CREATE TABLE snapshots (only_one_column INTEGER)", []).unwrap();
        drop(conn);
        let h = QuotaHistory::open(Some(&path));
        assert!(!h.health().persistent);
        assert_eq!(h.health().error.as_deref(), Some(ERR_MIGRATION));
        // The unusable disk database remains in place for recovery/inspection.
        assert!(path.exists());
        let mut h = h;
        h.record(&snap_with("fallback", "w", 10.0, 900.0, 7));
        assert_eq!(h.points("fallback", "w", 0, i64::MAX).len(), 1);
        assert_eq!(h.health().error.as_deref(), Some(ERR_MIGRATION));
    }

    #[test]
    fn record_rolls_back_all_rows_when_one_insert_fails() {
        let mut h = QuotaHistory::open_in_memory();
        h.conn.execute_batch(
            "CREATE TRIGGER reject_b BEFORE INSERT ON snapshots
             WHEN NEW.window_key = 'b'
             BEGIN SELECT RAISE(ABORT, 'test failure'); END;",
        ).unwrap();
        let mut s = snap_with("p", "a", 10.0, 900.0, 42);
        s.windows.push(QuotaWindow {
            key: "b".into(),
            label: "b".into(),
            used_percent: Some(20.0),
            remaining_quota: Some(800.0),
            total_quota: Some(1000.0),
            used_quota: Some(200.0),
            ..Default::default()
        });
        h.record(&s);
        assert!(h.points("p", "a", 0, i64::MAX).is_empty());
        assert_eq!(h.health().error.as_deref(), Some(ERR_WRITE));
    }

    #[test]
    fn healthy_disk_record_reports_success_timestamp_and_camel_case() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("healthy.sqlite");
        let mut h = QuotaHistory::open(Some(&path));
        assert!(h.health().persistent);
        h.record(&snap_with("p", "w", 10.0, 900.0, 1234));
        let health = h.health();
        assert_eq!(health.last_success_ms, Some(1234));
        assert!(health.error.is_none());
        let json = serde_json::to_value(health).unwrap();
        assert_eq!(json["lastSuccessMs"], 1234);
        assert!(json.get("last_success_ms").is_none());
    }

    #[test]
    fn dedups_unchanged_points_but_forces_periodic() {
        let mut h = QuotaHistory::open_in_memory();
        h.record(&snap_with("codex", "5h", 50.0, 500.0, 1_000_000));
        h.record(&snap_with("codex", "5h", 50.0, 500.0, 1_100_000)); // 100s later, same → skipped
        h.record(&snap_with("codex", "5h", 50.0, 500.0, 1_000_000 + FORCE_INTERVAL_MS + 1)); // forced
        assert_eq!(h.points("codex", "5h", 0, i64::MAX).len(), 2);
        h.record(&snap_with("codex", "5h", 50.3, 500.0, 1_200_000)); // ≥0.25pp change
        assert_eq!(h.points("codex", "5h", 0, i64::MAX).len(), 3);
    }

    #[test]
    fn error_snapshots_are_not_recorded() {
        let mut h = QuotaHistory::open_in_memory();
        let mut s = ProviderSnapshot::empty("codex", ProviderStatus::Error, 5);
        s.windows.push(QuotaWindow { key: "5h".into(), used_percent: Some(1.0), ..Default::default() });
        h.record(&s);
        assert!(h.points("codex", "5h", 0, i64::MAX).is_empty());
    }

    #[test]
    fn forecast_needs_samples_and_slope() {
        let mut h = QuotaHistory::open_in_memory();
        // insufficient samples
        for i in 0..5 {
            h.record(&snap_with("p", "w", 10.0, 1000.0 - i as f64, i * 3600_000));
        }
        assert!(h.forecast("p", "w", 10 * 3600_000).is_none());
        // steady −5/hour over 24 points × 1h
        for i in 0..24 {
            h.record(&snap_with("q", "w", 10.0, 1000.0 - 5.0 * i as f64, i * 3600_000));
        }
        let f = h.forecast("q", "w", 23 * 3600_000 + 60_000).expect("forecast");
        assert_eq!(f.samples, 24);
        assert_eq!(f.confidence, "high");
        // remaining 885 at −5/h → ~177h
        assert!((f.eta_ms as f64 / 3600_000.0 - 177.0).abs() < 2.0, "eta={}", f.eta_ms);
        // flat series → none
        for i in 0..24 {
            h.record(&snap_with("r", "w", 10.0, 900.0, i * 3600_000));
        }
        assert!(h.forecast("r", "w", 23 * 3600_000 + 60_000).is_none());
    }

    #[test]
    fn package_aggregation_sums_and_finds_nearest_expiry() {
        let mk = |no: &str, total: f64, avail: f64, expiry: Option<i64>| PackageInfo {
            instance_no: no.into(),
            name: no.into(),
            total_amount: total,
            available_amount: avail,
            used_amount: total - avail,
            unit: "千Token".into(),
            unit_multiplier: 1000.0,
            expiry_ms: expiry,
            ..Default::default()
        };
        let pkgs = vec![
            mk("a", 100.0, 50.0, Some(2_000_000_000)),
            mk("b", 100.0, 80.0, Some(1_000_000_000)),
            PackageInfo { instance_no: "c".into(), status: "UsedUp".into(), ..Default::default() },
        ];
        let w = aggregate_packages(&pkgs).unwrap();
        assert_eq!(w.remaining_quota, Some(130_000.0));
        assert_eq!(w.total_quota, Some(200_000.0));
        assert_eq!(w.reset_at_ms, Some(1_000_000_000));
        assert!((w.used_percent.unwrap() - 35.0).abs() < 1e-6);
    }

    #[test]
    fn daily_consumption_sums_deltas() {
        let mut h = QuotaHistory::open_in_memory();
        let day = 86_400_000i64;
        let base = 50 * day;
        for (i, r) in [1000.0, 900.0, 800.0, 800.0].iter().enumerate() {
            h.record(&snap_with("p", "w", 1.0, *r, base + i as i64 * 3600_000));
        }
        let d = h.daily_consumption("p", "w", 3, base + day);
        assert_eq!(d.len(), 1);
        assert!((d[0].1 - 200.0).abs() < 1e-6);
    }
}
