//! Dev smoke test: run the real providers once against this machine's data
//! and print sanitized snapshots (no credentials ever printed).
//!
//! Usage: cargo run --example providers_smoke

use zcode_usage_panel_lib::providers::codex::CodexProvider;
use zcode_usage_panel_lib::providers::quota_alerts::QuotaAlertEngine;
use zcode_usage_panel_lib::providers::history::QuotaHistory;
use zcode_usage_panel_lib::providers::secrets::MemoryStorage;
use zcode_usage_panel_lib::providers::zlauncher::{Launcher, PlatformProcOps};
use zcode_usage_panel_lib::providers::antigravity;
use zcode_usage_panel_lib::providers::quota_alerts::{AlertMemory, QuotaAlertRules};
use zcode_usage_panel_lib::providers::now_ms;
use zcode_usage_panel_lib::providers::ProviderStatus;

fn main() {
    let now = zcode_usage_panel_lib::providers::now_ms();

    // --- Codex against the real CODEX_HOME ---
    let mut codex = CodexProvider::new(None);
    let snap = codex.poll(now);
    println!("== codex ==");
    println!("  status: {:?}", snap.status);
    println!("  account: {:?}", snap.account);
    println!("  plan: {:?}", snap.plan_name);
    for w in &snap.windows {
        println!(
            "  window {} ({}) used={:?}% reset_at={:?} minutes={:?}",
            w.key, w.label, w.used_percent, w.reset_at_ms, w.window_minutes
        );
    }
    if let Some(lu) = &snap.local_usage {
        println!(
            "  local: today={} all_time={} sessions={} models={}",
            lu.today.total_tokens,
            lu.all_time.total_tokens,
            lu.sessions,
            lu.models.iter().map(|m| m.model.as_str()).collect::<Vec<_>>().join(",")
        );
    }
    for n in &snap.notes {
        println!("  note: {n}");
    }
    if let Some(e) = &snap.error {
        println!("  error: {e}");
    }

    // --- ZCode launcher detection on this host ---
    let mut launcher: Launcher<PlatformProcOps> = Launcher::new(PlatformProcOps);
    let st = launcher.status();
    println!("== zcode launcher ==");
    println!("  state={} via={:?} path={:?} version={:?}", st.state, st.detected_via, st.exe_path, st.version);

    // --- Antigravity (expected absent on dev hosts) ---
    let install = antigravity::detect_installation(None);
    let transport = antigravity::UreqLocalTransport { timeout_secs: 2 };
    let asnap = antigravity::poll(&install, &transport, now);
    println!("== antigravity ==");
    println!("  config_dir={:?} status={:?} err={:?}", install.config_dir, asnap.status, asnap.error);

    // --- Alert engine dry run over the codex snapshot ---
    let mut memory = AlertMemory::default();
    let history = QuotaHistory::open_in_memory();
    let rules = QuotaAlertRules::default();
    let alerts = QuotaAlertEngine::evaluate(&mut memory, &history, &snap, &rules, 60_000, now);
    println!("== alerts fired: {}", alerts.len());

    // --- secrets backend probe ---
    let _ = MemoryStorage::new();
    println!("== done (statuses: ok={:?} not_installed={:?})", ProviderStatus::Ok, ProviderStatus::NotInstalled);
}
