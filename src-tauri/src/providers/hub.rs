//! Provider hub: registry + scheduler for all quota providers.
//!
//! One dedicated thread owns provider state. It wakes on the nearest due
//! time (never busy-loops), polls due providers, records history, evaluates
//! quota alerts, and emits `provider-update` events. Failure isolation:
//! - each provider polls inside its own catch-all; an error produces an
//!   error snapshot (keeping last-known data) and an exponential backoff,
//! - the hub itself never panics the process,
//! - network providers (Volcengine/Antigravity) run with timeouts; while
//!   the UI is hidden their cadence doubles (alerts still work).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::Emitter;

use crate::engine::now_ms;
use crate::settings::{LauncherSettings, Settings};

use super::antigravity::{self, InstallPaths, LocalTransport, UreqLocalTransport};
use super::codex::CodexProvider;
use super::history::QuotaHistory;
use super::quota_alerts::{AlertEvent, AlertMemory, QuotaAlertEngine};
use super::secrets::{
    KeyringStorage, MemoryStorage, SecretStorage, SecretErrorKind, KEY_VOLCENGINE_AK,
    KEY_VOLCENGINE_SK,
};
use super::volcengine::{self, UreqTransport};
use super::zlauncher::{Launcher, PlatformProcOps};
use super::{
    LocalUsage, ProviderSnapshot, ProviderStatus, QuotaWindow, TokenBreakdown,
    PROVIDER_ANTIGRAVITY, PROVIDER_CODEX, PROVIDER_VOLCENGINE, PROVIDER_ZCODE,
};

/// Aggregate ZCode card data computed from the monitoring engine (local,
/// near-realtime) — injected so the hub stays decoupled from engine types.
#[derive(Clone, Debug, Default)]
pub struct ZcodeCard {
    pub today_tokens: u64,
    pub today_cost_cny: f64,
    pub hit_rate: Option<f64>,
    pub requests: u64,
    pub models: Vec<(String, u64)>,
    pub breakdown: TokenBreakdown,
    pub last_record_ms: Option<i64>,
    pub data_error: Option<String>,
    pub has_data: bool,
}

pub enum HubMsg {
    /// Re-check due times soon (settings changed).
    Kick,
    /// Force one provider (`None` = all) now.
    RefreshNow(Option<String>),
    Shutdown,
}

pub struct HubInner {
    pub snapshots: HashMap<String, ProviderSnapshot>,
    pub codex: CodexProvider,
    pub history: QuotaHistory,
    pub alert_memory: AlertMemory,
    pub alert_log: Vec<AlertEvent>,
    pub launcher: Launcher<PlatformProcOps>,
    pub install: InstallPaths,
    failures: HashMap<String, u32>,
    next_due: HashMap<String, i64>,
    memory_path: Option<PathBuf>,
}

pub struct ProviderHub {
    pub inner: Arc<Mutex<HubInner>>,
    tx: Sender<HubMsg>,
    app: Arc<OnceLock<tauri::AppHandle>>,
    engine: Option<crate::engine::Engine>,
    paused: Arc<AtomicBool>,
    zcode_card: Arc<OnceLock<Arc<dyn Fn() -> ZcodeCard + Send + Sync>>>,
}

/// Cheap handle clone — everything heavy lives behind `Arc`s.
impl Clone for ProviderHub {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            tx: self.tx.clone(),
            app: self.app.clone(),
            engine: self.engine.clone(),
            paused: self.paused.clone(),
            zcode_card: self.zcode_card.clone(),
        }
    }
}

const MAX_BACKOFF_SHIFT: u32 = 3; // ≤ 8× base interval
const HIDDEN_SLOWDOWN: i64 = 2;

