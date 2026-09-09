//! Dev smoke test: read the real ~/.zcode/cli/db/db.sqlite and print how many
//! sessions gain a real title / project folder from the `session` sidecar.
//!
//! Usage: cargo run --example sessions_meta_smoke

use std::collections::HashMap;

use zcode_usage_panel_lib::zcode::session_meta::{read_session_meta, SessionMetaEntry};
use zcode_usage_panel_lib::zcode::store::UsageStore;
use zcode_usage_panel_lib::zcode::usage::UsageRecord;

fn main() {
    let db = dirs::home_dir()
        .expect("home")
        .join(".zcode/cli/db/db.sqlite");
    let Some(meta) = read_session_meta(&db) else {
        println!("no session table metadata in {}", db.display());
        return;
    };
    println!("session table rows: {}", meta.len());

    // Push every meta entry through the store to see the resolved summaries.
    let mut store = UsageStore::new();
    let records: Vec<UsageRecord> = meta
        .keys()
        .map(|sid| UsageRecord {
            ts_ms: 1,
            model: "glm-5.3".into(),
            session_id: Some(sid.clone()),
            input_tokens: 1,
            output_tokens: 1,
            ..Default::default()
        })
        .collect();
    store.ingest(records);
    let owned: HashMap<String, SessionMetaEntry> = meta;
    store.apply_session_meta(owned);

    let summaries = store.session_summaries().to_vec();
    let with_title = summaries.iter().filter(|s| s.title.is_some()).count();
    let with_project = summaries.iter().filter(|s| s.project.is_some()).count();
    println!(
        "summaries: {} | with real title: {} | with project folder: {}",
        summaries.len(),
        with_title,
        with_project
    );
    for s in summaries.iter().take(6) {
        println!(
            "  {} → title={:?} project={:?} path={:?}",
            &s.id[..14.min(s.id.len())],
            s.title,
            s.project,
            s.project_path
        );
    }
}
