//! Volcengine (火山引擎) token-package provider.
//!
//! Queries the official Billing Center OpenAPI `ListResourcePackages`
//! (POST https://open.volcengineapi.com/?Action=ListResourcePackages&Version=2022-01-01,
//! service `billing`, region `cn-beijing`) with an IAM AccessKey/SecretKey
//! signed via Volcengine's HMAC-SHA256 (AWS SigV4-style) scheme. The signing
//! is implemented locally over sha2/hmac — no SDK, no extra supply chain.
//!
//! Secrets live in the OS keyring (see secrets.rs) and never appear in
//! errors, logs, or persisted files. Errors carry fixed, sanitized messages.

use std::collections::BTreeMap;

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{PackageInfo, ProviderSnapshot, ProviderStatus};

pub const HOST: &str = "open.volcengineapi.com";
pub const ACTION: &str = "ListResourcePackages";
pub const API_VERSION: &str = "2022-01-01";
pub const SERVICE: &str = "billing";
pub const DEFAULT_REGION: &str = "cn-beijing";
const MAX_PAGES: usize = 10; // 20/page → 200 packages is far beyond any real account

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, PartialEq)]
pub enum VolcError {
    /// Credentials missing (not yet configured).
    NotConfigured,
    /// Keyring unavailable — fixed message, no secret material inside.
    Keyring(String),
    Network(String),
    Http(u16, String),
    Parse(String),
    /// Volcengine returned an error envelope (Code/Message).
    Api(String, String),
}

