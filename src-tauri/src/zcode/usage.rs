//! Unified usage-record model plus tolerant field extraction.
//!
//! ZCode's on-disk format is not frozen: JSONL transcripts (Claude-Code style
//! `{"type":"assistant","message":{...,"usage":{...}}}` lines), OpenAI-style
//! flat objects (`prompt_tokens` / `completion_tokens` /
//! `prompt_tokens_details.cached_tokens`), and SQLite tables all exist in the
//! wild across harness versions. Instead of hard-coding one schema we probe a
//! set of well-known aliases per logical field. Fields that cannot be found
//! stay `None` and are surfaced as "unavailable" in the UI — we never
//! fabricate numbers.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRecord {
    /// Request completion time, UTC epoch milliseconds.
    pub ts_ms: i64,
    /// Model name as reported by the source (displayed as-is).
    pub model: String,
    pub session_id: Option<String>,
    pub project: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// `None` = field not present in the source schema (unavailable).
    pub reasoning_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    /// Originating file path (for the data-source inspector).
    pub source_file: String,
}

/// Context hints derived from the file a line was read from.
pub struct LineContext {
    pub session_hint: Option<String>,
    pub project_hint: Option<String>,
    pub source_file: String,
}

const USAGE_OBJECT_PATHS: &[&[&str]] = &[
    &["usage"],
    &["message", "usage"],
    &["tokens"],
    &["token_usage"],
    &["tokenUsage"],
    &["data", "usage"],
    &["message", "tokens"],
    &["cost", "usage"],
];

const MODEL_PATHS: &[&[&str]] = &[
    &["model"],
    &["message", "model"],
    &["modelName"],
    &["model_name"],
    &["modelInfo", "name"],
    &["request", "model"],
];

const TS_PATHS: &[&[&str]] = &[
    &["timestamp"],
    &["ts"],
    &["requestTimestamp"],
    &["request_timestamp"],
    &["createdAt"],
    &["created_at"],
    &["time"],
    &["date"],
    &["message", "created_at"],
];

const SESSION_PATHS: &[&[&str]] = &[&["sessionId"], &["session_id"], &["conversationId"]];

const PROJECT_PATHS: &[&[&str]] = &[
    &["project"],
    &["projectPath"],
    &["project_path"],
    &["cwd"],
    &["workspace"],
    &["gitBranch"], // last resort: better than nothing
];

const INPUT_ALIASES: &[&[&str]] = &[
    &["input_tokens"],
    &["inputTokens"],
    &["prompt_tokens"],
    &["promptTokens"],
    &["input_token_count"],
    &["inputTokensCount"],
];

const OUTPUT_ALIASES: &[&[&str]] = &[
    &["output_tokens"],
    &["outputTokens"],
    &["completion_tokens"],
    &["completionTokens"],
    &["output_token_count"],
];

const REASONING_ALIASES: &[&[&str]] = &[
    &["reasoning_tokens"],
    &["reasoningTokens"],
    &["thinking_tokens"],
    &["reasoning_output_tokens"],
    &["output_tokens_details", "reasoning_tokens"],
    &["completion_tokens_details", "reasoning_tokens"],
];

const CACHE_READ_ALIASES: &[&[&str]] = &[
    &["cache_read_input_tokens"],
    &["cacheReadInputTokens"],
    &["cached_input_tokens"],
    &["cachedInputTokens"],
    &["cached_tokens"],
    &["cache_read"],
    &["cacheReadTokens"],
    &["prompt_tokens_details", "cached_tokens"],
    &["usage_details", "cached_tokens"],
];

const CACHE_WRITE_ALIASES: &[&[&str]] = &[
    &["cache_creation_input_tokens"],
    &["cacheCreationInputTokens"],
    &["cache_write_input_tokens"],
    &["cacheWriteInputTokens"],
    &["cache_creation"],
    &["cache_written_input_tokens"],
];

pub fn at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut cur = v;
    for seg in path {
        cur = cur.get(*seg)?;
    }
    match cur {
        Value::Null => None,
        other => Some(other),
    }
}

pub fn pick_u64(container: &Value, aliases: &[&[&str]]) -> Option<u64> {
    for path in aliases {
        if let Some(v) = at(container, path) {
            if let Some(n) = value_as_u64(v) {
                return Some(n);
            }
        }
    }
    None
}

pub fn value_as_u64(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64().or_else(|| n.as_f64().map(|f| f.max(0.0) as u64)),
        Value::String(s) => s.trim().parse::<u64>().ok(),
        _ => None,
    }
}

/// Parse a timestamp that may be epoch seconds, epoch milliseconds, an
/// ISO-8601 / RFC-3339 string, or a numeric string.
pub fn parse_ts(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .and_then(normalize_epoch),
        Value::String(s) => {
            let s = s.trim();
            if let Ok(num) = s.parse::<i64>() {
                if let Some(ms) = normalize_epoch(num) {
                    return Some(ms);
                }
            }
            parse_datetime_str(s)
        }
        _ => None,
    }
}

fn normalize_epoch(n: i64) -> Option<i64> {
    // Heuristic: > 1e12 ⇒ already milliseconds; > 1e8 ⇒ seconds.
    if n > 1_000_000_000_000 {
        Some(n)
    } else if n > 100_000_000 {
        Some(n.checked_mul(1000)?)
    } else {
        None
    }
}

fn parse_datetime_str(s: &str) -> Option<i64> {
    use chrono::DateTime;
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    // Missing timezone: assume UTC.
    if s.len() >= 19 {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(&s[..19], "%Y-%m-%dT%H:%M:%S") {
            return Some(naive.and_utc().timestamp_millis());
        }
    }
    None
}

