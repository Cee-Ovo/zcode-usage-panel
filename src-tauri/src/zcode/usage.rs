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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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
    /// Whole-request wall time (ZCode `model_usage.duration_ms`).
    /// `None` = source does not record it.
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// Time to first token, as reported by the source (never derived here).
    #[serde(default)]
    pub ttft_ms: Option<u64>,
    /// Terminal status when the source records one ("completed" /
    /// "error" / "cancelled" / "running"); `None` = unknown ⇒ treated as
    /// completed for stats that must exclude failures.
    #[serde(default)]
    pub status: Option<String>,
    /// Source-provided grand total for the request (ZCode
    /// `computed_total_tokens`), when the schema offers one. Preferred over
    /// recombining fields so totals match the harness's own caliber exactly.
    #[serde(default)]
    pub total_override: Option<u64>,
    /// `true` when the schema counts `reasoning_tokens` inside
    /// `output_tokens` (ZCode `model_usage`: computed total == input +
    /// output even for reasoning rows). Totals and speed must not add
    /// reasoning again in that case.
    #[serde(default)]
    pub reasoning_in_output: bool,
    /// Whether `input_tokens` **excludes** the cache tokens, when the
    /// source's caliber is provider-documented and therefore known.
    /// `None` = unknown ⇒ auto-detected from the numbers (the legacy ZCode
    /// paths, whose behavior must not change). Knowing the schema beats the
    /// heuristic: a genuinely exclusive source with a large fresh input
    /// (input ≥ cache sums) would otherwise be misread as inclusive and its
    /// cache tokens silently dropped from totals and hit-rate denominators.
    #[serde(default)]
    pub schema_exclusive: Option<bool>,
    /// Originating file path (for the data-source inspector).
    pub source_file: String,
}

impl UsageRecord {
    /// Display total for this request under the auto-classified schema.
    ///
    /// A source-provided total wins outright. Otherwise cache tokens are
    /// added only for exclusive schemas (Claude-style `input_tokens` excludes
    /// cache); inclusive schemas (input already contains cache_read — and
    /// cache_write, as in ZCode `model_usage`) must not add them again, or
    /// heavily cached traffic doubles. Reasoning is added only when the
    /// schema does not already nest it inside `output_tokens`. Exclusivity
    /// comes from `schema_exclusive` when the source documents it, else the
    /// numeric heuristic.
    pub fn display_total_tokens(&self) -> u64 {
        if let Some(total) = self.total_override {
            return total;
        }
        let cache_extra = match self.cache_read_tokens {
            Some(cr) => {
                let cw = self.cache_write_tokens.unwrap_or(0);
                let exclusive = self
                    .schema_exclusive
                    .unwrap_or_else(|| !(self.input_tokens >= cr + cw && self.input_tokens > 0));
                if exclusive {
                    cr + cw
                } else {
                    0
                }
            }
            None => self.cache_write_tokens.unwrap_or(0),
        };
        let reasoning = if self.reasoning_in_output {
            0
        } else {
            self.reasoning_tokens.unwrap_or(0)
        };
        self.input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(cache_extra)
            .saturating_add(reasoning)
    }

    /// Output-side tokens for speed statistics: `output + reasoning` unless
    /// the schema already counts reasoning inside output.
    pub fn generated_tokens(&self) -> u64 {
        if self.reasoning_in_output {
            self.output_tokens
        } else {
            self.output_tokens
                .saturating_add(self.reasoning_tokens.unwrap_or(0))
        }
    }

    /// Input-side cache classification for the hit-rate denominator:
    /// documented schema wins, numeric heuristic only as the fallback.
    pub fn input_is_exclusive(&self) -> bool {
        match self.schema_exclusive {
            Some(known) => known,
            None => match self.cache_read_tokens {
                Some(cr) => {
                    let cw = self.cache_write_tokens.unwrap_or(0);
                    !(self.input_tokens >= cr + cw && self.input_tokens > 0)
                }
                None => false,
            },
        }
    }
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

/// Request wall time. Probed on the usage object first, then the whole line
/// (rollout-style logs put `durationMs` next to the request envelope).
const DURATION_ALIASES: &[&[&str]] = &[
    &["duration_ms"],
    &["durationMs"],
    &["duration"],
];

/// Time to first token, only ever taken from a source-provided field.
const TTFT_ALIASES: &[&[&str]] = &[
    &["time_to_first_token_ms"],
    &["timeToFirstTokenMs"],
    &["ttft_ms"],
    &["ttftMs"],
    &["first_token_ms"],
];

/// Terminal request status ("completed"/"error"/"cancelled"/"running").
const STATUS_ALIASES: &[&[&str]] = &[&["status"]];

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

    let pick_optional = |container: &Value, aliases: &[&[&str]]| -> Option<serde_json::Value> {
        aliases
            .iter()
            .find_map(|p| at(container, p))
            .cloned()
            .or_else(|| {
                aliases
                    .iter()
                    .find_map(|p| at(line, p))
                    .cloned()
            })
    };
    let status = pick_optional(usage, STATUS_ALIASES)
        .and_then(|v| v.as_str().map(str::to_string));

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
        duration_ms: pick_u64(usage, DURATION_ALIASES),
        ttft_ms: pick_u64(usage, TTFT_ALIASES),
        status,
        total_override: None,
        // JSONL schemas (Claude-style) report reasoning as a separate
        // usage field outside output_tokens; ZCode SQLite sets this itself.
        reasoning_in_output: false,
        schema_exclusive: None,
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
    fn documented_schema_beats_the_numeric_heuristic() {
        // Claude-style exclusive record where the fresh input happens to be
        // larger than the cache sums: the heuristic would misread it as
        // inclusive and silently drop the cache from the total.
        let mut r = UsageRecord {
            ts_ms: 1,
            model: "m".into(),
            session_id: None,
            project: None,
            input_tokens: 350,
            output_tokens: 35,
            reasoning_tokens: None,
            cache_read_tokens: Some(50),
            cache_write_tokens: Some(10),
            duration_ms: None,
            ttft_ms: None,
            status: None,
            total_override: None,
            reasoning_in_output: false,
            schema_exclusive: None,
            source_file: "t".into(),
        };
        assert_eq!(r.display_total_tokens(), 350 + 35, "heuristic path unchanged");
        r.schema_exclusive = Some(true);
        assert_eq!(r.display_total_tokens(), 350 + 35 + 50 + 10);
        assert!(r.input_is_exclusive());
        // Inclusive pinned: cache never added, regardless of magnitudes.
        r.schema_exclusive = Some(false);
        r.input_tokens = 5;
        assert_eq!(r.display_total_tokens(), 5 + 35);
        assert!(!r.input_is_exclusive());
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