impl VolcError {
    /// User-facing, credential-free description.
    pub fn message(&self) -> String {
        match self {
            VolcError::NotConfigured => "未配置火山引擎 AccessKey/SecretKey".into(),
            VolcError::Keyring(why) => format!("凭据存储不可用:{why}"),
            VolcError::Network(why) => {
                // ureq transport errors can embed the URL — strip any query.
                let base = why.split('?').next().unwrap_or("").to_string();
                format!("网络请求失败:{base}")
            }
            VolcError::Http(code, why) => format!("HTTP {code}:{why}"),
            VolcError::Parse(why) => format!("响应解析失败:{why}"),
            VolcError::Api(code, msg) => format!("火山引擎返回错误 {code}:{msg}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Signing (pure, unit-tested)
// ---------------------------------------------------------------------------

pub struct SignedRequest {
    pub url: String,
    pub x_date: String,
    pub authorization: String,
    pub x_content_sha256: String,
    pub body: String,
}

/// Build the signed request for one API call. `query` must be the full
/// canonical query string (sorted) without the leading '?'.
#[allow(clippy::too_many_arguments)]
pub fn sign(
    ak: &str,
    sk: &str,
    now_utc: &chrono::DateTime<chrono::Utc>,
    region: &str,
    method: &str,
    canonical_uri: &str,
    query_pairs: &BTreeMap<String, String>,
    body: &str,
) -> SignedRequest {
    let x_date = now_utc.format("%Y%m%dT%H%M%SZ").to_string();
    let date = &x_date[..8];
    let body_hash = hex(&Sha256::digest(body.as_bytes()));

    let canonical_query: String = query_pairs
        .iter()
        .map(|(k, v)| format!("{}={}", uri_encode(k), uri_encode(v)))
        .collect::<Vec<_>>()
        .join("&");

    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    headers.insert("content-type".into(), "application/json".into());
    headers.insert("host".into(), HOST.into());
    headers.insert("x-content-sha256".into(), body_hash.clone());
    headers.insert("x-date".into(), x_date.clone());
    let signed_headers = headers.keys().cloned().collect::<Vec<_>>().join(";");
    let canonical_headers: String = headers
        .iter()
        .map(|(k, v)| format!("{k}:{v}\n"))
        .collect::<String>();

    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{body_hash}"
    );

    let scope = format!("{date}/{region}/{SERVICE}/request");
    let string_to_sign = format!(
        "HMAC-SHA256\n{x_date}\n{scope}\n{}",
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );

    let k_date = hmac_hex(sk.as_bytes(), date.as_bytes());
    let k_region = hmac_hex(k_date.as_bytes(), region.as_bytes());
    let k_service = hmac_hex(k_region.as_bytes(), SERVICE.as_bytes());
    let k_signing = hmac_hex(k_service.as_bytes(), b"request");
    let signature = hex(&hmac_bytes(k_signing.as_bytes(), string_to_sign.as_bytes()));

    SignedRequest {
        url: format!("https://{HOST}/?{canonical_query}"),
        x_date,
        authorization: format!(
            "HMAC-SHA256 Credential={ak}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
        ),
        x_content_sha256: body_hash,
        body: body.to_string(),
    }
}

fn hmac_bytes(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

fn hmac_hex(key: &[u8], data: &[u8]) -> String {
    hex(&hmac_bytes(key, data))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Volcengine-style URI encoding: RFC3986 unreserved minus '/'.
fn uri_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Response parsing (pure, unit-tested)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ApiPackage {
    #[serde(default, rename = "InstanceNo")]
    instance_no: String,
    #[serde(default, rename = "InstanceName")]
    instance_name: String,
    #[serde(default, rename = "ConfigurationCode")]
    configuration_code: String,
    #[serde(default, rename = "ConfigurationName")]
    configuration_name: String,
    #[serde(default, rename = "Product")]
    product: String,
    #[serde(default, rename = "ProductName")]
    product_name: String,
    #[serde(default, rename = "TotalAmount")]
    total_amount: String,
    #[serde(default, rename = "AvailableAmount")]
    available_amount: String,
    #[serde(default, rename = "Unit")]
    unit: String,
    #[serde(default, rename = "EffectiveTime")]
    effective_time: String,
    #[serde(default, rename = "ExpiryTime")]
    expiry_time: String,
    #[serde(default, rename = "Status")]
    status: String,
}

fn as_f64(v: &str) -> f64 {
    v.trim().parse().unwrap_or(0.0)
}

fn as_ms(v: &str) -> Option<i64> {
    let t: i64 = v.trim().parse().ok()?;
    (t > 0).then_some(t * 1000)
}

/// "千Token" → 1000, "万Token" → 10000, "Token"/others → 1.
pub fn unit_multiplier(unit: &str) -> f64 {
    if unit.to_lowercase().contains("million") || unit.contains("百万") {
        1_000_000.0
    } else if unit.contains('万') {
        10_000.0
    } else if unit.contains('千') || unit.eq_ignore_ascii_case("k") {
        1_000.0
    } else {
        1.0
    }
}

pub fn parse_packages(body: &str) -> Result<Vec<PackageInfo>, VolcError> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| VolcError::Parse(e.to_string()))?;
    if let Some(err) = v.get("Error") {
        // e.g. {"Error":{"Code":"MissingParameter.XXX","Message":"..."}}
        let code = err.get("Code").and_then(|c| c.as_str()).unwrap_or("Unknown");
        let msg = err.get("Message").and_then(|m| m.as_str()).unwrap_or("");
        return Err(VolcError::Api(code.into(), msg.into()));
    }
    // Some error envelopes put ResponseMetadata.Error.
    if let Some(err) = v.pointer("/ResponseMetadata/Error") {
        let code = err.get("Code").and_then(|c| c.as_str()).unwrap_or("Unknown");
        let msg = err.get("Message").and_then(|m| m.as_str()).unwrap_or("");
        return Err(VolcError::Api(code.into(), msg.into()));
    }
    let list = v
        .pointer("/Result/List")
        .and_then(|l| l.as_array())
        .ok_or_else(|| VolcError::Parse("缺少 Result.List".into()))?;
    let mut out = Vec::with_capacity(list.len());
    for item in list {
        let p: ApiPackage =
            serde_json::from_value(item.clone()).map_err(|e| VolcError::Parse(e.to_string()))?;
        let total = as_f64(&p.total_amount);
        let available = as_f64(&p.available_amount);
        let used = (total - available).max(0.0);
        out.push(PackageInfo {
            instance_no: p.instance_no.clone(),
            name: if p.instance_name.is_empty() { p.configuration_name.clone() } else { p.instance_name.clone() },
            configuration: if p.configuration_name.is_empty() { p.configuration_code.clone() } else { p.configuration_name.clone() },
            product: if p.product_name.is_empty() { p.product.clone() } else { p.product_name.clone() },
            total_amount: total,
            available_amount: available,
            used_amount: used,
            unit: p.unit.clone(),
            unit_multiplier: unit_multiplier(&p.unit),
            effective_ms: as_ms(&p.effective_time),
            expiry_ms: as_ms(&p.expiry_time),
            status: p.status.clone(),
            usage_percent: (total > 0.0).then_some(used / total * 100.0),
        });
    }
    Ok(out)
}

fn next_token(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let t = v.pointer("/Result/NextToken")?.as_str()?.trim().to_string();
    (!t.is_empty()).then_some(t)
}

// ---------------------------------------------------------------------------
// Transport + provider poll
// ---------------------------------------------------------------------------

/// Injectable HTTP transport so tests never touch the network.
pub trait HttpTransport: Send + Sync {
    /// POST the signed request; returns the response body.
    fn post_json(&self, req: &SignedRequest) -> Result<String, VolcError>;
}

pub struct UreqTransport {
    pub timeout_secs: u64,
}

impl HttpTransport for UreqTransport {
    fn post_json(&self, req: &SignedRequest) -> Result<String, VolcError> {
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(self.timeout_secs.max(5)))
            .build();
        let resp = agent
            .post(&req.url)
            .set("Content-Type", "application/json")
            .set("X-Date", &req.x_date)
            .set("X-Content-Sha256", &req.x_content_sha256)
            .set("Authorization", &req.authorization)
            .send_string(&req.body)
            .map_err(|e| VolcError::Network(e.to_string()))?;
        let status = resp.status();
        let body = resp
            .into_string()
            .map_err(|e| VolcError::Network(e.to_string()))?;
        if status >= 400 {
            // Surface the API's own error code when present.
            let why = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| {
                    v.pointer("/ResponseMetadata/Error")
                        .or_else(|| v.get("Error"))
                        .and_then(|e| e.get("Message").or(Some(e)))
                        .and_then(|m| m.as_str().map(|s| s.to_string()))
                })
                .unwrap_or_else(|| "请求被拒绝".into());
            return Err(VolcError::Http(status, why));
        }
        Ok(body)
    }
}

pub struct VolcengineProvider<'a> {
    pub region: String,
    pub transport: &'a dyn HttpTransport,
    pub now: chrono::DateTime<chrono::Utc>,
}