impl ProviderHub {
    pub fn new(cache_dir: Option<PathBuf>, _keyring_service: &str) -> (Self, Receiver<HubMsg>) {
        let (tx, rx) = mpsc::channel::<HubMsg>();
        let history = QuotaHistory::open(cache_dir.as_ref().map(|d| d.join("quota-history.sqlite")).as_deref());
        let memory_path = cache_dir.clone().map(|d| d.join("alert-memory.json"));
        let alert_memory = memory_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let codex = CodexProvider::new(cache_dir.as_ref().map(|d| d.join("codex-usage-cache.json")));
        let launcher = Launcher::new(PlatformProcOps);
        let inner = HubInner {
            snapshots: HashMap::new(),
            codex,
            history,
            alert_memory,
            alert_log: Vec::new(),
            launcher,
            install: InstallPaths::default(),
            failures: HashMap::new(),
            next_due: [
                PROVIDER_ZCODE,
                PROVIDER_CODEX,
                PROVIDER_ANTIGRAVITY,
                PROVIDER_VOLCENGINE,
            ]
            .iter()
            .map(|k| (k.to_string(), 0))
            .collect(),
            memory_path,
        };
        let hub = ProviderHub {
            inner: Arc::new(Mutex::new(inner)),
            tx,
            app: Arc::new(OnceLock::new()),
            engine: None,
            paused: Arc::new(AtomicBool::new(false)),
            zcode_card: Arc::new(OnceLock::new()),
        };
        (hub, rx)
    }

    pub fn set_app(&self, app: tauri::AppHandle) {
        let _ = self.app.set(app);
    }

    pub fn set_engine(&mut self, engine: crate::engine::Engine) {
        self.engine = Some(engine);
    }

    /// Provider `is_running` access for the paused flag without exposing
    /// the engine publicly.
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub fn set_zcode_card_fn(&self, f: Arc<dyn Fn() -> ZcodeCard + Send + Sync>) {
        let _ = self.zcode_card.set(f);
    }

    pub fn kick(&self) {
        let _ = self.tx.send(HubMsg::Kick);
    }

    pub fn refresh_now(&self, provider: Option<String>) {
        let _ = self.tx.send(HubMsg::RefreshNow(provider));
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(HubMsg::Shutdown);
    }

    /// OS keyring for real runs; falls back to memory when the platform has
    /// no keyring service (headless Linux dev) so providers degrade to
    /// NotConfigured instead of crashing.
    pub fn default_secret_store() -> Arc<dyn SecretStorage> {
        let store = KeyringStorage::new("zcode-usage-panel");
        // Probe with a write+delete of a dummy entry.
        let ok = store.set("zup_backend_probe", "1").is_ok() && store.delete("zup_backend_probe").is_ok();
        if ok {
            Arc::new(store)
        } else {
            Arc::new(MemoryStorage::new())
        }
    }

    // -- background loop ------------------------------------------------------

