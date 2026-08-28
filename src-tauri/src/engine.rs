//! Refresh engine: filesystem watcher + incremental refresh + event emission.
//!
//! Threading model:
//! - one **background thread** owns the `notify` watcher (ReadDirectoryChangesW
//!   on Windows). FS events are debounced in-thread, then trigger an
//!   incremental refresh — no busy loops, no high-frequency polling;
//! - its 5-second idle timeout doubles as the housekeeping timer: busy-source
//!   retries, a once-a-minute safety rediscover, and snapshot persistence;
//! - all parsing/aggregation happens on that thread under the engine mutex;
//!   UI queries only ever read aggregated slices.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::Emitter;

use notify::Watcher;
use crate::alerts::{AlertEngine, AlertEvent};
use crate::settings::Settings;
use crate::zcode::aggregate::{self, Agg, ModelStat, SessionSummary};
use crate::zcode::discover::{self, DataLayout};
use crate::zcode::errors::SourceError;
use crate::zcode::jsonl::{self, JsonlSourceState};
use crate::zcode::sqlite::{self, SqliteSourceState};
use crate::zcode::store::UsageStore;
use crate::zcode::usage::UsageRecord;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BootSnapshot {
    pub saved_at_ms: i64,
    /// Local day the "today" numbers refer to (days since epoch).
    pub day_key: i64,
    pub today_agg: Agg,
    pub today_models: Vec<ModelStat>,
    pub all_models: Vec<ModelStat>,
    pub sessions: Vec<SessionSummary>,
    pub last_record_ms: Option<i64>,
    pub record_count: u64,
}

