//! Official-API-equivalent cost estimation.
//!
//! Loads the built-in official price table (`prices_builtin.json`, compiled in
//! via `include_str!`), applies user overrides and promo expiry at runtime,
//! and converts per-record token usage into an estimated USD/CNY cost. The
//! parsing / matching / costing logic lives in pure functions so the billing
//! rules can be unit-tested without a running Tauri app. All network work
//! (the daily FX rate and the optional remote price-table pull) runs through
//! injectable fetchers and is only triggered from a background thread.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use chrono::{Datelike, TimeZone, Timelike};
use serde::{Deserialize, Serialize};

use super::aggregate::Agg;
use super::usage::UsageRecord;

/// Built-in official price table, compiled into the binary at build time.
const BUILTIN_PRICES: &str = include_str!("prices_builtin.json");

const DISCLAIMER: &str = "按官方 API 单价估算 · 非实际 Billing";
const FX_REFRESH_INTERVAL_MS: i64 = 24 * 3600 * 1000;
const FX_FETCH_URL: &str = "https://api.frankfurter.dev/v1/latest?from=USD&to=CNY";

// ---------------------------------------------------------------------------
// Built-in table types (mirror prices_builtin.json — keys are snake_case)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
pub struct PricingTable {
    pub schema: u64,
    pub updated_at: String,
    pub entries: Vec<ProviderEntry>,
    pub fx_fallback: FxFallback,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FxFallback {
    pub usd_cny: f64,
    pub date: String,
    pub source: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ProviderEntry {
    pub provider: String,
    pub display_name: String,
    pub provider_match: Vec<String>,
    pub models: Vec<PriceEntry>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PriceEntry {
    pub model: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub currency: String,
    pub pricing: Pricing,
    pub reasoning_policy: String,
    pub source_url: String,
    pub updated_at: String,
    pub promo: Option<Promo>,
    /// 计费输入口径覆写："exclusive"（input 不含缓存，Anthropic 原生）或
    /// "inclusive"（input 已含缓存读，OpenAI 兼容）。缺省走逐条数值启发式。
    #[serde(default)]
    pub input_schema: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Pricing {
    Flat(FlatPricing),
    Tiers(TierPricing),
}

#[derive(Clone, Debug, Deserialize)]
pub struct FlatPricing {
    pub input_per_m: f64,
    pub cache_hit_per_m: f64,
    pub cache_write_per_m: Option<f64>,
    pub cache_write_1h_per_m: Option<f64>,
    pub cache_storage_per_m: Option<f64>,
    pub output_per_m: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TierPricing {
    pub rule: String,
    pub peak: Tier,
    pub offpeak: Tier,
    pub cache_storage_per_m: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Tier {
    pub input_per_m: f64,
    pub cache_hit_per_m: f64,
    pub output_per_m: f64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Promo {
    pub active_until: String,
    pub note: String,
    pub list: PromoList,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PromoList {
    pub input_per_m: f64,
    pub cache_hit_per_m: f64,
    pub output_per_m: f64,
}

// ---------------------------------------------------------------------------
// Shared DTOs / value types
// ---------------------------------------------------------------------------

/// FX rate used to convert USD prices into CNY (the display currency).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FxInfo {
    pub usd_cny: f64,
    pub updated_at: String,
    pub source: String,
}

/// DeepSeek peak/off-peak window (Beijing time, fixed UTC+8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BeijingTier {
    Peak,
    Offpeak,
}

/// User override of one model's flat pricing (keyed by lowercase model id).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverrideDto {
    pub currency: String,
    pub input_per_m: f64,
    pub cache_hit_per_m: f64,
    pub cache_write_per_m: Option<f64>,
    pub cache_write_1h_per_m: Option<f64>,
    pub cache_storage_per_m: Option<f64>,
    pub output_per_m: f64,
    pub source_url: Option<String>,
    pub note: Option<String>,
}

/// Flat price actually in effect for one record (promo / override resolved).
#[derive(Clone, Debug)]
pub struct EffectiveFlat {
    pub currency: String,
    pub input_per_m: f64,
    pub cache_hit_per_m: f64,
    pub cache_write_per_m: Option<f64>,
    pub cache_write_1h_per_m: Option<f64>,
    pub cache_storage_per_m: Option<f64>,
    pub output_per_m: f64,
    pub source_url: Option<String>,
    pub note: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PromoState {
    pub active_until: String,
    pub note: String,
    pub current: bool,
}

#[derive(Clone, Debug)]
pub struct ResolvedPrice {
    pub effective: EffectiveFlat,
    pub tier: Option<BeijingTier>,
    pub overridden: bool,
    pub promo: Option<PromoState>,
    /// 计费输入口径（来自条目，覆写价格时同样沿用模型的数据语义）。
    pub schema: Option<String>,
}

// ---------------------------------------------------------------------------
// Pure helpers: matching, promo expiry, Beijing tiers, input split
// ---------------------------------------------------------------------------

/// Find the price entry for a model id. Matching is case-insensitive and
/// considers `model` and `aliases`. Returns the entry and its provider group.
pub fn find_entry<'a>(
    table: &'a PricingTable,
    model: &str,
) -> Option<(&'a PriceEntry, &'a ProviderEntry)> {
    let m = model.trim().to_lowercase();
    if m.is_empty() {
        return None;
    }
    for pe in &table.entries {
        for e in &pe.models {
            if e.model.to_lowercase() == m {
                return Some((e, pe));
            }
            if e.aliases.iter().any(|a| a.to_lowercase() == m) {
                return Some((e, pe));
            }
        }
    }
    None
}

/// Build a flat `PriceEntry` standing in for a model that has no built-in
/// entry but carries a user override. The override values become the entry's
/// pricing so all existing flat-resolution paths treat it uniformly.
pub fn synthetic_entry(model: &str, o: &OverrideDto) -> PriceEntry {
    PriceEntry {
        model: model.to_string(),
        aliases: Vec::new(),
        currency: o.currency.clone(),
        pricing: Pricing::Flat(FlatPricing {
            input_per_m: o.input_per_m,
            cache_hit_per_m: o.cache_hit_per_m,
            cache_write_per_m: o.cache_write_per_m,
            cache_write_1h_per_m: o.cache_write_1h_per_m,
            cache_storage_per_m: o.cache_storage_per_m,
            output_per_m: o.output_per_m,
        }),
        reasoning_policy: "included_in_output".to_string(),
        source_url: o.source_url.clone().unwrap_or_default(),
        updated_at: String::new(),
        promo: None,
        input_schema: None,
        notes: o.note.clone().map(|n| vec![n]).unwrap_or_default(),
    }
}

enum EntrySource<'a> {
    /// A built-in table entry (possibly overridden).
    Builtin(&'a PriceEntry),
    /// A synthetic entry built from a user override for an otherwise-unknown
    /// model.
    OverrideOnly(PriceEntry),
}

/// A model's effective pricing entry, resolved override-first: a manual
/// override makes even a model with no built-in entry priced.
pub struct ResolvedModel<'a> {
    source: EntrySource<'a>,
    provider: Option<&'a ProviderEntry>,
    /// Present when a user override is in effect on a built-in entry.
    override_: Option<&'a OverrideDto>,
}

impl<'a> ResolvedModel<'a> {
    pub fn entry(&self) -> &PriceEntry {
        match &self.source {
            EntrySource::Builtin(e) => e,
            EntrySource::OverrideOnly(e) => e,
        }
    }

    pub fn provider(&self) -> Option<&'a ProviderEntry> {
        self.provider
    }

    pub fn is_overridden(&self) -> bool {
        self.override_.is_some() || matches!(self.source, EntrySource::OverrideOnly(_))
    }
}

/// Resolve the pricing entry for a model: ① user override by lowercase model
/// name (a model with an override is always priced), then ② the built-in
/// table (`model`/`aliases`, case-insensitive). `None` ⇒ neither source knows
/// the model.
pub fn resolve_model<'a>(
    table: &'a PricingTable,
    overrides: &'a HashMap<String, OverrideDto>,
    model: &str,
) -> Option<ResolvedModel<'a>> {
    let key = model.trim().to_lowercase();
    if key.is_empty() {
        return None;
    }
    let builtin = find_entry(table, model);
    if let Some(ov) = overrides.get(&key) {
        if let Some((e, p)) = builtin {
            return Some(ResolvedModel {
                source: EntrySource::Builtin(e),
                provider: Some(p),
                override_: Some(ov),
            });
        }
        return Some(ResolvedModel {
            source: EntrySource::OverrideOnly(synthetic_entry(model, ov)),
            provider: None,
            override_: None,
        });
    }
    builtin.map(|(e, p)| ResolvedModel {
        source: EntrySource::Builtin(e),
        provider: Some(p),
        override_: None,
    })
}

pub fn promo_active_until_ms(active_until: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(active_until)
        .ok()
        .map(|d| d.timestamp_millis())
}

/// Beijing-time (fixed UTC+8) peak/off-peak classification, independent of the
/// machine timezone. Peak = Mon–Fri 09:00–12:00 or 14:00–18:00; weekends are
/// always off-peak.
pub fn beijing_tier(ts_ms: i64) -> BeijingTier {
    let Some(dt) = chrono::FixedOffset::east_opt(8 * 3600)
        .and_then(|tz| tz.timestamp_millis_opt(ts_ms).single())
    else {
        return BeijingTier::Offpeak;
    };
    let weekday = dt.weekday().number_from_monday(); // 1 = Monday .. 7 = Sunday
    let hour = dt.hour();
    let workday = weekday <= 5;
    let peak_hour = (9..12).contains(&hour) || (14..18).contains(&hour);
    if workday && peak_hour {
        BeijingTier::Peak
    } else {
        BeijingTier::Offpeak
    }
}

/// Split one record's input into (non-cached input, cache read, cache write)
/// per its schema:
/// - exclusive (Claude-style input excludes cache): input billed as-is;
/// - inclusive (OpenAI-style input already contains cached tokens):
///   non-cached = max(input − cache_read, 0).
pub fn split_input(rec: &UsageRecord) -> (u64, u64, u64) {
    let cr = rec.cache_read_tokens.unwrap_or(0);
    let cw = rec.cache_write_tokens.unwrap_or(0);
    if cr == 0 && cw == 0 {
        return (rec.input_tokens, 0, 0);
    }
    let inclusive = rec.input_tokens >= cr + cw && rec.input_tokens > 0;
    if inclusive {
        (rec.input_tokens.saturating_sub(cr), cr, cw)
    } else {
        (rec.input_tokens, cr, cw)
    }
}

/// Same split, but with an explicit schema hint from the price entry that
/// overrides the numeric heuristic (Anthropic input excludes cache).
pub fn split_input_schema(rec: &UsageRecord, schema: Option<&str>) -> (u64, u64, u64) {
    match schema {
        Some("exclusive") => {
            let cr = rec.cache_read_tokens.unwrap_or(0);
            let cw = rec.cache_write_tokens.unwrap_or(0);
            (rec.input_tokens, cr, cw)
        }
        Some("inclusive") => {
            let cr = rec.cache_read_tokens.unwrap_or(0);
            let cw = rec.cache_write_tokens.unwrap_or(0);
            (rec.input_tokens.saturating_sub(cr), cr, cw)
        }
        _ => split_input(rec),
    }
}

fn base_cache_write(e: &PriceEntry) -> Option<f64> {
    match &e.pricing {
        Pricing::Flat(f) => f.cache_write_per_m,
        Pricing::Tiers(_) => None,
    }
}

fn base_cache_write_1h(e: &PriceEntry) -> Option<f64> {
    match &e.pricing {
        Pricing::Flat(f) => f.cache_write_1h_per_m,
        Pricing::Tiers(_) => None,
    }
}

fn base_cache_storage(e: &PriceEntry) -> Option<f64> {
    match &e.pricing {
        Pricing::Flat(f) => f.cache_storage_per_m,
        Pricing::Tiers(t) => t.cache_storage_per_m,
    }
}

/// The base (non-promo) flat price of an entry, resolving tiers by timestamp.
fn flat_base(entry: &PriceEntry, ts_ms: i64) -> (EffectiveFlat, Option<BeijingTier>) {
    match &entry.pricing {
        Pricing::Flat(f) => (
            EffectiveFlat {
                currency: entry.currency.clone(),
                input_per_m: f.input_per_m,
                cache_hit_per_m: f.cache_hit_per_m,
                cache_write_per_m: f.cache_write_per_m,
                cache_write_1h_per_m: f.cache_write_1h_per_m,
                cache_storage_per_m: f.cache_storage_per_m,
                output_per_m: f.output_per_m,
                source_url: Some(entry.source_url.clone()),
                note: None,
            },
            None,
        ),
        Pricing::Tiers(t) => {
            let tier = beijing_tier(ts_ms);
            let tp = if tier == BeijingTier::Peak {
                &t.peak
            } else {
                &t.offpeak
            };
            (
                EffectiveFlat {
                    currency: entry.currency.clone(),
                    input_per_m: tp.input_per_m,
                    cache_hit_per_m: tp.cache_hit_per_m,
                    cache_write_per_m: None,
                    cache_write_1h_per_m: None,
                    cache_storage_per_m: t.cache_storage_per_m,
                    output_per_m: tp.output_per_m,
                    source_url: Some(entry.source_url.clone()),
                    note: None,
                },
                Some(tier),
            )
        }
    }
}

/// The official list price a promo falls back to once expired (`promo.list`).
fn flat_promo_list(entry: &PriceEntry, p: &Promo) -> EffectiveFlat {
    EffectiveFlat {
        currency: entry.currency.clone(),
        input_per_m: p.list.input_per_m,
        cache_hit_per_m: p.list.cache_hit_per_m,
        cache_write_per_m: base_cache_write(entry),
        cache_write_1h_per_m: base_cache_write_1h(entry),
        cache_storage_per_m: base_cache_storage(entry),
        output_per_m: p.list.output_per_m,
        source_url: Some(entry.source_url.clone()),
        note: None,
    }
}

/// Resolve the price in effect for one record: user override wins, then promo
/// (base price while active, `promo.list` after expiry), then base flat price.
pub fn resolve_price(
    entry: &PriceEntry,
    override_: Option<&OverrideDto>,
    now_ms: i64,
    ts_ms: i64,
) -> ResolvedPrice {
    if let Some(o) = override_ {
        return ResolvedPrice {
            effective: EffectiveFlat {
                currency: o.currency.clone(),
                input_per_m: o.input_per_m,
                cache_hit_per_m: o.cache_hit_per_m,
                cache_write_per_m: o.cache_write_per_m,
                cache_write_1h_per_m: o.cache_write_1h_per_m,
                cache_storage_per_m: o.cache_storage_per_m,
                output_per_m: o.output_per_m,
                source_url: o.source_url.clone(),
                note: o.note.clone(),
            },
            tier: None,
            overridden: true,
            schema: entry.input_schema.clone(),
            promo: entry.promo.as_ref().map(|p| PromoState {
                active_until: p.active_until.clone(),
                note: p.note.clone(),
                current: false,
            }),
        };
    }

    if let Some(p) = &entry.promo {
        let active = promo_active_until_ms(&p.active_until)
            .map(|t| now_ms < t)
            .unwrap_or(false);
        // While active the promo price IS the base pricing (e.g. glm-5.3-flash
        // 0.075/0.015/0.25); once expired we fall back to the list price.
        let (effective, tier) = if active {
            flat_base(entry, ts_ms)
        } else {
            (flat_promo_list(entry, p), None)
        };
        return ResolvedPrice {
            effective,
            tier,
            overridden: false,
            schema: entry.input_schema.clone(),
            promo: Some(PromoState {
                active_until: p.active_until.clone(),
                note: p.note.clone(),
                current: active,
            }),
        };
    }

    let (effective, tier) = flat_base(entry, ts_ms);
    ResolvedPrice {
        effective,
        tier,
        overridden: false,
        schema: entry.input_schema.clone(),
        promo: None,
    }
}

// ---------------------------------------------------------------------------
// Cost lines
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LineKey {
    Input,
    CacheHit,
    CacheWrite,
    CacheStorage,
    Output,
    Reasoning,
}

impl LineKey {
    pub fn key(&self) -> &'static str {
        match self {
            LineKey::Input => "input",
            LineKey::CacheHit => "cache_hit",
            LineKey::CacheWrite => "cache_write",
            LineKey::CacheStorage => "cache_storage",
            LineKey::Output => "output",
            LineKey::Reasoning => "reasoning",
        }
    }

