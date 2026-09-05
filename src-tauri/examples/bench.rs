//! Data-layer performance benchmark with synthetic data.
//!
//! Run (any host):
//!   npm run build          # frontend dist must exist for the lib to compile
//!   cargo run --release --example bench
//!
//! What it measures — the paths the long-running app actually exercises:
//! 1. bulk ingest of 1,000,000 synthetic records (10,000 sessions),
//!    fed in 20 batches the way incremental refreshes would,
//! 2. an out-of-order late batch (worst case: triggers re-sort),
//! 3. "today" range aggregation,
//! 4. 30-day trend bucketization (90 buckets) over the full store,
//! 5. per-model grouping over the full store,
//! 6. session summary rebuild,
//! 7. resident memory (RSS) at the end.
//!
//! NOTE: synthetic records exist only to size the engine. They are never
//! shown as, or compared with, real ZCode statistics.

use std::time::Instant;
use zcode_usage_panel_lib::zcode::aggregate::{bucketize, group_by_model, resolve_span, TrendRange};
use zcode_usage_panel_lib::zcode::store::UsageStore;
use zcode_usage_panel_lib::zcode::usage::UsageRecord;

const TOTAL: usize = 1_000_000;
const SESSIONS: usize = 10_000;
const BATCHES: usize = 20;

fn synth(start_ts: i64, i: usize, models: &[&str]) -> UsageRecord {
    let session = format!("session-{:08}", i % SESSIONS);
    let model = models[i % models.len()];
    let ts = start_ts + (i as i64) * 60_000 / 12; // 5 requests/minute
    UsageRecord {
        ts_ms: ts,
        model: model.to_string(),
        session_id: Some(session),
        project: Some(format!("proj-{}", i % 50)),
        input_tokens: 500 + (i % 900) as u64,
        output_tokens: 200 + (i % 700) as u64,
        reasoning_tokens: Some((i % 300) as u64),
        cache_read_tokens: Some(20_000 + (i % 50_000) as u64),
        cache_write_tokens: Some(2_000 + (i % 8_000) as u64),
        source_file: "bench".into(),
    }
}

fn rss_mb() -> Option<f64> {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct Counters {
            cb: u32,
            page_fault_count: u32,
            peak_working_set: usize,
            working_set: usize,
            quota_peak_paged_pool: usize,
            quota_paged_pool: usize,
            quota_peak_non_paged_pool: usize,
            quota_non_paged_pool: usize,
            pagefile: usize,
            peak_pagefile: usize,
        }
        #[link(name = "psapi")]
        extern "system" {
            fn GetProcessMemoryInfo(process: *mut std::ffi::c_void, counters: *mut Counters, size: u32) -> i32;
        }
        let mut counters = Counters { cb: std::mem::size_of::<Counters>() as u32, ..Default::default() };
        // Windows' current-process pseudo-handle; no handle ownership/close.
        let ok = unsafe { GetProcessMemoryInfo(-1isize as *mut _, &mut counters, std::mem::size_of::<Counters>() as u32) };
        (ok != 0).then_some(counters.working_set as f64 / (1024.0 * 1024.0))
    }
    #[cfg(not(windows))]
    {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("VmRSS:"))
                    .and_then(|l| l.split_whitespace().nth(1).and_then(|kb| kb.parse::<f64>().ok()))
                    .map(|kb| kb / 1024.0)
            })
    }
}

fn main() {
    let models = ["GLM-5.3", "GLM-5.3-Flash", "GPT-5.6-Sol", "GLM-5.3V"];
    let start = 1_756_000_000_000i64;
    println!("=== ZCode Usage Panel data-layer benchmark ===");
    println!(
        "synthetic records: {TOTAL} | sessions: {SESSIONS} | span: ~{} days",
        TOTAL as f64 * 5_000.0 / 60.0 / 60.0 / 24.0 / 1_000.0
    );

    let mut store = UsageStore::new();

    // 1. streamed ingest
    let t0 = Instant::now();
    let per_batch = TOTAL / BATCHES;
    for b in 0..BATCHES {
        let batch: Vec<UsageRecord> = (b * per_batch..(b + 1) * per_batch)
            .map(|i| synth(start, i, &models))
            .collect();
        store.ingest(batch);
    }
    println!("1. bulk ingest ({BATCHES} batches): {:>8.1} ms", t0.elapsed().as_secs_f64() * 1000.0);

    // 2. out-of-order late batch
    let t1 = Instant::now();
    let late: Vec<UsageRecord> = (0..1000).map(|i| synth(start + 5000, i, &models)).collect();
    store.ingest(late);
    println!("2. out-of-order batch (1k records, re-sort): {:>8.1} ms", t1.elapsed().as_secs_f64() * 1000.0);

    // 3. today range
    let now = start + TOTAL as i64 * 5_000;
    let today_from = zcode_usage_panel_lib::zcode::aggregate::local_day_start_ms(now);
    let t2 = Instant::now();
    let slice = store.range(today_from, now);
    let count = slice.len();
    println!(
        "3. today slice ({count} records): {:>8.2} ms",
        t2.elapsed().as_secs_f64() * 1000.0
    );

    // 4. 30-day trend
    let (from, to, n) = resolve_span(TrendRange::Last30d, now, store.history_start_ms());
    let t3 = Instant::now();
    let buckets = bucketize(store.range(from, to), from, to, n);
    println!(
        "4. 30d bucketize ({} buckets): {:>8.1} ms",
        buckets.len(),
        t3.elapsed().as_secs_f64() * 1000.0
    );

    // 5. model grouping (full store)
    let t4 = Instant::now();
    let groups = group_by_model(store.all());
    println!(
        "5. group_by_model (full store, {} models): {:>8.1} ms",
        groups.len(),
        t4.elapsed().as_secs_f64() * 1000.0
    );

    // Direct live-card lookup must not rebuild the full sessions cache.
    let direct_start = Instant::now();
    let active = store.active_session_id().and_then(|id| store.session_summary(&id));
    assert!(active.is_some());
    println!("6a. direct active-session lookup: {:>8.3} ms", direct_start.elapsed().as_secs_f64() * 1000.0);

    // 6. session summaries
    let t5 = Instant::now();
    let sessions = store.session_summaries().len();
    println!(
        "6. session summaries ({sessions} sessions): {:>8.1} ms",
        t5.elapsed().as_secs_f64() * 1000.0
    );

    match rss_mb() {
        Some(mb) => println!("7. resident memory (RSS): {:>8.1} MB", mb),
        None => println!("7. resident memory (RSS): unavailable"),
    }
    println!("=== done ===");
}