/// Try to build a `UsageRecord` out of one parsed JSON line.
///
/// Returns `Ok(None)` for lines that carry no token data at all (user
/// messages, tool results, metadata events …) — that is a normal skip, not an
/// error.
pub fn extract_record(line: &Value, ctx: &LineContext) -> Result<Option<UsageRecord>, String> {
    // Find the usage object; fall back to the line itself for flat schemas.
    let mut usage = None;
    for path in USAGE_OBJECT_PATHS {
        if let Some(found) = at(line, path) {
            if found.is_object() {
                usage = Some(found);
                break;
            }
        }
    }
    let usage = match usage {
        Some(u) => u,
        None if has_any_token_field(line) => line,
        None => return Ok(None),
    };

    let input = pick_u64(usage, INPUT_ALIASES);
    let output = pick_u64(usage, OUTPUT_ALIASES);
    if input.is_none() && output.is_none() {
        return Ok(None);
    }

    let model = MODEL_PATHS
        .iter()
        .find_map(|p| at(line, p))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        // Some schemas nest the model inside the usage object.
        .or_else(|| usage.get("model").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .unwrap_or_else(|| "unknown".to_string());

    let ts_ms = TS_PATHS
        .iter()
        .find_map(|p| at(line, p))
        .and_then(parse_ts)
        .ok_or_else(|| "line has tokens but no parsable timestamp".to_string())?;

    let session_id = SESSION_PATHS
        .iter()
        .find_map(|p| at(line, p))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| ctx.session_hint.clone());

    let project = PROJECT_PATHS
        .iter()
        .find_map(|p| at(line, p))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| ctx.project_hint.clone());

    Ok(Some(UsageRecord {
        ts_ms,
        model,
        session_id,
        project,
        input_tokens: input.unwrap_or(0),
        output_tokens: output.unwrap_or(0),
        reasoning_tokens: pick_u64(usage, REASONING_ALIASES),
        cache_read_tokens: pick_u64(usage, CACHE_READ_ALIASES),
        cache_write_tokens: pick_u64(usage, CACHE_WRITE_ALIASES),
        source_file: ctx.source_file.clone(),
    }))
}

fn has_any_token_field(v: &Value) -> bool {
    INPUT_ALIASES.iter().any(|p| at(v, p).is_some())
        || OUTPUT_ALIASES.iter().any(|p| at(v, p).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> LineContext {
        LineContext {
            session_hint: Some("sess-1".into()),
            project_hint: Some("proj-A".into()),
            source_file: "test.jsonl".into(),
        }
    }

    #[test]
    fn claude_style_assistant_line() {
        let line: Value = serde_json::json!({
            "type": "assistant",
            "sessionId": "abc",
            "timestamp": "2026-08-27T10:00:00Z",
            "message": {
                "model": "GLM-5.3",
                "usage": {
                    "input_tokens": 1200,
                    "output_tokens": 340,
                    "cache_creation_input_tokens": 5000,
                    "cache_read_input_tokens": 40000
                }
            }
        });
        let rec = extract_record(&line, &ctx()).unwrap().unwrap();
        assert_eq!(rec.model, "GLM-5.3");
        assert_eq!(rec.input_tokens, 1200);
        assert_eq!(rec.output_tokens, 340);
        assert_eq!(rec.cache_read_tokens, Some(40000));
        assert_eq!(rec.cache_write_tokens, Some(5000));
        assert_eq!(rec.reasoning_tokens, None);
        assert_eq!(rec.session_id.as_deref(), Some("abc"));
    }

    #[test]
    fn openai_style_flat_line() {
        let line: Value = serde_json::json!({
            "ts": 1809607200,
            "model_name": "GPT-5.6",
            "prompt_tokens": 900,
            "completion_tokens": 100,
            "completion_tokens_details": { "reasoning_tokens": 60 },
            "prompt_tokens_details": { "cached_tokens": 800 }
        });
        let rec = extract_record(&line, &ctx()).unwrap().unwrap();
        assert_eq!(rec.model, "GPT-5.6");
        assert_eq!(rec.input_tokens, 900);
        assert_eq!(rec.reasoning_tokens, Some(60));
        assert_eq!(rec.cache_read_tokens, Some(800));
        assert_eq!(rec.ts_ms, 1809607200_000);
    }

    #[test]
    fn non_usage_line_is_none_not_error() {
        let line: Value = serde_json::json!({"type": "user", "message": {"content": "hi"}});
        assert!(extract_record(&line, &ctx()).unwrap().is_none());
    }

    #[test]
    fn tokens_without_timestamp_is_error() {
        let line: Value =
            serde_json::json!({"input_tokens": 5, "output_tokens": 6, "when": "oops"});
        assert!(extract_record(&line, &ctx()).is_err());
    }

    #[test]
    fn timestamp_normalization() {
        assert_eq!(parse_ts(&serde_json::json!(1756300800)), Some(1756300800_000));
        assert_eq!(parse_ts(&serde_json::json!(1756300800123i64)), Some(1756300800123));
        assert_eq!(parse_ts(&serde_json::json!(5)), None);
        assert_eq!(
            parse_ts(&serde_json::json!("2026-08-27T10:00:00Z")),
            Some(chrono::DateTime::parse_from_rfc3339("2026-08-27T10:00:00Z")
                .unwrap()
                .timestamp_millis())
        );
    }
}