    pub fn label(&self, tier: Option<BeijingTier>) -> String {
        let base = match self {
            LineKey::Input => "输入",
            LineKey::CacheHit => "缓存命中",
            LineKey::CacheWrite => "缓存写入",
            LineKey::CacheStorage => "缓存存储",
            LineKey::Output => "输出",
            LineKey::Reasoning => "思考",
        };
        match tier {
            Some(BeijingTier::Peak) => format!("{base}(高峰)"),
            Some(BeijingTier::Offpeak) => format!("{base}(空闲)"),
            None => base.to_string(),
        }
    }
}

fn line_cost(tokens: u64, per_m: f64, currency: &str, fx: &FxInfo) -> f64 {
    let mult = if currency == "USD" { fx.usd_cny } else { 1.0 };
    tokens as f64 / 1_000_000.0 * per_m * mult
}

/// Total CNY cost of one record under a resolved price (input + cache hit +
/// cache write + output; reasoning is display-only and never billed).
fn effective_record_cost(rec: &UsageRecord, rp: &ResolvedPrice, fx: &FxInfo) -> f64 {
    let eff = &rp.effective;
    let (inp, cr, cw) = split_input_schema(rec, rp.schema.as_deref());
    line_cost(inp, eff.input_per_m, &eff.currency, fx)
        + line_cost(cr, eff.cache_hit_per_m, &eff.currency, fx)
        + eff.cache_write_per_m
            .map(|pm| line_cost(cw, pm, &eff.currency, fx))
            .unwrap_or(0.0)
        + line_cost(rec.output_tokens, eff.output_per_m, &eff.currency, fx)
}