impl Default for BootSnapshot {
    fn default() -> Self {
        Self {
            saved_at_ms: 0,
            day_key: 0,
            today_agg: Agg::default(),
            today_models: Vec::new(),
            all_models: Vec::new(),
            sessions: Vec::new(),
            last_record_ms: None,
            record_count: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageUpdateEvent {
    pub record_count: u64,
    pub last_refresh_ms: Option<i64>,
    pub last_record_ms: Option<i64>,
    pub error: Option<String>,
    pub paused: bool,
    pub restored_from_cache: bool,
    /// true while the UI is hidden (auto-suspend of polling).
    pub suspended: bool,
}

pub struct EngineInner {
    pub store: UsageStore,
    pub jsonl: HashMap<PathBuf, JsonlSourceState>,
    pub sqlite: HashMap<PathBuf, SqliteSourceState>,
    pub layout: Option<DataLayout>,
    pub last_refresh: Option<i64>,
    pub last_error: Option<String>,
    busy_until_ms: Option<i64>,
    pub boot: Option<BootSnapshot>,
    pub alerts: AlertEngine,
    pub alert_log: Vec<AlertEvent>,
    last_emit: Option<Instant>,
    last_discover: Option<Instant>,
    last_alert_check: Option<Instant>,
}

#[derive(Clone)]
pub struct Engine {
    pub inner: Arc<Mutex<EngineInner>>,
    tx: Sender<()>,
    app: Arc<OnceLock<tauri::AppHandle>>,
    pricing: Arc<OnceLock<Arc<crate::zcode::pricing::PricingManager>>>,
    pub snapshot_dirty: Arc<AtomicBool>,
    /// Auto-suspend flag: true while the UI is hidden to tray.
    auto_paused: Arc<AtomicBool>,
}

impl Engine {
    pub fn new() -> (Self, Receiver<()>) {
        let (tx, rx) = mpsc::channel::<()>();
        let engine = Engine {
            inner: Arc::new(Mutex::new(EngineInner {
                store: UsageStore::new(),
                jsonl: HashMap::new(),
                sqlite: HashMap::new(),
                layout: None,
                last_refresh: None,
                last_error: None,
                busy_until_ms: None,
                boot: None,
                alerts: AlertEngine::new(),
                alert_log: Vec::new(),
                last_emit: None,
                last_discover: None,
                last_alert_check: None,
            })),
            tx,
            app: Arc::new(OnceLock::new()),
            pricing: Arc::new(OnceLock::new()),
            snapshot_dirty: Arc::new(AtomicBool::new(true)),
            auto_paused: Arc::new(AtomicBool::new(false)),
        };
        (engine, rx)
    }

    pub fn set_app(&self, app: tauri::AppHandle) {
        let _ = self.app.set(app);
    }

    /// Register the pricing manager so the idle loop can schedule the daily
    /// FX / remote price-table background refresh.
    pub fn set_pricing(&self, pricing: Arc<crate::zcode::pricing::PricingManager>) {
        let _ = self.pricing.set(pricing);
    }

    /// Force a refresh soon (used by settings changes / manual refresh).
    pub fn kick(&self) {
        let _ = self.tx.send(());
    }

    /// Auto-suspend flag driven by UI visibility. Returns true if the flag
    /// value changed (caller may then kick a refresh on resume).
    pub fn set_auto_paused(&self, paused: bool) -> bool {
        self.auto_paused.swap(paused, Ordering::Relaxed) != paused
    }

    // -- background loop ------------------------------------------------------

    /// Runs on the dedicated engine thread. Never panics: every step degrades.
    pub fn run_background(
        self,
        rx: Receiver<()>,
        get_settings: impl Fn() -> Settings + Send + Sync + 'static,
    ) {
        let (fs_tx, fs_rx) = mpsc::channel::<notify::Result<notify::Event>>();
        // Holds the active watcher so it is not dropped (underscore-prefixed:
        // never read, only kept alive).
        let mut _watcher: Option<notify::RecommendedWatcher> = None;
        let mut watched_root: Option<PathBuf> = None;
        let mut debounce_deadline: Option<Instant> = None;
        let mut tick_count: u64 = 0;
        let pricing = self.pricing.clone();

        loop {
            // Wake: debounce poll (fast) | idle cadence (5 s, 30 s while the
            // UI is hidden) | explicit kick.
            let auto_paused = self.auto_paused.load(Ordering::Relaxed);
            let timeout = if debounce_deadline.is_some() {
                Duration::from_millis(25)
            } else if auto_paused {
                Duration::from_secs(30)
            } else {
                Duration::from_secs(5)
            };
            let mut explicit_kick = match rx.recv_timeout(timeout) {
                Ok(()) => true,
                Err(mpsc::RecvTimeoutError::Timeout) => false,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };

            let settings = get_settings();
            let now = Instant::now();
            tick_count += 1;

            // Daily pricing refresh (FX + optional remote price table) runs on
            // a detached thread — it never blocks the ingest/refresh path.
            if let Some(pm) = pricing.get().cloned() {
                if pm.try_claim_refresh(now_ms(), settings.pricing_remote_url.as_deref()) {
                    let url = settings.pricing_remote_url.clone();
                    pm.spawn_background_refresh(url);
                }
            }

            // (Re)subscribe the FS watcher if the data root changed.
            match discover::resolve_root(settings.data_dir.as_deref()) {
                Some(root) if watched_root.as_deref() != Some(root.as_path()) => {
                    match notify::recommended_watcher(fs_tx.clone()) {
                        Ok(mut w) => {
                            if w.watch(&root, notify::RecursiveMode::Recursive).is_ok() {
                                watched_root = Some(root);
                                _watcher = Some(w);
                                explicit_kick = true;
                            }
                        }
                        Err(e) => {
                            self.inner.lock().unwrap().last_error =
                                Some(format!("fs watcher failed: {e}"));
                        }
                    }
                }
                _ => {}
            }

            // Debounce FS events → one refresh.
            while fs_rx.try_recv().is_ok() {
                debounce_deadline =
                    Some(now + Duration::from_millis(settings.refresh_debounce_ms.min(5000)));
            }
            let debounce_fired = debounce_deadline
                .map(|d| now >= d)
                .unwrap_or(false);
            if debounce_fired {
                debounce_deadline = None;
            }

            if explicit_kick || debounce_fired {
                self.refresh_once(&settings);
            } else if tick_count % 1 == 0 {
                // busy-source retry when its backoff elapsed
                let due = {
                    let inner = self.inner.lock().unwrap();
                    inner.busy_until_ms.map(|t| now_ms() >= t).unwrap_or(false)
                };
                if due {
                    self.refresh_once(&settings);
                }
            }

            // Safety-net rediscover once a minute (missed FS events, moved
            // files). Skipped while the UI is hidden.
            if tick_count % 12 == 0 && !explicit_kick && !debounce_fired && !auto_paused {
                self.refresh_once(&settings);
            }
            if tick_count % 12 == 0 && self.snapshot_dirty.swap(false, Ordering::Relaxed) {
                self.save_snapshot();
            }
        }
    }

    // -- refresh cycle ---------------------------------------------------------

    pub fn refresh_once(&self, settings: &Settings) {
        let now = now_ms();
        let mut new_records: Vec<UsageRecord> = Vec::new();
        let mut errors: Vec<String> = Vec::new();
        let mut busy = false;
        let mut changed = false;
        let mut new_alerts: Vec<AlertEvent> = Vec::new();
        let mut emit_now = false;
        let mut payload: Option<UsageUpdateEvent> = None;

        {
            let mut inner = self.inner.lock().unwrap();

            if settings.monitoring_paused || self.auto_paused.load(Ordering::Relaxed) {
                inner.last_refresh = Some(now);
                return;
            }

            // Rate-limited directory re-discovery (listing only — record reads
            // resume from per-file watermarks, never full rescans).
            let need_discover = inner
                .last_discover
                .map(|t| t.elapsed() > Duration::from_secs(5))
                .unwrap_or(true);
            if need_discover {
                match discover::resolve_root(settings.data_dir.as_deref()) {
                    Some(root) => match discover::discover(&root) {
                        Ok(l) => {
                            let jsonl_set: std::collections::HashSet<_> =
                                l.jsonl_files.iter().cloned().collect();
                            inner.jsonl.retain(|p, _| jsonl_set.contains(p));
                            for p in &l.jsonl_files {
                                inner
                                    .jsonl
                                    .entry(p.clone())
                                    .or_insert_with(|| JsonlSourceState::new(p.clone()));
                            }
                            let sqlite_set: std::collections::HashSet<_> =
                                l.sqlite_files.iter().cloned().collect();
                            inner.sqlite.retain(|p, _| sqlite_set.contains(p));
                            for p in &l.sqlite_files {
                                inner
                                    .sqlite
                                    .entry(p.clone())
                                    .or_insert_with(|| SqliteSourceState::new(p.clone()));
                            }
                            inner.layout = Some(l);
                        }
                        Err(e) => errors.push(format!("scan failed: {e}")),
                    },
                    None => errors.push(format!(
                        "data directory not found (configured: {:?}) — expected <home>/.zcode or ZCODE_HOME",
                        settings.data_dir
                    )),
                }
                inner.last_discover = Some(Instant::now());
            }

            // JSONL incremental reads
            let paths: Vec<PathBuf> = inner.jsonl.keys().cloned().collect();
            for path in paths {
                let Some(state) = inner.jsonl.get_mut(&path) else { continue };
                match jsonl::read_new(state) {
                    Ok(recs) => {
                        if !recs.is_empty() {
                            changed = true;
                        }
                        new_records.extend(recs);
                    }
                    Err(SourceError::Busy) | Err(SourceError::RetryLater(_)) => busy = true,
                    Err(SourceError::Gone) => {
                        inner.jsonl.remove(&path);
                    }
                    Err(SourceError::Fatal(why)) => {
                        errors.push(format!("{}: {why}", path.display()));
                        inner.jsonl.remove(&path);
                    }
                }
            }

            // SQLite incremental reads
            let paths: Vec<PathBuf> = inner.sqlite.keys().cloned().collect();
            for path in paths {
                let Some(state) = inner.sqlite.get_mut(&path) else { continue };
                match sqlite::read_new(state) {
                    Ok(recs) => {
                        if !recs.is_empty() {
                            changed = true;
                        }
                        new_records.extend(recs);
                    }
                    Err(SourceError::Busy) | Err(SourceError::RetryLater(_)) => busy = true,
                    Err(SourceError::Gone) => {
                        inner.sqlite.remove(&path);
                    }
                    Err(SourceError::Fatal(why)) => {
                        errors.push(format!("{}: {why}", path.display()));
                        inner.sqlite.remove(&path);
                    }
                }
            }

            if !new_records.is_empty() {
                inner.store.ingest(new_records);
                self.snapshot_dirty.store(true, Ordering::Relaxed);
            }

            inner.last_refresh = Some(now);
            inner.busy_until_ms = busy.then_some(now + 2000);
            inner.last_error = errors.into_iter().next();

            // Boot snapshot stays authoritative only until real data lands.
            if !inner.store.is_empty() {
                if inner.boot.is_some() {
                    inner.boot = None;
                    changed = true;
                }
                inner.store.restored_from_cache = false;
            }

            // Anomaly detection after data changes, and at least once a
            // minute so time-based rules (data staleness) can fire even
            // when nothing changed.
            let alert_check_due = inner
                .last_alert_check
                .map(|t| t.elapsed() > Duration::from_secs(60))
                .unwrap_or(true);
            if changed || alert_check_due {
                inner.last_alert_check = Some(Instant::now());
                let inner = &mut *inner;
                new_alerts = inner.alerts.evaluate(&mut inner.store, &settings.notifications, now);
                for ev in &new_alerts {
                    inner.alert_log.insert(0, ev.clone());
                    if inner.alert_log.len() > 50 {
                        inner.alert_log.pop();
                    }
                }
            }

            // Emit "usage-update" at most ~2×/s.
            emit_now = changed
                || inner
                    .last_emit
                    .map(|t| t.elapsed() > Duration::from_millis(500))
                    .unwrap_or(true);
            if emit_now {
                inner.last_emit = Some(Instant::now());
                payload = Some(UsageUpdateEvent {
                    record_count: inner.store.len() as u64,
                    last_refresh_ms: inner.last_refresh,
                    last_record_ms: inner.store.last_record_ms,
                    error: inner.last_error.clone(),
                    paused: settings.monitoring_paused,
                    restored_from_cache: inner.boot.is_some(),
                    suspended: self.auto_paused.load(Ordering::Relaxed),
                });
            }
        } // lock released

        if let Some(app) = self.app.get() {
            use tauri_plugin_notification::NotificationExt;
            if let Some(p) = &payload {
                let _ = app.emit("usage-update", p);
            }
            for ev in &new_alerts {
                let _ = app.emit("alert", ev);
                let _ = app
                    .notification()
                    .builder()
                    .title(ev.title.clone())
                    .body(ev.body.clone())
                    .show();
            }
        }
    }

    // -- snapshot persistence ---------------------------------------------------

    pub fn load_snapshot(&self, app: &tauri::AppHandle) {
        use tauri::Manager;
        let Ok(dir) = app.path().app_cache_dir() else { return };
        let path = dir.join("boot-snapshot.json");
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(snap) = serde_json::from_str::<BootSnapshot>(&text) {
                let mut inner = self.inner.lock().unwrap();
                if inner.store.is_empty() {
                    inner.store.restored_from_cache = true;
                    inner.boot = Some(snap);
                }
            }
        }
    }

    pub fn save_snapshot(&self) {
        let Some(app) = self.app.get().cloned() else { return };
        use tauri::Manager;
        let Ok(dir) = app.path().app_cache_dir() else { return };
        let snapshot = {
            let mut inner = self.inner.lock().unwrap();
            if inner.store.is_empty() {
                return;
            }
            let now = now_ms();
            let today_from = aggregate::local_day_start_ms(now);
            let today_records = inner.store.range(today_from, now);
            let today_agg = today_records.iter().fold(Agg::default(), |mut a, r| {
                a.add(r);
                a
            });
            let today_models = aggregate::group_by_model(today_records);
            let all_models = aggregate::group_by_model(inner.store.all());
            let sessions: Vec<SessionSummary> = inner.store.session_summaries().to_vec();
            BootSnapshot {
                saved_at_ms: now,
                day_key: today_from / 86_400_000,
                today_agg,
                today_models,
                all_models,
                sessions,
                last_record_ms: inner.store.last_record_ms,
                record_count: inner.store.len() as u64,
            }
        };
        let _ = std::fs::create_dir_all(&dir);
        let tmp = dir.join("boot-snapshot.json.tmp");
        if let Ok(json) = serde_json::to_string(&snapshot) {
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, dir.join("boot-snapshot.json"));
            }
        }
    }
}