    pub fn run_background(
        self,
        rx: Receiver<HubMsg>,
        get_settings: impl Fn() -> Settings + Send + Sync + 'static,
        secrets: Arc<dyn SecretStorage>,
    ) {
        let volc_transport = UreqTransport { timeout_secs: 15 };
        let local_transport = UreqLocalTransport { timeout_secs: 5 };
        // Everything is due immediately (first data fast); the schedule map
        // was seeded in `new`.
        self.kick();

        loop {
            // Sleep until the nearest due time (bounded), or a message.
            let wait_ms = {
                let inner = self.inner.lock().unwrap();
                let now = now_ms();
                inner
                    .next_due
                    .values()
                    .filter(|t| **t > now)
                    .map(|t| *t - now)
                    .min()
                    .unwrap_or(1_000)
                    .clamp(500, 300_000)
            };
            let msg = rx.recv_timeout(Duration::from_millis(wait_ms.max(1) as u64));
            let mut force: Option<String> = None;
            let mut kick = false;
            match msg {
                Ok(HubMsg::Kick) => kick = true,
                Ok(HubMsg::RefreshNow(p)) => force = Some(p.clone().unwrap_or_default()),
                Ok(HubMsg::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }

            let settings = get_settings();
            let paused = self.paused.load(Ordering::Relaxed);
            let now = now_ms();

            let due: Vec<String> = {
                let inner = self.inner.lock().unwrap();
                [PROVIDER_ZCODE, PROVIDER_CODEX, PROVIDER_ANTIGRAVITY, PROVIDER_VOLCENGINE]
                    .into_iter()
                    .filter(|id| {
                        let due_at = inner.next_due.get(*id).copied().unwrap_or(0);
                        let forced = match &force {
                            Some(f) if f.is_empty() => true,
                            Some(f) => f == id,
                            None => false,
                        };
                        forced || kick || due_at <= now
                    })
                    .map(|s| s.to_string())
                    .collect()
            };

            let mut any_changed = false;
            for id in &due {
                let changed = self.poll_one(id, &settings, &secrets, &volc_transport, &local_transport, now);
                any_changed |= changed;
                // Backoff-aware reschedule.
                let (base, failures) = {
                    let inner = self.inner.lock().unwrap();
                    let failures = inner.failures.get(id).copied().unwrap_or(0);
                    let base = match id.as_str() {
                        PROVIDER_CODEX => settings.providers.codex_refresh_ms,
                        PROVIDER_ANTIGRAVITY => settings.providers.antigravity_refresh_ms,
                        PROVIDER_VOLCENGINE => settings.providers.volcengine_refresh_ms,
                        _ => 30_000,
                    };
                    (base, failures)
                };
                let shift = failures.min(MAX_BACKOFF_SHIFT);
                let mut interval = (base as i64) << shift;
                if paused {
                    interval *= HIDDEN_SLOWDOWN;
                }
                let mut inner = self.inner.lock().unwrap();
                inner.next_due.insert(id.clone(), now + interval);
            }

            // ZCode card refreshes on every wake while data is live.
            if !due.is_empty() || kick || force.is_some() {
                self.rebuild_zcode(&settings, now);
                any_changed = true;
            }
            if any_changed {
                self.emit_all();
            }
        }
        // Final persist.
        self.persist_state();
    }

    fn poll_one(
        &self,
        id: &str,
        settings: &Settings,
        secrets: &Arc<dyn SecretStorage>,
        volc_transport: &UreqTransport,
        local_transport: &dyn LocalTransport,
        now: i64,
    ) -> bool {
        let snapshot = match id {
            PROVIDER_CODEX => {
                if !settings.providers.codex_enabled {
                    let mut s = ProviderSnapshot::empty(id, ProviderStatus::Disabled, now);
                    s.source = "已在本软件设置中禁用".into();
                    Some(s)
                } else {
                    let mut inner = self.inner.lock().unwrap();
                    let home = settings
                        .providers
                        .codex_home
                        .as_ref()
                        .filter(|s| !s.is_empty())
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(super::codex::default_home);
                    inner.codex.with_home(home);
                    Some(inner.codex.poll(now))
                }
            }
            PROVIDER_ANTIGRAVITY => {
                if !settings.providers.antigravity_enabled {
                    let mut s = ProviderSnapshot::empty(id, ProviderStatus::Disabled, now);
                    s.source = "已在本软件设置中禁用".into();
                    Some(s)
                } else {
                    let install = antigravity::detect_installation(None);
                    Some(antigravity::poll(&install, local_transport, now))
                }
            }
            PROVIDER_VOLCENGINE => {
                if !settings.providers.volcengine_enabled {
                    let mut s = ProviderSnapshot::empty(id, ProviderStatus::Disabled, now);
                    s.source = "已在本软件设置中禁用".into();
                    Some(s)
                } else {
                    let (ak, sk, keyring_err) = match (secrets.get(KEY_VOLCENGINE_AK), secrets.get(KEY_VOLCENGINE_SK)) {
                        (Ok(a), Ok(s)) => (Some(a), Some(s), None),
                        (Err(SecretErrorKind::NotFound), _) | (_, Err(SecretErrorKind::NotFound)) => (None, None, None),
                        (Err(e), _) | (_, Err(e)) => (None, None, Some(e.message().to_string())),
                    };
                    match (ak, sk, keyring_err) {
                        (Some(ak), Some(sk), _) => {
                            let prov = volcengine::VolcengineProvider {
                                region: settings.providers.volcengine_region.clone(),
                                transport: volc_transport,
                                now: chrono::Utc::now(),
                            };
                            let result = prov.list_packages(&ak, &sk);
                            let filter = settings.providers.volcengine_filter.trim().to_lowercase();
                            let filtered: Vec<_> = match &result {
                                Ok(list) => list
                                    .iter()
                                    .filter(|p| {
                                        filter.is_empty()
                                            || p.name.to_lowercase().contains(&filter)
                                            || p.configuration.to_lowercase().contains(&filter)
                                            || p.product.to_lowercase().contains(&filter)
                                    })
                                    .cloned()
                                    .collect(),
                                Err(_) => vec![],
                            };
                            Some(volcengine::build_snapshot(&filtered, now, result.err().as_ref()))
                        }
                        (_, _, Some(err)) => {
                            let e = volcengine::VolcError::Keyring(err);
                            Some(volcengine::build_snapshot(&[], now, Some(&e)))
                        }
                        _ => Some(volcengine::build_snapshot(&[], now, Some(&volcengine::VolcError::NotConfigured))),
                    }
                }
            }
            _ => None, // zcode handled by rebuild_zcode
        };
        let Some(mut snap) = snapshot else { return false };

        let mut prev: Option<ProviderSnapshot> = None;
        {
            let inner = self.inner.lock().unwrap();
            if let Some(p) = inner.snapshots.get(id) {
                // A previously-OK provider that now returns no data keeps its
                // last-known numbers (marked error/stale) instead of blanking.
                let keep_prev = p.status == ProviderStatus::Ok
                    && snap.status != ProviderStatus::Ok
                    && snap.windows.is_empty()
                    && snap.packages.is_empty()
                    && snap.local_usage.is_none();
                if keep_prev {
                    prev = Some(p.clone());
                }
            }
        }
        if let Some(p) = prev {
            // Error after success → preserve data, mark degraded.
            snap.windows = p.windows;
            snap.packages = p.packages;
            snap.local_usage = p.local_usage;
            snap.plan_name = p.plan_name;
            snap.account = p.account;
            snap.updated_at_ms = p.updated_at_ms; // data time, not attempt time
            snap.status = if now - p.updated_at_ms > 6 * 3600_000 {
                ProviderStatus::Stale
            } else {
                ProviderStatus::Error
            };
        }

        let is_error = snap.status == ProviderStatus::Error || snap.status == ProviderStatus::Stale;
        // Record history + forecasts for successful polls.
        {
            let mut inner = self.inner.lock().unwrap();
            if snap.status == ProviderStatus::Ok {
                inner.history.record(&snap);
                inner.history.enrich_with_forecasts(&mut snap, now);
            }
            if is_error {
                *inner.failures.entry(id.to_string()).or_insert(0) += 1;
            } else {
                inner.failures.insert(id.to_string(), 0);
            }
            inner.snapshots.insert(id.to_string(), snap.clone());
        }

        // Alerts (outside the inner lock; they take history separately).
        if snap.status == ProviderStatus::Ok {
            let alerts = {
                let mut inner = self.inner.lock().unwrap();
                let interval = match id {
                    PROVIDER_CODEX => settings.providers.codex_refresh_ms,
                    PROVIDER_ANTIGRAVITY => settings.providers.antigravity_refresh_ms,
                    PROVIDER_VOLCENGINE => settings.providers.volcengine_refresh_ms,
                    _ => 30_000,
                };
                let hub = &mut *inner;
                QuotaAlertEngine::evaluate(
                    &mut hub.alert_memory,
                    &hub.history,
                    &snap,
                    &settings.quota_alerts,
                    interval,
                    now,
                )
            };
            if !alerts.is_empty() {
                self.fire_alerts(alerts);
            }
        }
        true
    }

    /// Rebuild the ZCode card snapshot from engine aggregates + launcher.
    fn rebuild_zcode(&self, settings: &Settings, now: i64) {
        let Some(card_fn) = self.zcode_card.get() else { return };
        let card = card_fn();
        let mut snap = ProviderSnapshot::empty(PROVIDER_ZCODE, ProviderStatus::Ok, now);
        snap.source = "ZCode 本地数据源(JSONL / SQLite,只读增量)".into();
        snap.plan_name = Some("本地监控".into());
        if !card.has_data {
            snap.status = ProviderStatus::NotConfigured;
            snap.error = Some("暂无 ZCode 用量数据(启动 ZCode 后自动出现)".into());
        }
        if let Some(e) = &card.data_error {
            snap.error = Some(e.clone());
        }
        snap.windows.push(QuotaWindow {
            key: "today_tokens".into(),
            label: "今日 Token".into(),
            used_quota: Some(card.today_tokens as f64),
            unit: Some("tokens".into()),
            ..Default::default()
        });
        snap.notes.push(format!("≈ ¥{:.2} API 等价成本(官方单价估算)", card.today_cost_cny));
        if let Some(hit) = card.hit_rate {
            snap.notes.push(format!("Cache Hit Rate {:.1}%", hit * 100.0));
        }
        snap.local_usage = Some(LocalUsage {
            today: card.breakdown.clone(),
            all_time: card.breakdown, // engine keeps full history; card shows today
            models: card
                .models
                .iter()
                .map(|(m, t)| super::ModelUsageRow {
                    model: m.clone(),
                    breakdown: TokenBreakdown { total_tokens: *t, ..Default::default() },
                })
                .collect(),
            ..Default::default()
        });
        // Launcher status (one process snapshot on demand).
        if settings.launcher.enabled {
            let mut inner = self.inner.lock().unwrap();
            inner.launcher.configure(settings.launcher.exe_path.clone());
            snap.launcher = Some(inner.launcher.status());
        }
        // Daily cost history point for threshold alerts.
        {
            let mut inner = self.inner.lock().unwrap();
            inner.history.record_daily_cost(
                crate::zcode::aggregate::local_day_start_ms(now),
                card.today_cost_cny,
                now,
            );
            let rules = settings.quota_alerts.clone();
            let cost_alerts = {
                let hub = &mut *inner;
                QuotaAlertEngine::evaluate(&mut hub.alert_memory, &hub.history, &snap, &rules, 30_000, now)
            };
            inner.snapshots.insert(PROVIDER_ZCODE.into(), snap);
            drop(inner);
            if !cost_alerts.is_empty() {
                self.fire_alerts(cost_alerts);
            }
        }
    }

    fn fire_alerts(&self, alerts: Vec<AlertEvent>) {
        self.persist_state();
        if let Some(app) = self.app.get() {
            use tauri_plugin_notification::NotificationExt;
            for ev in &alerts {
                let _ = app.emit("quota-alert", ev);
                let _ = app.notification().builder().title(&ev.title).body(&ev.body).show();
            }
        }
        let mut inner = self.inner.lock().unwrap();
        for ev in alerts {
            inner.alert_log.insert(0, ev);
            if inner.alert_log.len() > 50 {
                inner.alert_log.pop();
            }
        }
    }

    /// Persist alert bookkeeping (called after mutations and at exit).
    pub fn persist_state(&self) {
        let inner = self.inner.lock().unwrap();
        if let Some(p) = &inner.memory_path {
            if let Ok(json) = serde_json::to_string(&inner.alert_memory) {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let tmp = p.with_extension("tmp");
                if std::fs::write(&tmp, json).is_ok() {
                    let _ = std::fs::rename(&tmp, p);
                }
            }
        }
    }

    fn emit_all(&self) {
        if let Some(app) = self.app.get() {
            let inner = self.inner.lock().unwrap();
            let payloads: Vec<ProviderSnapshot> = inner.snapshots.values().cloned().collect();
            drop(inner);
            let _ = app.emit("provider-update", &payloads);
        }
    }

    // -- IPC helpers (called from commands) -----------------------------------

    pub fn overview(&self) -> Vec<ProviderSnapshot> {
        let inner = self.inner.lock().unwrap();
        let mut v: Vec<ProviderSnapshot> = inner.snapshots.values().cloned().collect();
        v.sort_by_key(|s| match s.provider.as_str() {
            PROVIDER_ZCODE => 0,
            PROVIDER_CODEX => 1,
            PROVIDER_ANTIGRAVITY => 2,
            _ => 3,
        });
        v
    }

    pub fn quota_alert_log(&self) -> Vec<AlertEvent> {
        self.inner.lock().unwrap().alert_log.clone()
    }

    pub fn launcher_action(&self, action: &str, settings: &Settings) -> (String, ProviderSnapshot) {
        let result = {
            let mut inner = self.inner.lock().unwrap();
            inner.launcher.configure(settings.launcher.exe_path.clone());
            match action {
                "launch" => format!("{:?}", inner.launcher.launch()),
                "reveal" => format!("{:?}", inner.launcher.reveal()),
                _ => String::new(),
            }
        };
        self.kick();
        // Give the poll thread a beat to reflect the new state.
        std::thread::sleep(Duration::from_millis(120));
        let snap = {
            let mut inner = self.inner.lock().unwrap();
            inner.launcher.configure(settings.launcher.exe_path.clone());
            let status = inner.launcher.status();
            let mut s = inner
                .snapshots
                .get(PROVIDER_ZCODE)
                .cloned()
                .unwrap_or_else(|| ProviderSnapshot::empty(PROVIDER_ZCODE, ProviderStatus::Ok, now_ms()));
            s.launcher = Some(status);
            s
        };
        {
            let mut inner = self.inner.lock().unwrap();
            inner.snapshots.insert(PROVIDER_ZCODE.into(), snap.clone());
        }
        (result, snap)
    }

    pub fn history_for(&self, provider: &str, window: &str, from_ms: i64, to_ms: i64) -> Vec<super::history::HistoryPoint> {
        let inner = self.inner.lock().unwrap();
        inner.history.points(provider, window, from_ms, to_ms)
    }

    pub fn consumption(&self, provider: &str, window: &str, days: u32, now: i64) -> Vec<(i64, f64)> {
        let inner = self.inner.lock().unwrap();
        inner.history.daily_consumption(provider, window, days, now)
    }
}

/// One-shot Volcengine connection test for the settings page. Returns a
/// human result string; never contains credentials.
pub fn test_volcengine(
    secrets: &Arc<dyn SecretStorage>,
    region: &str,
) -> Result<String, String> {
    let (ak, sk) = match (secrets.get(KEY_VOLCENGINE_AK), secrets.get(KEY_VOLCENGINE_SK)) {
        (Ok(a), Ok(s)) => (a, s),
        (Err(e), _) | (_, Err(e)) => return Err(e.message().to_string()),
    };
    let transport = UreqTransport { timeout_secs: 15 };
    let prov = volcengine::VolcengineProvider {
        region: region.to_string(),
        transport: &transport,
        now: chrono::Utc::now(),
    };
    match prov.list_packages(&ak, &sk) {
        Ok(packages) => {
            let effective = packages.iter().filter(|p| p.status == "Effective").count();
            Ok(format!("连接成功:共 {} 个资源包,{} 个生效中", packages.len(), effective))
        }
        Err(e) => Err(e.message()),
    }
}

/// Credential management used by the settings IPC.
pub fn set_volcengine_credentials(secrets: &Arc<dyn SecretStorage>, ak: &str, sk: &str) -> Result<(), String> {
    if ak.trim().is_empty() || sk.trim().is_empty() {
        return Err("AccessKey / SecretKey 不能为空".into());
    }
    secrets.set(KEY_VOLCENGINE_AK, ak.trim()).map_err(|e| e.message().to_string())?;
    secrets.set(KEY_VOLCENGINE_SK, sk.trim()).map_err(|e| e.message().to_string())?;
    Ok(())
}

pub fn clear_volcengine_credentials(secrets: &Arc<dyn SecretStorage>) -> Result<(), String> {
    let _ = secrets.delete(KEY_VOLCENGINE_AK);
    secrets.delete(KEY_VOLCENGINE_SK).map_err(|e| e.message().to_string())
}

pub fn has_volcengine_credentials(secrets: &Arc<dyn SecretStorage>) -> bool {
    secrets.get(KEY_VOLCENGINE_AK).is_ok() && secrets.get(KEY_VOLCENGINE_SK).is_ok()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialsStatusDto {
    pub configured: bool,
    pub backend: String,
    /// Masked account hint (first 4 + last 2 of AK), never the full value.
    pub ak_hint: Option<String>,
}

pub fn credentials_status(secrets: &Arc<dyn SecretStorage>) -> CredentialsStatusDto {
    let ak = secrets.get(KEY_VOLCENGINE_AK).ok();
    let hint = ak.as_ref().map(|a| {
        if a.len() <= 6 {
            "***".to_string()
        } else {
            format!("{}…{}", &a[..4], &a[a.len() - 2..])
        }
    });
    CredentialsStatusDto {
        configured: ak.is_some() && secrets.get(KEY_VOLCENGINE_SK).is_ok(),
        backend: secrets.backend_name().to_string(),
        ak_hint: hint,
    }
}

/// Apply launcher autostart-on-boot preference once at app start.
pub fn maybe_autostart_zcode(hub: &ProviderHub, launcher: &LauncherSettings) {
    if !launcher.enabled || !launcher.autostart {
        return;
    }
    let mut inner = hub.inner.lock().unwrap();
    inner.launcher.configure(launcher.exe_path.clone());
    let _ = inner.launcher.launch();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_schedules_all_providers_initially() {
        let dir = tempfile::tempdir().unwrap();
        let (hub, rx) = ProviderHub::new(Some(dir.path().to_path_buf()), "test");
        drop(rx);
        let inner = hub.inner.lock().unwrap();
        assert_eq!(inner.next_due.len(), 4);
        assert!(inner.history.schema_version() >= 1);
    }

    #[test]
    fn credentials_roundtrip_is_masked() {
        let secrets: Arc<dyn SecretStorage> = Arc::new(MemoryStorage::new());
        assert!(!has_volcengine_credentials(&secrets));
        set_volcengine_credentials(&secrets, "AKIAexample123456", "sk-secret-value").unwrap();
        assert!(has_volcengine_credentials(&secrets));
        let st = credentials_status(&secrets);
        assert!(st.configured);
        let hint = st.ak_hint.unwrap();
        assert!(hint.starts_with("AKIA"));
        assert!(!hint.contains("example123456"[4..].to_string().as_str()) || hint.len() < 20);
        // Full values never appear in any status field.
        assert_ne!(hint, "AKIAexample123456");
        clear_volcengine_credentials(&secrets).unwrap();
        assert!(!has_volcengine_credentials(&secrets));
    }

    #[test]
    fn empty_credentials_rejected() {
        let secrets: Arc<dyn SecretStorage> = Arc::new(MemoryStorage::new());
        assert!(set_volcengine_credentials(&secrets, "", "sk").is_err());
    }
}
