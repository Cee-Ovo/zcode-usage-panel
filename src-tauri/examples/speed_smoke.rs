//! Dev smoke test: read this machine's real ZCode SQLite usage store and
//! print the derived response-speed statistics, so the TTFT / tok-s caliber
//! can be eyeballed against the raw `model_usage` table.
//!
//! Usage: cargo run --example speed_smoke

use zcode_usage_panel_lib::zcode::aggregate::compute_speed_stats;
use zcode_usage_panel_lib::zcode::sqlite::{read_new, SqliteSourceState};

fn main() {
    let Some(home) = dirs::home_dir() else { return };
    let db = home.join(".zcode/cli/db/db.sqlite");
    if !db.is_file() {
        println!("no zcode db at {}", db.display());
        return;
    }
    let mut state = SqliteSourceState::new(db);
    let records = match read_new(&mut state) {
        Ok(r) => r,
        Err(e) => {
            println!("read failed: {e:?}");
            return;
        }
    };
    println!("records: {}", records.len());
    let with_ttft = records.iter().filter(|r| r.ttft_ms.is_some()).count();
    let with_duration = records.iter().filter(|r| r.duration_ms.is_some()).count();
    let with_status = records.iter().filter(|r| r.status.is_some()).count();
    println!(
        "timing coverage: ttft {with_ttft}, duration {with_duration}, status {with_status}"
    );

    let now = zcode_usage_panel_lib::providers::now_ms();
    for (label, from) in [
        ("today", zcode_usage_panel_lib::zcode::aggregate::local_day_start_ms(now)),
        ("24h", now - 24 * 3600_000),
        ("7d", now - 7 * 24 * 3600_000),
    ] {
        let in_range: Vec<_> = records.iter().filter(|r| r.ts_ms >= from).cloned().collect();
        let s = compute_speed_stats(&in_range);
        println!(
            "{label}: requests={} completed={} ttft_samples={} ttft_avg={:?}ms p95={:?}ms | speed_samples={} tps={:?} p50_tps={:?}",
            in_range.len(),
            s.completed_requests,
            s.ttft_samples,
            s.ttft_avg_ms.map(|v| v as u64),
            s.ttft_p95_ms.map(|v| v as u64),
            s.speed_samples,
            s.speed_tps.map(|v| v as u64),
            s.speed_p50_tps.map(|v| v as u64),
        );
    }

    // Cross-check one sample against raw SQL numbers.
    if let Some(r) = records.iter().rev().find(|r| r.ttft_ms.is_some()) {
        println!(
            "latest timed record: ts={} model={} ttft={}ms duration={}ms status={:?} out={} reasoning={:?}",
            r.ts_ms,
            r.model,
            r.ttft_ms.unwrap_or(0),
            r.duration_ms.unwrap_or(0),
            r.status,
            r.output_tokens,
            r.reasoning_tokens
        );
    }
}
