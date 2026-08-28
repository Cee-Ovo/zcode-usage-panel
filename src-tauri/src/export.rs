//! Data export: scopes → CSV / JSON.
//!
//! Exports are ALWAYS written to a user-chosen location via the native save
//! dialog (default: the suggested filename). Nothing is exported into the
//! app's own config/cache directories, so uninstalling the app can never
//! delete user exports.
//!
//! Scopes:
//! - `range`  : one row per bucket/day for the requested range
//! - `models` : one row per model (whole history)
//! - `sessions`: one row per session
//! - `raw`    : one row per usage record (can be large!)

use serde::Serialize;

use crate::engine::{now_ms, Engine};
use crate::settings::Settings;
use crate::zcode::aggregate::{resolve_span, TrendRange};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportData {
    pub scope: String,
    pub generated_at_ms: i64,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
}

pub fn build_export(
    engine: &Engine,
    _settings: &Settings,
    scope: &str,
    range_key: &str,
) -> Result<ExportData, String> {
    let now = now_ms();
    let mut inner = engine.inner.lock().unwrap();
    if inner.store.is_empty() {
        return Err("没有可用数据(数据目录未找到或尚未完成首次读取)".into());
    }

    match scope {
        "range" => {
            let range = TrendRange::from_key(range_key).unwrap_or(TrendRange::TodayHourly);
            let (from, to, n) = resolve_span(range, now, inner.store.history_start_ms());
            let buckets = crate::zcode::aggregate::bucketize(
                inner.store.range(from, to),
                from,
                to,
                n,
            );
            let columns = vec![
                "bucketStart".into(),
                "bucketEnd".into(),
                "requests".into(),
                "inputTokens".into(),
                "outputTokens".into(),
                "reasoningTokens".into(),
                "cacheReadTokens".into(),
                "cacheWriteTokens".into(),
                "totalTokens".into(),
                "cacheHitRate".into(),
            ];
            let rows = buckets
                .iter()
                .map(|b| {
                    vec![
                        b.start_ms.into(),
                        b.end_ms.into(),
                        b.agg.requests.into(),
                        b.agg.input.into(),
                        b.agg.output.into(),
                        b.agg.reasoning.sum.into(),
                        b.agg.cache_read.sum.into(),
                        b.agg.cache_write.sum.into(),
                        b.agg.total_tokens().into(),
                        b.agg
                            .cache_hit_rate()
                            .map(|r| serde_json::json!(format!("{:.4}", r)))
                            .unwrap_or(serde_json::json!("unavailable")),
                    ]
                })
                .collect();
            Ok(ExportData {
                scope: scope.into(),
                generated_at_ms: now,
                columns,
                rows,
            })
        }
        "models" => {
            let models = crate::zcode::aggregate::group_by_model(inner.store.all());
            let columns = vec![
                "model".into(),
                "requests".into(),
                "inputTokens".into(),
                "outputTokens".into(),
                "reasoningTokens".into(),
                "cacheReadTokens".into(),
                "cacheWriteTokens".into(),
                "totalTokens".into(),
                "cacheHitRate".into(),
                "firstUsedMs".into(),
                "lastUsedMs".into(),
            ];
            let rows = models
                .iter()
                .map(|m| {
                    vec![
                        serde_json::json!(m.name),
                        m.agg.requests.into(),
                        m.agg.input.into(),
                        m.agg.output.into(),
                        m.agg.reasoning.sum.into(),
                        m.agg.cache_read.sum.into(),
                        m.agg.cache_write.sum.into(),
                        m.agg.total_tokens().into(),
                        m.agg
                            .cache_hit_rate()
                            .map(|r| serde_json::json!(format!("{:.4}", r)))
                            .unwrap_or(serde_json::json!("unavailable")),
                        m.agg.first_ts_ms.into(),
                        m.agg.last_ts_ms.into(),
                    ]
                })
                .collect();
            Ok(ExportData {
                scope: scope.into(),
                generated_at_ms: now,
                columns,
                rows,
            })
        }
        "sessions" => {
            let sessions = inner.store.session_summaries().to_vec();
            let columns = vec![
                "sessionId".into(),
                "project".into(),
                "models".into(),
                "requests".into(),
                "inputTokens".into(),
                "outputTokens".into(),
                "reasoningTokens".into(),
                "cacheReadTokens".into(),
                "totalTokens".into(),
                "cacheHitRate".into(),
                "startMs".into(),
                "lastActivityMs".into(),
            ];
            let rows = sessions
                .iter()
                .map(|s| {
                    vec![
                        serde_json::json!(s.id),
                        serde_json::json!(s.project.clone().unwrap_or_default()),
                        serde_json::json!(s.models.join(", ")),
                        s.agg.requests.into(),
                        s.agg.input.into(),
                        s.agg.output.into(),
                        s.agg.reasoning.sum.into(),
                        s.agg.cache_read.sum.into(),
                        s.agg.total_tokens().into(),
                        s.agg
                            .cache_hit_rate()
                            .map(|r| serde_json::json!(format!("{:.4}", r)))
                            .unwrap_or(serde_json::json!("unavailable")),
                        s.agg.first_ts_ms.into(),
                        s.agg.last_ts_ms.into(),
                    ]
                })
                .collect();
            Ok(ExportData {
                scope: scope.into(),
                generated_at_ms: now,
                columns,
                rows,
            })
        }
        "raw" => {
            let columns = vec![
                "timestampMs".into(),
                "model".into(),
                "sessionId".into(),
                "project".into(),
                "inputTokens".into(),
                "outputTokens".into(),
                "reasoningTokens".into(),
                "cacheReadTokens".into(),
                "cacheWriteTokens".into(),
                "sourceFile".into(),
            ];
            let rows = inner
                .store
                .all()
                .iter()
                .map(|r| {
                    vec![
                        r.ts_ms.into(),
                        serde_json::json!(r.model),
                        serde_json::json!(r.session_id.clone().unwrap_or_default()),
                        serde_json::json!(r.project.clone().unwrap_or_default()),
                        r.input_tokens.into(),
                        r.output_tokens.into(),
                        serde_json::json!(r.reasoning_tokens),
                        serde_json::json!(r.cache_read_tokens),
                        serde_json::json!(r.cache_write_tokens),
                        serde_json::json!(r.source_file),
                    ]
                })
                .collect();
            Ok(ExportData {
                scope: scope.into(),
                generated_at_ms: now,
                columns,
                rows,
            })
        }
        other => Err(format!("unknown export scope: {other}")),
    }
}

/// Render to the requested file format: `(content, extension, filter name)`.
pub fn render(data: &ExportData, format: &str) -> Result<(String, String, String), String> {
    match format {
        "json" => {
            let json = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
            Ok((json, "json".into(), "JSON 文件".into()))
        }
        "csv" => {
            let mut out = String::new();
            // BOM keeps Excel happy with UTF-8.
            out.push('\u{FEFF}');
            let header = data
                .columns
                .iter()
                .map(|c| csv_escape(c))
                .collect::<Vec<_>>()
                .join(",");
            out.push_str(&header);
            out.push('\n');
            for row in &data.rows {
                let line = row
                    .iter()
                    .map(|cell| match cell {
                        serde_json::Value::String(s) => csv_escape(s),
                        other => csv_escape(&other.to_string()),
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                out.push_str(&line);
                out.push('\n');
            }
            Ok((out, "csv".into(), "CSV 文件".into()))
        }
        other => Err(format!("unknown export format: {other}")),
    }
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}