/// One record → (key, tier, tokens, perM, costCny) rows.
fn record_lines(
    rec: &UsageRecord,
    rp: &ResolvedPrice,
    fx: &FxInfo,
) -> Vec<(LineKey, Option<BeijingTier>, u64, Option<f64>, f64)> {
    let eff = &rp.effective;
    let (inp, cr, cw) = split_input_schema(rec, rp.schema.as_deref());
    let out = rec.output_tokens;
    let reasoning = rec.reasoning_tokens.unwrap_or(0);
    let mut lines = Vec::new();
    lines.push((
        LineKey::Input,
        rp.tier,
        inp,
        Some(eff.input_per_m),
        line_cost(inp, eff.input_per_m, &eff.currency, fx),
    ));
    lines.push((
        LineKey::CacheHit,
        rp.tier,
        cr,
        Some(eff.cache_hit_per_m),
        line_cost(cr, eff.cache_hit_per_m, &eff.currency, fx),
    ));
    if let Some(pm) = eff.cache_write_per_m {
        lines.push((
            LineKey::CacheWrite,
            rp.tier,
            cw,
            Some(pm),
            line_cost(cw, pm, &eff.currency, fx),
        ));
    }
    if eff.cache_storage_per_m.is_some() {
        // Storage is a monthly cost with no per-record token data; surfaced as
        // a zero-token line so the entry's unit is visible.
        lines.push((LineKey::CacheStorage, rp.tier, 0, eff.cache_storage_per_m, 0.0));
    }
    lines.push((
        LineKey::Output,
        rp.tier,
        out,
        Some(eff.output_per_m),
        line_cost(out, eff.output_per_m, &eff.currency, fx),
    ));
    if reasoning > 0 {
        // reasoning_policy is always "included_in_output" for built-in entries:
        // never billed again, only shown for transparency.
        lines.push((
            LineKey::Reasoning,
            rp.tier,
            reasoning,
            Some(eff.output_per_m),
            0.0,
        ));
    }
    lines
}