impl<'a> VolcengineProvider<'a> {
    /// Fetch all pages of effective+used-up token packages.
    pub fn list_packages(&self, ak: &str, sk: &str) -> Result<Vec<PackageInfo>, VolcError> {
        let mut packages = Vec::new();
        let mut token = String::new();
        for _ in 0..MAX_PAGES {
            let body = if token.is_empty() {
                r#"{"ResourceType":"Package","MaxResults":"20"}"#.to_string()
            } else {
                format!(
                    r#"{{"ResourceType":"Package","MaxResults":"20","NextToken":{}}}"#,
                    serde_json::to_string(&token).unwrap_or_default()
                )
            };
            let mut query = BTreeMap::new();
            query.insert("Action".into(), ACTION.into());
            query.insert("Version".into(), API_VERSION.into());
            let req = sign(ak, sk, &self.now, &self.region, "POST", "/", &query, &body);
            let resp = self.transport.post_json(&req)?;
            packages.extend(parse_packages(&resp)?);
            match next_token(&resp) {
                Some(t) => token = t,
                None => break,
            }
        }
        Ok(packages)
    }
}

/// Build the provider snapshot from a package list (pure — reused by tests
/// and the hub).
pub fn build_snapshot(packages: &[PackageInfo], now_ms: i64, err: Option<&VolcError>) -> ProviderSnapshot {
    let mut snap = ProviderSnapshot::empty(super::PROVIDER_VOLCENGINE, ProviderStatus::Ok, now_ms);
    snap.source = "火山引擎费用中心 OpenAPI ListResourcePackages(官方接口)".into();
    snap.source_url = Some("https://www.volcengine.com/docs/6269/1337079".into());
    if let Some(e) = err {
        snap.status = match e {
            VolcError::NotConfigured => ProviderStatus::NotConfigured,
            _ => ProviderStatus::Error,
        };
        snap.error = Some(e.message());
        return snap;
    }
    snap.packages = packages.to_vec();
    if packages.is_empty() {
        snap.notes.push("当前账号没有查询到资源包(或均已过期)".into());
    }
    let effective: Vec<&PackageInfo> = packages
        .iter()
        .filter(|p| p.status == "Effective")
        .collect();
    if let Some(w) = super::history::aggregate_packages(packages) {
        snap.windows.push(w);
    }
    // Nearest-expiry note (≤ 30 d) for the alert engine + UI.
    if let Some(nearest) = effective.iter().filter_map(|p| p.expiry_ms).min() {
        let days = (nearest - now_ms) / 86_400_000;
        if days <= 30 {
            snap.notes.push(format!("最近到期的资源包还有 {days} 天到期"));
        }
    }
    snap
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(2026, 8, 30, 12, 0, 0).unwrap()
    }

    #[test]
    fn signature_matches_reference_vector() {
        // Cross-checked against Volcengine's documented signing steps:
        // deterministic for the fixed inputs below.
        let query = BTreeMap::from([
            ("Action".to_string(), ACTION.to_string()),
            ("Version".to_string(), API_VERSION.to_string()),
        ]);
        let body = r#"{"ResourceType":"Package","MaxResults":"20"}"#;
        let req = sign("AKTEST", "SKTEST", &utc(), "cn-beijing", "POST", "/", &query, body);
        assert!(req.url.starts_with("https://open.volcengineapi.com/?Action=ListResourcePackages&Version=2022-01-01"));
        assert_eq!(req.x_date, "20260830T120000Z");
        assert!(req.authorization.starts_with("HMAC-SHA256 Credential=AKTEST/20260830/cn-beijing/billing/request"));
        assert!(req.authorization.contains("SignedHeaders=content-type;host;x-content-sha256;x-date"));
        // Deterministic signature — cross-checked against an independent
        // Python implementation of the documented signing steps.
        let sig = req.authorization.rsplit("Signature=").next().unwrap().to_string();
        assert_eq!(sig.len(), 64);
        assert_eq!(
            sig,
            "020d0464bcf204aebcd01d8af762acd6ae50a64acdd420282e932d9b23079455",
            "signature must match the independent reference implementation"
        );
        assert_eq!(req.x_content_sha256, "73da547950309cc3549fa2f13edefd50fe814f47259bd2cf22c8a590fa17bc50");
        // Same inputs → same output; different body → different signature.
        let req2 = sign("AKTEST", "SKTEST", &utc(), "cn-beijing", "POST", "/", &query, body);
        assert_eq!(req.authorization, req2.authorization);
        let req3 = sign("AKTEST", "SKTEST", &utc(), "cn-beijing", "POST", "/", &query, "other");
        assert_ne!(req.authorization, req3.authorization);
    }

    #[test]
    fn uri_encode_specials() {
        assert_eq!(uri_encode("ListResourcePackages"), "ListResourcePackages");
        assert_eq!(uri_encode("a b/c"), "a%20b%2Fc");
        assert_eq!(uri_encode("2022-01-01"), "2022-01-01");
    }

    #[test]
    fn parses_packages_and_units() {
        let body = r#"{"Result":{"List":[
            {"InstanceNo":"i-1","InstanceName":"方舟大模型Token包","ConfigurationName":"Doubao-1.5-pro 500万Token","Product":"ark",
             "TotalAmount":"500","AvailableAmount":"360.5","Unit":"万Token","EffectiveTime":"1756000000","ExpiryTime":"1790000000","Status":"Effective"},
            {"InstanceNo":"i-2","ConfigurationName":"入门包","TotalAmount":"100","AvailableAmount":"0","Unit":"千Token","ExpiryTime":"1750000000","Status":"UsedUp"}
        ],"NextToken":""}}"#;
        let pkgs = parse_packages(body).unwrap();
        assert_eq!(pkgs.len(), 2);
        let p0 = &pkgs[0];
        assert_eq!(p0.unit_multiplier, 10_000.0);
        assert!((p0.total_amount - 500.0).abs() < 1e-9);
        assert!((p0.used_amount - 139.5).abs() < 1e-9);
        assert_eq!(p0.expiry_ms, Some(1_790_000_000_000));
        assert!((p0.usage_percent.unwrap() - 27.9).abs() < 0.01);
        assert_eq!(pkgs[1].unit_multiplier, 1_000.0);
        assert_eq!(next_token(body), None);
    }

    #[test]
    fn api_error_envelope_parsed() {
        let body = r#"{"ResponseMetadata":{"Error":{"Code":"MissingParameter.ResourceType","Message":"The request missed required parameter"}}}"#;
        match parse_packages(body) {
            Err(VolcError::Api(code, msg)) => {
                assert_eq!(code, "MissingParameter.ResourceType");
                assert!(msg.contains("required"));
            }
            other => panic!("expected api error, got {other:?}"),
        }
    }

    #[test]
    fn pagination_follows_next_token() {
        let page1 = r#"{"Result":{"List":[{"InstanceNo":"a","TotalAmount":"1","AvailableAmount":"1","Status":"Effective"}],"NextToken":"tok2"}}"#;
        let page2 = r#"{"Result":{"List":[{"InstanceNo":"b","TotalAmount":"2","AvailableAmount":"0","Status":"Expired"}],"NextToken":""}}"#;
        struct TwoPage {
            calls: std::sync::Mutex<Vec<String>>,
            page1: String,
            page2: String,
        }
        impl HttpTransport for TwoPage {
            fn post_json(&self, req: &SignedRequest) -> Result<String, VolcError> {
                let mut c = self.calls.lock().unwrap();
                c.push(req.body.clone());
                if c.len() == 1 { Ok(self.page1.clone()) } else { Ok(self.page2.clone()) }
            }
        }
        let t = TwoPage { calls: std::sync::Mutex::new(vec![]), page1: page1.into(), page2: page2.into() };
        let p = VolcengineProvider { region: "cn-beijing".into(), transport: &t, now: utc() };
        let pkgs = p.list_packages("AK", "SK").unwrap();
        assert_eq!(pkgs.len(), 2);
        let calls = t.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(calls[1].contains("tok2"), "second page must carry NextToken");
    }

    #[test]
    fn snapshot_aggregates_and_marks_nearest_expiry() {
        let pkgs = vec![PackageInfo {
            instance_no: "i-1".into(),
            name: "包1".into(),
            total_amount: 100.0,
            available_amount: 50.0,
            used_amount: 50.0,
            unit: "万Token".into(),
            unit_multiplier: 10_000.0,
            expiry_ms: Some(1_788_100_000_000),
            status: "Effective".into(),
            usage_percent: Some(50.0),
            ..Default::default()
        }];
        let now = 1_788_000_000_000;
        let snap = build_snapshot(&pkgs, now, None);
        assert_eq!(snap.status, ProviderStatus::Ok);
        assert_eq!(snap.windows.len(), 1);
        assert_eq!(snap.windows[0].remaining_quota, Some(500_000.0));
        assert!(snap.notes.iter().any(|n| n.contains("到期")));

        let snap_err = build_snapshot(&[], now, Some(&VolcError::NotConfigured));
        assert_eq!(snap_err.status, ProviderStatus::NotConfigured);
        let snap_http = build_snapshot(&[], now, Some(&VolcError::Http(403, "SignatureDoesNotMatch".into())));
        assert_eq!(snap_http.status, ProviderStatus::Error);
        assert!(snap_http.error.unwrap().contains("403"));
    }

    #[test]
    fn unit_multipliers() {
        assert_eq!(unit_multiplier("万Token"), 10_000.0);
        assert_eq!(unit_multiplier("千Token"), 1_000.0);
        assert_eq!(unit_multiplier("百万Token"), 1_000_000.0);
        assert_eq!(unit_multiplier("Token"), 1.0);
        assert_eq!(unit_multiplier("张"), 1.0);
    }
}