// ---------------------------------------------------------------------------
// IPC DTOs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CostSummaryDto {
    pub range: String,
    pub total_tokens: u64,
    pub total_cost_cny: f64,
    pub fully_priced: bool,
    pub models: Vec<ModelCost>,
    pub unknown_models: Vec<String>,
    pub fx: FxInfo,
    pub price_updated_at: String,
    pub disclaimer: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCost {
    pub name: String,
    pub cost_cny: f64,
    pub priced: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CostDetailDto {
    pub model: String,
    pub priced: bool,
    pub notes: Vec<String>,
    pub total_cny: f64,
    pub lines: Vec<CostLine>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CostLine {
    pub key: String,
    pub label: String,
    pub tokens: u64,
    pub per_m: Option<f64>,
    pub currency: String,
    pub tier: Option<BeijingTier>,
    pub cost_cny: f64,
    pub included_in: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingTableDto {
    pub entries: Vec<PriceEntryDto>,
    pub unknown_models: Vec<String>,
    pub fx: FxInfo,
    pub remote_url: Option<String>,
    pub last_refresh: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceEntryDto {
    pub provider: String,
    pub display_name: String,
    pub model: String,
    pub currency: String,
    pub input_per_m: Option<f64>,
    pub cache_hit_per_m: Option<f64>,
    pub cache_write_per_m: Option<f64>,
    pub cache_write_1h_per_m: Option<f64>,
    pub cache_storage_per_m: Option<f64>,
    pub output_per_m: Option<f64>,
    pub tiers: Option<Vec<TierDto>>,
    pub reasoning_policy: String,
    pub source_url: String,
    pub updated_at: String,
    pub promo: Option<PromoDto>,
    pub overridden: bool,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TierDto {
    pub name: String,
    pub input_per_m: f64,
    pub cache_hit_per_m: f64,
    pub output_per_m: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromoDto {
    pub active_until: String,
    pub note: String,
    pub current_is_promo: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingRefreshResultDto {
    pub ok: bool,
    pub fx_ok: bool,
    pub error: Option<String>,
    pub refreshed_at: String,
}

// ---------------------------------------------------------------------------
// Pure DTO builders (unit-testable, no Tauri state)
// ---------------------------------------------------------------------------

/// Build the pricing-table DTO for one entry with its effective (promo /
/// override resolved) prices. Tiered entries report `tiers` and null flat
/// fields unless overridden (an override is always flat).
pub fn entry_dto(
    prov: &ProviderEntry,
    entry: &PriceEntry,
    overrides: &HashMap<String, OverrideDto>,
    table: &PricingTable,
    now_ms: i64,
) -> PriceEntryDto {
    let ov = overrides.get(&entry.model.to_lowercase());
    let overridden = ov.is_some();

    let (
        currency,
        input_per_m,
        cache_hit_per_m,
        cache_write_per_m,
        cache_write_1h_per_m,
        cache_storage_per_m,
        output_per_m,
        source_url,
        promo,
        effective_updated_at,
    ) = if let Some(o) = ov {
        (
            o.currency.clone(),
            Some(o.input_per_m),
            Some(o.cache_hit_per_m),
            o.cache_write_per_m,
            o.cache_write_1h_per_m,
            o.cache_storage_per_m,
            Some(o.output_per_m),
            o.source_url.clone().unwrap_or_else(|| entry.source_url.clone()),
            entry.promo.as_ref().map(|p| PromoDto {
                active_until: p.active_until.clone(),
                note: p.note.clone(),
                current_is_promo: false,
            }),
            table.updated_at.clone(),
        )
    } else if let Some(p) = &entry.promo {
        let active = promo_active_until_ms(&p.active_until)
            .map(|t| now_ms < t)
            .unwrap_or(false);
        // Effective price: base pricing while the promo is active, the
        // official list price (`promo.list`) once it expires.
        let (input, hit, write, write_1h, storage, output) = if active {
            match &entry.pricing {
                Pricing::Flat(f) => (
                    Some(f.input_per_m),
                    Some(f.cache_hit_per_m),
                    f.cache_write_per_m,
                    f.cache_write_1h_per_m,
                    f.cache_storage_per_m,
                    Some(f.output_per_m),
                ),
                Pricing::Tiers(_) => (None, None, None, None, None, None),
            }
        } else {
            (
                Some(p.list.input_per_m),
                Some(p.list.cache_hit_per_m),
                base_cache_write(entry),
                base_cache_write_1h(entry),
                base_cache_storage(entry),
                Some(p.list.output_per_m),
            )
        };
        (
            entry.currency.clone(),
            input,
            hit,
            write,
            write_1h,
            storage,
            output,
            entry.source_url.clone(),
            Some(PromoDto {
                active_until: p.active_until.clone(),
                note: p.note.clone(),
                current_is_promo: active,
            }),
            // Fallback (expired) prices are effective as of the table update.
            if active {
                entry.updated_at.clone()
            } else {
                table.updated_at.clone()
            },
        )
    } else {
        match &entry.pricing {
            Pricing::Flat(f) => (
                entry.currency.clone(),
                Some(f.input_per_m),
                Some(f.cache_hit_per_m),
                f.cache_write_per_m,
                f.cache_write_1h_per_m,
                f.cache_storage_per_m,
                Some(f.output_per_m),
                entry.source_url.clone(),
                None,
                entry.updated_at.clone(),
            ),
            Pricing::Tiers(t) => (
                entry.currency.clone(),
                None,
                None,
                None,
                None,
                t.cache_storage_per_m,
                None,
                entry.source_url.clone(),
                None,
                entry.updated_at.clone(),
            ),
        }
    };

    let tiers = if overridden {
        None
    } else {
        match &entry.pricing {
            Pricing::Tiers(t) => Some(vec![
                TierDto {
                    name: "peak".into(),
                    input_per_m: t.peak.input_per_m,
                    cache_hit_per_m: t.peak.cache_hit_per_m,
                    output_per_m: t.peak.output_per_m,
                },
                TierDto {
                    name: "offpeak".into(),
                    input_per_m: t.offpeak.input_per_m,
                    cache_hit_per_m: t.offpeak.cache_hit_per_m,
                    output_per_m: t.offpeak.output_per_m,
                },
            ]),
            _ => None,
        }
    };

    PriceEntryDto {
        provider: prov.provider.clone(),
        display_name: prov.display_name.clone(),
        model: entry.model.clone(),
        currency,
        input_per_m,
        cache_hit_per_m,
        cache_write_per_m,
        cache_write_1h_per_m,
        cache_storage_per_m,
        output_per_m,
        tiers,
        reasoning_policy: entry.reasoning_policy.clone(),
        source_url,
        updated_at: effective_updated_at,
        promo,
        overridden,
        notes: entry.notes.clone(),
    }
}

/// `cost_summary` over a record slice (pure; see the manager wrapper).
pub fn compute_cost_summary(
    range_key: &str,
    records: &[UsageRecord],
    table: &PricingTable,
    overrides: &HashMap<String, OverrideDto>,
    fx: &FxInfo,
    now_ms: i64,
) -> CostSummaryDto {
    let mut agg = Agg::default();
    let mut by_model: HashMap<String, (f64, bool)> = HashMap::new();

    for rec in records {
        agg.add(rec);
        // Override-first: a manual override prices even unknown models.
        let resolved = resolve_model(table, overrides, &rec.model);
        let cost = resolved
            .as_ref()
            .map(|rm| {
                let rp = resolve_price(rm.entry(), rm.override_, now_ms, rec.ts_ms);
                effective_record_cost(rec, &rp, fx)
            })
            .unwrap_or(0.0);
        let slot = by_model.entry(rec.model.clone()).or_insert((0.0, false));
        slot.0 += cost;
        if resolved.is_some() {
            slot.1 = true;
        }
    }

    let mut models: Vec<ModelCost> = by_model
        .into_iter()
        .map(|(name, (cost, priced))| ModelCost { name, cost_cny: cost, priced })
        .collect();
    models.sort_by(|a, b| b.cost_cny.partial_cmp(&a.cost_cny).unwrap_or(std::cmp::Ordering::Equal));

    let mut unknown_models: Vec<String> = models
        .iter()
        .filter(|m| !m.priced)
        .map(|m| m.name.clone())
        .collect();
    unknown_models.sort();
    unknown_models.dedup();

    let total_cost_cny: f64 = models.iter().map(|m| m.cost_cny).sum();
    CostSummaryDto {
        range: range_key.to_string(),
        total_tokens: agg.total_tokens(),
        total_cost_cny,
        fully_priced: unknown_models.is_empty(),
        models,
        unknown_models,
        fx: fx.clone(),
        price_updated_at: table.updated_at.clone(),
        disclaimer: DISCLAIMER.to_string(),
    }
}

/// `cost_detail` for one model name over a record slice (pure).
pub fn compute_cost_detail(
    model: &str,
    records: &[UsageRecord],
    table: &PricingTable,
    overrides: &HashMap<String, OverrideDto>,
    fx: &FxInfo,
    now_ms: i64,
) -> CostDetailDto {
    let lower = model.trim().to_lowercase();
    let Some(rm) = resolve_model(table, overrides, model) else {
        return CostDetailDto {
            model: model.to_string(),
            priced: false,
            notes: Vec::new(),
            total_cny: 0.0,
            lines: Vec::new(),
        };
    };
    let entry = rm.entry();
    let currency = rm
        .override_
        .map(|o| o.currency.clone())
        .unwrap_or_else(|| entry.currency.clone());

    let mut acc: HashMap<(LineKey, Option<BeijingTier>), (u64, Option<f64>, f64)> = HashMap::new();
    let mut total = 0.0;
    for rec in records {
        if rec.model.trim().to_lowercase() != lower {
            continue;
        }
        let rp = resolve_price(entry, rm.override_, now_ms, rec.ts_ms);
        for (key, tier, tokens, per_m, cost) in record_lines(rec, &rp, fx) {
            let slot = acc.entry((key, tier)).or_insert((0, None, 0.0));
            slot.0 += tokens;
            if per_m.is_some() {
                slot.1 = per_m;
            }
            slot.2 += cost;
            total += cost;
        }
    }

    let mut lines: Vec<CostLine> = Vec::new();
    for key in [
        LineKey::Input,
        LineKey::CacheHit,
        LineKey::CacheWrite,
        LineKey::CacheStorage,
        LineKey::Output,
        LineKey::Reasoning,
    ] {
        for tier in [Some(BeijingTier::Peak), Some(BeijingTier::Offpeak), None] {
            if let Some((tokens, per_m, cost)) = acc.get(&(key, tier)) {
                lines.push(CostLine {
                    key: key.key().to_string(),
                    label: key.label(tier),
                    tokens: *tokens,
                    per_m: *per_m,
                    currency: currency.clone(),
                    tier,
                    cost_cny: *cost,
                    included_in: if key == LineKey::Reasoning {
                        Some("Output".to_string())
                    } else {
                        None
                    },
                });
            }
        }
    }

    let mut notes = entry.notes.clone();
    if let Some(p) = &entry.promo {
        notes.push(format!("促销价至 {}：{}", p.active_until, p.note));
    }
    if let Some(o) = rm.override_ {
        if let Some(n) = &o.note {
            notes.push(n.clone());
        }
    }

    CostDetailDto {
        model: model.to_string(),
        priced: true,
        notes,
        total_cny: total,
        lines,
    }
}

// ---------------------------------------------------------------------------
// Network fetchers (injectable for tests)
// ---------------------------------------------------------------------------

pub type FxFetchFn = Arc<dyn Fn() -> Result<FxInfo, String> + Send + Sync>;
pub type PriceFetchFn = Arc<dyn Fn(&str) -> Result<PricingTable, String> + Send + Sync>;

pub fn network_fetch_fx() -> Result<FxInfo, String> {
    let resp = ureq::get(FX_FETCH_URL)
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .map_err(|e| e.to_string())?;
    let body = resp.into_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let rate = v
        .pointer("/rates/CNY")
        .and_then(|x| x.as_f64())
        .ok_or_else(|| "rates.CNY missing in FX response".to_string())?;
    let date = v.get("date").and_then(|x| x.as_str()).unwrap_or("").to_string();
    Ok(FxInfo {
        usd_cny: rate,
        updated_at: date,
        source: "frankfurter.dev".to_string(),
    })
}

pub fn network_fetch_price_table(url: &str) -> Result<PricingTable, String> {
    let resp = ureq::get(url)
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .map_err(|e| e.to_string())?;
    let body = resp.into_string().map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Pricing manager (state + persistence + background refresh)
// ---------------------------------------------------------------------------

fn fallback_fx(table: &PricingTable) -> FxInfo {
    FxInfo {
        usd_cny: table.fx_fallback.usd_cny,
        updated_at: table.fx_fallback.date.clone(),
        source: "builtin fallback".to_string(),
    }
}

fn load_overrides(dir: Option<&PathBuf>) -> HashMap<String, OverrideDto> {
    let Some(dir) = dir else { return HashMap::new() };
    match std::fs::read_to_string(dir.join("pricing_overrides.json")) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

fn load_fx_cache(dir: Option<&PathBuf>) -> Option<FxInfo> {
    let dir = dir?;
    let text = std::fs::read_to_string(dir.join("fx_cache.json")).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub struct PricingManager {
    pub config_dir: Option<PathBuf>,
    table: RwLock<PricingTable>,
    overrides: RwLock<HashMap<String, OverrideDto>>,
    fx: RwLock<FxInfo>,
    remote_url: RwLock<Option<String>>,
    last_refresh: RwLock<Option<String>>,
    last_error: RwLock<Option<String>>,
    last_fx_check: Mutex<i64>,
    last_price_check: Mutex<i64>,
    fx_fetcher: FxFetchFn,
    price_fetcher: PriceFetchFn,
}

impl PricingManager {
    pub fn new(config_dir: Option<PathBuf>) -> Self {
        Self::with_fetchers(
            config_dir,
            Arc::new(network_fetch_fx),
            Arc::new(network_fetch_price_table),
        )
    }

    pub fn with_fetchers(
        config_dir: Option<PathBuf>,
        fx_fetcher: FxFetchFn,
        price_fetcher: PriceFetchFn,
    ) -> Self {
        let builtin: PricingTable =
            serde_json::from_str(BUILTIN_PRICES).expect("builtin price table is valid");
        let fx = load_fx_cache(config_dir.as_ref()).unwrap_or_else(|| fallback_fx(&builtin));
        let overrides = load_overrides(config_dir.as_ref());
        Self {
            config_dir,
            table: RwLock::new(builtin),
            overrides: RwLock::new(overrides),
            fx: RwLock::new(fx),
            remote_url: RwLock::new(None),
            last_refresh: RwLock::new(None),
            last_error: RwLock::new(None),
            last_fx_check: Mutex::new(0),
            last_price_check: Mutex::new(0),
            fx_fetcher,
            price_fetcher,
        }
    }

    pub fn fx(&self) -> FxInfo {
        self.fx.read().unwrap().clone()
    }

    pub fn current_table(&self) -> PricingTable {
        self.table.read().unwrap().clone()
    }

    pub fn overrides_snapshot(&self) -> HashMap<String, OverrideDto> {
        self.overrides.read().unwrap().clone()
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.read().unwrap().clone()
    }

    pub fn last_refresh(&self) -> Option<String> {
        self.last_refresh.read().unwrap().clone()
    }

    /// Model names present in the data but absent from both the built-in
    /// table and the user overrides (an override makes a model priced).
    pub fn unknown_models(&self, model_names: &[String]) -> Vec<String> {
        let table = self.table.read().unwrap();
        let overrides = self.overrides.read().unwrap();
        let mut out: Vec<String> = model_names
            .iter()
            .filter(|n| resolve_model(&table, &overrides, n).is_none())
            .cloned()
            .collect();
        out.sort();
        out.dedup();
        out
    }

    pub fn cost_summary(&self, range_key: &str, records: &[UsageRecord]) -> CostSummaryDto {
        let table = self.table.read().unwrap();
        let overrides = self.overrides.read().unwrap();
        let fx = self.fx.read().unwrap();
        compute_cost_summary(range_key, records, &table, &overrides, &fx, now_iso_ms())
    }

    pub fn cost_detail(&self, model: &str, records: &[UsageRecord]) -> CostDetailDto {
        let table = self.table.read().unwrap();
        let overrides = self.overrides.read().unwrap();
        let fx = self.fx.read().unwrap();
        compute_cost_detail(model, records, &table, &overrides, &fx, now_iso_ms())
    }

    pub fn build_table_dto(
        &self,
        unknown_models: Vec<String>,
        remote_url: Option<String>,
    ) -> PricingTableDto {
        let table = self.table.read().unwrap();
        let overrides = self.overrides.read().unwrap();
        let fx = self.fx.read().unwrap();
        let now = now_iso_ms();
        let overrides_ref: &HashMap<String, OverrideDto> = &overrides;
        let table_ref: &PricingTable = &table;
        let mut entries: Vec<PriceEntryDto> = table
            .entries
            .iter()
            .flat_map(|pe| {
                pe.models
                    .iter()
                    .map(move |e| entry_dto(pe, e, overrides_ref, table_ref, now))
            })
            .collect();
        // Override-only models (no built-in entry) are surfaced as entries too,
        // so the table matches what the cost engine actually prices.
        let builtin_keys: std::collections::HashSet<String> = table
            .entries
            .iter()
            .flat_map(|pe| pe.models.iter().map(|e| e.model.to_lowercase()))
            .collect();
        for (key, o) in overrides_ref.iter() {
            if builtin_keys.contains(key) {
                continue;
            }
            let synthetic = synthetic_entry(key, o);
            let provider = ProviderEntry {
                provider: "override".to_string(),
                display_name: key.clone(),
                provider_match: Vec::new(),
                models: vec![synthetic.clone()],
            };
            entries.push(entry_dto(&provider, &synthetic, overrides_ref, table_ref, now));
        }
        PricingTableDto {
            entries,
            unknown_models,
            fx: fx.clone(),
            remote_url,
            last_refresh: self.last_refresh.read().unwrap().clone(),
            last_error: self.last_error.read().unwrap().clone(),
        }
    }

    pub fn set_override(&self, model: &str, o: Option<OverrideDto>) {
        let key = model.trim().to_lowercase();
        {
            let mut map = self.overrides.write().unwrap();
            if let Some(o) = o {
                map.insert(key, o);
            } else {
                map.remove(&key);
            }
        }
        self.save_overrides();
    }

    fn save_overrides(&self) {
        let Some(dir) = &self.config_dir else { return };
        let _ = std::fs::create_dir_all(dir);
        let map = self.overrides.read().unwrap().clone();
        let path = dir.join("pricing_overrides.json");
        if let Ok(json) = serde_json::to_string_pretty(&map) {
            let tmp = dir.join("pricing_overrides.json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }

    fn set_fx_persist(&self, info: FxInfo) {
        *self.fx.write().unwrap() = info.clone();
        let Some(dir) = &self.config_dir else { return };
        let _ = std::fs::create_dir_all(dir);
        let path = dir.join("fx_cache.json");
        if let Ok(json) = serde_json::to_string_pretty(&info) {
            let tmp = dir.join("fx_cache.json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }

    /// Synchronous refresh (runs on a background thread in production; callers
    /// in tests inject fake fetchers so no network is touched). Always
    /// refreshes the FX rate; the remote price table is pulled only when a URL
    /// is configured. Any failure keeps local data and records the error.
    pub fn refresh(&self, remote_url: Option<&str>) -> PricingRefreshResultDto {
        let refreshed_at = now_iso();
        let mut ok = true;
        let mut fx_ok = true;
        let mut error: Option<String> = None;

        if let Some(url) = remote_url.filter(|u| !u.trim().is_empty()) {
            *self.remote_url.write().unwrap() = Some(url.to_string());
            match (self.price_fetcher)(url) {
                Ok(table) => {
                    *self.table.write().unwrap() = table;
                    *self.last_refresh.write().unwrap() = Some(now_iso());
                    *self.last_error.write().unwrap() = None;
                }
                Err(e) => {
                    ok = false;
                    error = Some(format!("pricing table fetch failed: {e}"));
                    *self.last_error.write().unwrap() = error.clone();
                }
            }
        } else {
            *self.remote_url.write().unwrap() = None;
        }

        match (self.fx_fetcher)() {
            Ok(info) => {
                self.set_fx_persist(info);
            }
            Err(e) => {
                fx_ok = false;
                ok = false;
                let msg = format!("fx refresh failed: {e}");
                if error.is_none() {
                    error = Some(msg.clone());
                }
                *self.last_error.write().unwrap() = Some(msg);
            }
        }

        PricingRefreshResultDto {
            ok,
            fx_ok,
            error,
            refreshed_at,
        }
    }

    /// Atomically claim a periodic refresh if one is due (FX always; the
    /// remote price table only when a URL is configured). Returns true when a
    /// refresh should be spawned; updates the in-memory last-check timestamps
    /// so the next due check is 24 h away.
    pub fn try_claim_refresh(&self, now: i64, remote_url: Option<&str>) -> bool {
        let mut fx = self.last_fx_check.lock().unwrap();
        let mut price = self.last_price_check.lock().unwrap();
        let fx_due = now - *fx >= FX_REFRESH_INTERVAL_MS;
        let price_due = remote_url.is_some() && now - *price >= FX_REFRESH_INTERVAL_MS;
        if fx_due {
            *fx = now;
        }
        if price_due {
            *price = now;
        }
        fx_due || price_due
    }

    /// Spawn a detached thread that performs a network refresh — never blocks
    /// the engine ingest/refresh path.
    pub fn spawn_background_refresh(self: &Arc<Self>, remote_url: Option<String>) {
        let me = self.clone();
        std::thread::Builder::new()
            .name("zup-pricing-refresh".into())
            .spawn(move || {
                let _ = me.refresh(remote_url.as_deref());
            })
            .ok();
    }
}

fn now_iso_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Wed 2026-08-26 10:00 Beijing (UTC+8) — deepseek peak window.
    const WED_10: i64 = 1_787_709_600_000;
    /// Wed 2026-08-26 12:00 Beijing (UTC+8) — deepseek off-peak.
    const WED_12: i64 = 1_787_716_800_000;
    /// Sat 2026-08-29 10:00 Beijing (UTC+8) — deepseek off-peak (weekend).
    const SAT_10: i64 = 1_787_968_800_000;
    /// Wed 2026-08-26 — before the built-in glm-5.3-flash promo expiry.
    const NOW: i64 = 1_787_709_600_000;

    fn builtin() -> PricingTable {
        serde_json::from_str(BUILTIN_PRICES).unwrap()
    }

    fn fx() -> FxInfo {
        FxInfo {
            usd_cny: 6.7203,
            updated_at: "2026-08-27".into(),
            source: "builtin fallback".into(),
        }
    }

    fn rec(ts: i64, model: &str, input: u64, output: u64, cr: Option<u64>, cw: Option<u64>) -> UsageRecord {
        UsageRecord {
            ts_ms: ts,
            model: model.into(),
            session_id: None,
            project: None,
            input_tokens: input,
            output_tokens: output,
            reasoning_tokens: None,
            cache_read_tokens: cr,
            cache_write_tokens: cw,
            source_file: "t".into(),
        }
    }

    #[test]
    fn anthropic_schema_forces_exclusive_split() {
        let t = builtin();
        let (e, _) = find_entry(&t, "claude-sonnet-5").unwrap();
        assert_eq!(e.input_schema.as_deref(), Some("exclusive"));
        // 真实库 claude-sonnet-5 聚合值：input=68,108,920 cache_read=64,609,953
        // cache_write=1,903,895 output=280,835。数值启发式会误判 inclusive
        //（68.1M ≥ 66.5M）少算约 5.7 倍；Anthropic 原生语义 input 不含缓存。
        let r = rec(
            NOW,
            "claude-sonnet-5",
            68_108_920,
            280_835,
            Some(64_609_953),
            Some(1_903_895),
        );
        let rp = resolve_price(e, None, NOW, NOW);
        assert_eq!(rp.schema.as_deref(), Some("exclusive"));
        let cost = effective_record_cost(&r, &rp, &fx());
        // (68.10892×2 + 64.609953×0.2 + 1.903895×2.5 + 0.280835×10) USD = 156.7079
        // ×6.7203 = 1053.13 CNY
        assert!((cost - 156.7079181 * 6.7203).abs() < 0.01, "cost={cost}");
    }

    #[test]
    fn builtin_json_parses() {
        let t = builtin();
        assert_eq!(t.entries.len(), 7);
        let models: Vec<&str> = t.entries.iter().flat_map(|p| p.models.iter().map(|m| m.model.as_str())).collect();
        assert!(models.contains(&"glm-5.3-flash"));
        assert!(models.contains(&"deepseek-v4-flash"));
        assert!(models.contains(&"claude-fable-5"));
    }

    #[test]
    fn promo_active_uses_promo_price() {
        let t = builtin();
        let (e, _) = find_entry(&t, "glm-5.3-flash").unwrap();
        let rp = resolve_price(e, None, NOW, NOW);
        assert!((rp.effective.input_per_m - 0.075).abs() < 1e-9);
        assert!((rp.effective.cache_hit_per_m - 0.015).abs() < 1e-9);
        assert!((rp.effective.output_per_m - 0.25).abs() < 1e-9);
        assert_eq!(rp.promo.as_ref().unwrap().current, true);
    }

    #[test]
    fn promo_expired_falls_back_to_list_price() {
        let mut t = builtin();
        for pe in &mut t.entries {
            for e in &mut pe.models {
                if e.model == "glm-5.3-flash" {
                    e.promo.as_mut().unwrap().active_until = "2026-01-01T00:00:00Z".into();
                }
            }
        }
        let (e, _) = find_entry(&t, "glm-5.3-flash").unwrap();
        let rp = resolve_price(e, None, NOW, NOW);
        assert!((rp.effective.input_per_m - 0.15).abs() < 1e-9);
        assert!((rp.effective.cache_hit_per_m - 0.03).abs() < 1e-9);
        assert!((rp.effective.output_per_m - 0.5).abs() < 1e-9);
        assert_eq!(rp.promo.as_ref().unwrap().current, false);
    }

    #[test]
    fn deepseek_tier_boundaries() {
        // Wed 10:00 Beijing = peak
        assert_eq!(beijing_tier(WED_10), BeijingTier::Peak);
        // Wed 12:00 Beijing = off-peak (peak ends at 12:00)
        assert_eq!(beijing_tier(WED_12), BeijingTier::Offpeak);
        // Sat 10:00 Beijing = off-peak (weekend)
        assert_eq!(beijing_tier(SAT_10), BeijingTier::Offpeak);
    }

    #[test]
    fn inclusive_vs_exclusive_input_split() {
        // OpenAI-style: input_tokens already contains the cached tokens.
        let openai = rec(0, "m", 900, 100, Some(800), None);
        assert_eq!(split_input(&openai), (100, 800, 0));
        // Claude-style: input excludes cache.
        let claude = rec(0, "m", 1000, 500, Some(39000), Some(5000));
        assert_eq!(split_input(&claude), (1000, 39000, 5000));
        // No cache fields → everything is billed as input.
        let plain = rec(0, "m", 10, 20, None, None);
        assert_eq!(split_input(&plain), (10, 0, 0));
    }

    #[test]
    fn reasoning_is_display_only_not_billed() {
        let t = builtin();
        let mut r = rec(NOW, "claude-fable-5", 1000, 100, Some(0), None);
        r.reasoning_tokens = Some(40);
        let dto = compute_cost_detail("claude-fable-5", &[r], &t, &HashMap::new(), &fx(), NOW);
        assert!(dto.priced);
        let out = dto.lines.iter().find(|l| l.key == "output").unwrap();
        // Only the 100 output tokens are billed (reasoning already included).
        assert!((out.cost_cny - 100.0 / 1e6 * 50.0 * 6.7203).abs() < 1e-9);
        let reason = dto.lines.iter().find(|l| l.key == "reasoning").unwrap();
        assert_eq!(reason.tokens, 40);
        assert_eq!(reason.cost_cny, 0.0);
        assert_eq!(reason.included_in.as_deref(), Some("Output"));
        // total = input + cache-hit + output only
        let expect = 1000.0 / 1e6 * 10.0 * 6.7203 + 0.0 + 100.0 / 1e6 * 50.0 * 6.7203;
        assert!((dto.total_cny - expect).abs() < 1e-9);
    }

    #[test]
    fn usd_fx_conversion_and_cny_direct() {
        let t = builtin();
        // USD: glm-5.3 input 1.4/M → CNY = 1.4 * 6.7203.
        let glm = rec(NOW, "glm-5.3", 1_000_000, 0, None, None);
        let d1 = compute_cost_detail("glm-5.3", &[glm], &t, &HashMap::new(), &fx(), NOW);
        let inp = d1.lines.iter().find(|l| l.key == "input").unwrap();
        assert_eq!(inp.currency, "USD");
        assert!((inp.cost_cny - 1.4 * 6.7203).abs() < 1e-9);
        // CNY: deepseek peak input 3.0/M stays 3.0 CNY (no FX).
        let ds = rec(WED_10, "deepseek-v4-flash", 1_000_000, 0, None, None);
        let d2 = compute_cost_detail("deepseek-v4-flash", &[ds], &t, &HashMap::new(), &fx(), WED_10);
        let inp2 = d2.lines.iter().find(|l| l.key == "input").unwrap();
        assert_eq!(inp2.currency, "CNY");
        assert_eq!(inp2.tier, Some(BeijingTier::Peak));
        assert!((inp2.cost_cny - 3.0).abs() < 1e-9);
    }

    #[test]
    fn override_set_clear_and_persist_roundtrip() {
        let dir = tempdir().unwrap();
        let pm = PricingManager::new(Some(dir.path().to_path_buf()));
        let ov = OverrideDto {
            currency: "CNY".into(),
            input_per_m: 0.5,
            cache_hit_per_m: 0.1,
            cache_write_per_m: None,
            cache_write_1h_per_m: None,
            cache_storage_per_m: None,
            output_per_m: 1.0,
            source_url: Some("https://example.com/pricing".into()),
            note: Some("自定义价".into()),
        };
        pm.set_override("glm-5.3", Some(ov));
        let table = pm.current_table();
        let (e, _) = find_entry(&table, "glm-5.3").unwrap();
        let rp = resolve_price(e, pm.overrides_snapshot().get("glm-5.3"), NOW, NOW);
        assert!(rp.overridden);
        assert!((rp.effective.input_per_m - 0.5).abs() < 1e-9);
        assert_eq!(rp.effective.currency, "CNY");
        // Persisted file exists and is reloaded by a fresh manager.
        assert!(dir.path().join("pricing_overrides.json").exists());
        let pm2 = PricingManager::new(Some(dir.path().to_path_buf()));
        assert_eq!(pm2.overrides_snapshot().get("glm-5.3").unwrap().input_per_m, 0.5);
        // Clearing removes the override and the persisted file entry.
        pm.set_override("glm-5.3", None);
        assert!(pm.overrides_snapshot().is_empty());
        let pm3 = PricingManager::new(Some(dir.path().to_path_buf()));
        assert!(pm3.overrides_snapshot().is_empty());
    }

    #[test]
    fn fx_fallback_from_builtin() {
        let pm = PricingManager::new(None);
        let f = pm.fx();
        assert!((f.usd_cny - 6.7203).abs() < 1e-9);
        assert_eq!(f.updated_at, "2026-08-27");
        assert_eq!(f.source, "builtin fallback");
    }

    #[test]
    fn cost_summary_matches_hand_calc() {
        let t = builtin();
        // glm-5.3 (USD): 1M input × 1.4 × 6.7203.
        let a = rec(NOW, "glm-5.3", 1_000_000, 0, None, None);
        // deepseek off-peak (Saturday): 2M output × 4.5 (CNY, no FX).
        let b = rec(SAT_10, "deepseek-v4-flash", 0, 2_000_000, None, None);
        // Unknown model: not priced, never guessed.
        let c = rec(NOW, "gpt-unknown", 10, 10, None, None);
        let dto = compute_cost_summary("7d", &[a, b, c], &t, &HashMap::new(), &fx(), NOW);
        assert!(!dto.fully_priced);
        assert_eq!(dto.unknown_models, vec!["gpt-unknown".to_string()]);
        let expect_a = 1_000_000.0 / 1e6 * 1.4 * 6.7203;
        let expect_b = 2_000_000.0 / 1e6 * 4.5;
        assert!((dto.total_cost_cny - (expect_a + expect_b)).abs() < 1e-9);
        let glm = dto.models.iter().find(|m| m.name == "glm-5.3").unwrap();
        assert!(glm.priced);
        assert!((glm.cost_cny - expect_a).abs() < 1e-9);
        let unk = dto.models.iter().find(|m| m.name == "gpt-unknown").unwrap();
        assert!(!unk.priced);
        assert_eq!(unk.cost_cny, 0.0);
        assert_eq!(dto.total_tokens, 1_000_000 + 2_000_000 + 20);
        assert_eq!(dto.disclaimer, DISCLAIMER);
    }

    #[test]
    fn case_insensitive_matching_same_price() {
        let t = builtin();
        assert!(find_entry(&t, "GLM-5.3").is_some());
        assert!(find_entry(&t, "glm-5.3").is_some());
        assert!(find_entry(&t, "DEEPSEEK-V4-FLASH-0731").is_some()); // alias
        let a = compute_cost_summary("all", &[rec(NOW, "GLM-5.3", 1_000_000, 0, None, None)], &t, &HashMap::new(), &fx(), NOW);
        let b = compute_cost_summary("all", &[rec(NOW, "glm-5.3", 1_000_000, 0, None, None)], &t, &HashMap::new(), &fx(), NOW);
        assert!((a.models[0].cost_cny - b.models[0].cost_cny).abs() < 1e-9);
    }

    #[test]
    fn detail_unknown_model_is_unpriced() {
        let t = builtin();
        let dto = compute_cost_detail("gpt-unknown", &[rec(NOW, "gpt-unknown", 1, 1, None, None)], &t, &HashMap::new(), &fx(), NOW);
        assert!(!dto.priced);
        assert!(dto.lines.is_empty());
        assert_eq!(dto.total_cny, 0.0);
    }

    #[test]
    fn pricing_table_dto_effective_prices() {
        let t = builtin();
        let now = NOW;
        let dto = |model: &str| {
            let mut found = None;
            for pe in &t.entries {
                for e in &pe.models {
                    let d = entry_dto(pe, e, &HashMap::new(), &t, now);
                    if d.model == model {
                        found = Some(d);
                    }
                }
            }
            found.unwrap()
        };
        let flash = dto("glm-5.3-flash");
        assert_eq!(flash.promo.as_ref().unwrap().current_is_promo, true);
        assert_eq!(flash.input_per_m, Some(0.075));
        assert_eq!(flash.output_per_m, Some(0.25));
        assert!(!flash.overridden);
        let ds = dto("deepseek-v4-flash");
        assert!(ds.input_per_m.is_none());
        assert!(ds.tiers.is_some());
        assert_eq!(ds.tiers.as_ref().unwrap().len(), 2);
        let claude = dto("claude-fable-5");
        assert_eq!(claude.reasoning_policy, "included_in_output");
        assert_eq!(claude.cache_write_per_m, Some(12.5));
    }

    #[test]
    fn override_effective_in_table_dto() {
        let t = builtin();
        let mut ov: HashMap<String, OverrideDto> = HashMap::new();
        ov.insert(
            "glm-5.3".into(),
            OverrideDto {
                currency: "CNY".into(),
                input_per_m: 9.9,
                cache_hit_per_m: 0.0,
                cache_write_per_m: None,
                cache_write_1h_per_m: None,
                cache_storage_per_m: None,
                output_per_m: 9.9,
                source_url: None,
                note: None,
            },
        );
        let mut found = None;
        for pe in &t.entries {
            for e in &pe.models {
                let d = entry_dto(pe, e, &ov, &t, NOW);
                if d.model == "glm-5.3" {
                    found = Some(d);
                }
            }
        }
        let dto = found.unwrap();
        assert!(dto.overridden);
        assert_eq!(dto.currency, "CNY");
        assert_eq!(dto.input_per_m, Some(9.9));
        assert_eq!(dto.output_per_m, Some(9.9));
    }

    #[test]
    fn refresh_uses_injected_fetchers_and_persists() {
        let dir = tempdir().unwrap();
        let fake_fx = Arc::new(|| -> Result<FxInfo, String> {
            Ok(FxInfo {
                usd_cny: 7.5,
                updated_at: "2026-08-26".into(),
                source: "fake".into(),
            })
        });
        let fake_price = Arc::new(|_url: &str| -> Result<PricingTable, String> {
            let mut t = builtin();
            t.updated_at = "2026-08-26".into();
            Ok(t)
        });
        let pm = PricingManager::with_fetchers(Some(dir.path().to_path_buf()), fake_fx, fake_price);
        let res = pm.refresh(Some("https://example.test/table.json"));
        assert!(res.ok);
        assert!(res.fx_ok);
        // FX updated in memory and persisted.
        assert!((pm.fx().usd_cny - 7.5).abs() < 1e-9);
        assert_eq!(pm.fx().source, "fake");
        assert!(dir.path().join("fx_cache.json").exists());
        // Price table replaced + last refresh stamped.
        assert_eq!(pm.current_table().updated_at, "2026-08-26");
        assert!(pm.last_refresh().is_some());
        assert!(pm.last_error().is_none());
    }

    #[test]
    fn refresh_failure_keeps_local_and_records_error() {
        let dir = tempdir().unwrap();
        let fail_fx = Arc::new(|| -> Result<FxInfo, String> { Err("network down".into()) });
        let fail_price = Arc::new(|_url: &str| -> Result<PricingTable, String> {
            Err("bad table".into())
        });
        let pm = PricingManager::with_fetchers(Some(dir.path().to_path_buf()), fail_fx, fail_price);
        let before = pm.current_table().updated_at.clone();
        let res = pm.refresh(Some("https://example.test/table.json"));
        assert!(!res.ok);
        assert!(!res.fx_ok);
        assert!(res.error.is_some());
        // Local table and fallback FX are retained; error is recorded.
        assert_eq!(pm.current_table().updated_at, before);
        assert!((pm.fx().usd_cny - 6.7203).abs() < 1e-9);
        assert_eq!(pm.fx().source, "builtin fallback");
        assert!(pm.last_error().is_some());
    }

    #[test]
    fn refresh_claim_gates_to_24h() {
        let pm = PricingManager::new(None);
        let base = 1_787_709_600_000i64; // realistic epoch (2026-08-26)
        // Startup (last check = 0) is immediately due.
        assert!(pm.try_claim_refresh(base, None));
        // Claimed just now → not due again.
        assert!(!pm.try_claim_refresh(base, None));
        // Just under 24 h later → still not due.
        assert!(!pm.try_claim_refresh(base + 23 * 3600_000, None));
        // Over 24 h later → due again.
        assert!(pm.try_claim_refresh(base + 24 * 3600_000 + 1, None));
    }

    #[test]
    fn override_prices_unknown_model_in_summary_and_detail() {
        let t = builtin();
        let mut ov: HashMap<String, OverrideDto> = HashMap::new();
        ov.insert(
            "test-future-model".into(),
            OverrideDto {
                currency: "USD".into(),
                input_per_m: 2.0,
                cache_hit_per_m: 0.5,
                cache_write_per_m: None,
                cache_write_1h_per_m: None,
                cache_storage_per_m: None,
                output_per_m: 8.0,
                source_url: Some("https://example.com/pricing".into()),
                note: Some("用户定价".into()),
            },
        );
        // Mixed-case record model must still match the lowercase override key.
        let recs = vec![rec(NOW, "Test-Future-Model", 1_000_000, 0, None, None)];
        let dto = compute_cost_summary("all", &recs, &t, &ov, &fx(), NOW);
        assert!(dto.fully_priced);
        assert!(dto.unknown_models.is_empty());
        let m = dto.models.iter().find(|m| m.name == "Test-Future-Model").unwrap();
        assert!(m.priced);
        // 1M input × 2.0 USD/M × 6.7203 = 13.4406 CNY.
        assert!((m.cost_cny - 13.4406).abs() < 1e-9);
        assert!((dto.total_cost_cny - 13.4406).abs() < 1e-9);

        let detail = compute_cost_detail("Test-Future-Model", &recs, &t, &ov, &fx(), NOW);
        assert!(detail.priced);
        assert!((detail.total_cny - 13.4406).abs() < 1e-9);
        let inp = detail.lines.iter().find(|l| l.key == "input").unwrap();
        assert_eq!(inp.currency, "USD");
        assert_eq!(inp.tokens, 1_000_000);
        assert!((inp.cost_cny - 13.4406).abs() < 1e-9);
        assert!(detail.notes.iter().any(|n| n.contains("用户定价")));
    }

    #[test]
    fn clearing_override_restores_unknown_model() {
        let t = builtin();
        let no_ov: HashMap<String, OverrideDto> = HashMap::new();
        let recs = vec![rec(NOW, "test-future-model", 1_000_000, 0, None, None)];
        let dto = compute_cost_summary("all", &recs, &t, &no_ov, &fx(), NOW);
        assert!(!dto.fully_priced);
        assert_eq!(dto.unknown_models, vec!["test-future-model".to_string()]);
        let m = dto.models.iter().find(|m| m.name == "test-future-model").unwrap();
        assert!(!m.priced);
        assert_eq!(m.cost_cny, 0.0);
        assert_eq!(dto.total_cost_cny, 0.0);
        let detail = compute_cost_detail("test-future-model", &recs, &t, &no_ov, &fx(), NOW);
        assert!(!detail.priced);
        assert!(detail.lines.is_empty());
    }

    #[test]
    fn unknown_models_excludes_overridden() {
        let dir = tempdir().unwrap();
        let pm = PricingManager::new(Some(dir.path().to_path_buf()));
        pm.set_override(
            "test-future-model",
            Some(OverrideDto {
                currency: "USD".into(),
                input_per_m: 1.0,
                cache_hit_per_m: 0.0,
                cache_write_per_m: None,
                cache_write_1h_per_m: None,
                cache_storage_per_m: None,
                output_per_m: 1.0,
                source_url: None,
                note: None,
            }),
        );
        let unknown = pm.unknown_models(&[
            "test-future-model".into(),
            "deepseek-v4-flash".into(),
            "test-unlisted-model".into(),
        ]);
        assert_eq!(unknown, vec!["test-unlisted-model".to_string()]);
    }

    #[test]
    fn override_only_entry_appears_in_table_dto() {
        let dir = tempdir().unwrap();
        let pm = PricingManager::new(Some(dir.path().to_path_buf()));
        pm.set_override(
            "test-future-model",
            Some(OverrideDto {
                currency: "CNY".into(),
                input_per_m: 3.0,
                cache_hit_per_m: 0.2,
                cache_write_per_m: None,
                cache_write_1h_per_m: None,
                cache_storage_per_m: None,
                output_per_m: 6.0,
                source_url: None,
                note: Some("手动价".into()),
            }),
        );
        let dto = pm.build_table_dto(vec!["test-unlisted-model".to_string()], None);
        let entry = dto.entries.iter().find(|e| e.model == "test-future-model").unwrap();
        assert!(entry.overridden);
        assert_eq!(entry.provider, "override");
        assert_eq!(entry.currency, "CNY");
        assert_eq!(entry.input_per_m, Some(3.0));
        assert_eq!(entry.output_per_m, Some(6.0));
        assert_eq!(entry.notes, vec!["手动价".to_string()]);
    }
}
