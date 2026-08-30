//! Google Antigravity provider — local official-client data only.
//!
//! Antigravity has no public quota API. Its IDE/CLI daemons (language_server)
//! expose a local Connect-RPC service on 127.0.0.1; the port and CSRF token
//! are printed by the daemon itself into its own log files. We:
//!   1. detect an installation (config dir / CLI dir),
//!   2. tail `language_server.log` / `main.log` for the endpoint,
//!   3. call `GetUserStatus` (Connect protocol) on loopback only,
//!   4. parse account/plan/quota defensively (fields all optional).
//!
//! No browser cookies are read, no credentials decrypted, nothing scraped.
//! When any step fails the provider degrades to NotInstalled/Unavailable —
//! data is never invented.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{ProviderSnapshot, ProviderStatus, QuotaWindow};

pub const RPC_PATH: &str = "/exa.language_server_pb.LanguageServerService/GetUserStatus";
const LOG_SCAN_BYTES: u64 = 512 * 1024; // tail only — logs can be large

#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
    pub port: u16,
    pub csrf_token: Option<String>,
    pub https: bool,
}

impl Endpoint {
    pub fn url(&self) -> String {
        format!(
            "{}://127.0.0.1:{}{}",
            if self.https { "https" } else { "http" },
            self.port,
            RPC_PATH
        )
    }
}

/// Injectable loopback transport (tests inject fixtures).
pub trait LocalTransport: Send + Sync {
    fn call(&self, url: &str, csrf: Option<&str>, body: &str) -> Result<String, String>;
}

pub struct UreqLocalTransport {
    pub timeout_secs: u64,
}

impl LocalTransport for UreqLocalTransport {
    fn call(&self, url: &str, csrf: Option<&str>, body: &str) -> Result<String, String> {
        let agent = agent_for(url)?;
        let mut req = agent
            .post(url)
            .set("Content-Type", "application/json")
            .set("Connect-Protocol-Version", "1")
            .timeout(std::time::Duration::from_secs(self.timeout_secs.max(2)));
        if let Some(t) = csrf {
            req = req.set("X-Codeium-Csrf-Token", t);
        }
        let resp = req
            .send_string(body)
            .map_err(|e| format!("rpc: {e}"))?;
        resp.into_string().map_err(|e| format!("rpc body: {e}"))
    }
}

/// Build a ureq agent. For loopback HTTPS the daemon uses a self-signed
/// cert, so verification is relaxed — but ONLY for 127.0.0.1 URLs.
fn agent_for(url: &str) -> Result<ureq::Agent, String> {
    if url.starts_with("https://127.0.0.1") {
        let cfg = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(std::sync::Arc::new(LoopbackOnlyVerifier))
            .with_no_client_auth();
        Ok(ureq::AgentBuilder::new()
            .tls_config(std::sync::Arc::new(cfg))
            .build())
    } else {
        Ok(ureq::AgentBuilder::new().build())
    }
}

#[derive(Debug)]
struct LoopbackOnlyVerifier;

impl rustls::client::danger::ServerCertVerifier for LoopbackOnlyVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        // Trust only reaches this verifier for 127.0.0.1 URLs (agent_for).
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA384,
        ]
    }
}

// ---------------------------------------------------------------------------
// Installation + endpoint discovery (pure-ish, testable)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct InstallPaths {
    pub config_dir: Option<PathBuf>,
    pub cli_root: Option<PathBuf>,
}

/// Locate Antigravity's on-disk footprint. Windows paths first (product
/// target), then Linux/macOS for dev builds.
pub fn detect_installation(home: Option<&Path>) -> InstallPaths {
    let home = home.map(|h| h.to_path_buf()).or_else(dirs::home_dir).unwrap_or_default();
    let mut out = InstallPaths::default();
    let candidates = [
        // IDE config (Electron userData)
        home.join("AppData/Roaming/Antigravity"), // Windows
        home.join(".config/Antigravity"),         // Linux
        home.join("Library/Application Support/Antigravity"), // macOS
    ];
    for c in &candidates {
        if c.join("User/globalStorage").is_dir() || c.is_dir() {
            out.config_dir = Some(c.clone());
            break;
        }
    }
    // CLI daemon root (~/.gemini/antigravity-cli)
    let cli = home.join(".gemini/antigravity-cli");
    if cli.is_dir() {
        out.cli_root = Some(cli);
    }
    out
}

/// Candidate log files that may reveal the daemon endpoint.
pub fn log_candidates(install: &InstallPaths) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(cfg) = &install.config_dir {
        v.push(cfg.join("logs/language_server.log"));
        v.push(cfg.join("logs/main.log"));
        v.push(cfg.join("language_server.log"));
    }
    if let Some(cli) = &install.cli_root {
        v.push(cli.join("cli.log"));
    }
    v
}

/// Parse a log tail for the newest endpoint hints. Recognized forms
/// (case-insensitive keys): `--extension_server_port <p>`, `port=<p>`,
/// `"port":<p>`, `listening on 127.0.0.1:<p>`, `--csrf_token <t>`,
/// `csrf_token=<t>`.
pub fn parse_endpoint_from_log(text: &str) -> Vec<Endpoint> {
    let mut out: Vec<Endpoint> = Vec::new();
    let mut csrf: Option<String> = None;
    let plausible = |v: &str| v.len() >= 8 && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    let mut tokens = text.split_whitespace();
    while let Some(tok) = tokens.next() {
        let tok = tok.trim_end_matches(',');
        if let Some(v) = tok.strip_prefix("--csrf_token=") {
            if plausible(v) {
                csrf = Some(v.to_string());
            }
        } else if tok == "--csrf_token" || tok.eq_ignore_ascii_case("csrf_token") {
            // `--csrf_token <value>` — the value is the next token.
            if let Some(next) = tokens.next().map(|t| t.trim_end_matches(',')) {
                if plausible(next) && next.parse::<f64>().is_err() {
                    csrf = Some(next.to_string());
                }
            }
        } else if let Some((k, v)) = tok.split_once('=') {
            if (k.eq_ignore_ascii_case("--csrf_token") || k.eq_ignore_ascii_case("csrf_token")) && plausible(v) {
                csrf = Some(v.to_string());
            }
        }
    }
    // find ports after "port" tokens…
    for hit in text.match_indices("port").map(|(i, _)| i) {
        let rest = &text[hit..];
        let port = extract_port_after(rest);
        if let Some(p) = port {
            if !(1024..=65535).contains(&p) {
                continue;
            }
            let ctx = text.get(..(hit + 64).min(text.len())).unwrap_or(text);
            let https = ctx.to_ascii_lowercase().contains("https");
            let ep = Endpoint { port: p, csrf_token: csrf.clone(), https };
            if !out.contains(&ep) {
                out.push(ep);
            }
            if out.len() >= 8 {
                break;
            }
        }
    }
    // …and "127.0.0.1:<port>" listen lines.
    for hit in text.match_indices("127.0.0.1").map(|(i, _)| i) {
        let rest = &text[hit..];
        if let Some(p) = extract_port_after(rest) {
            if (1024..=65535).contains(&p) {
                let ep = Endpoint { port: p, csrf_token: csrf.clone(), https: false };
                if !out.contains(&ep) {
                    out.push(ep);
                }
            }
        }
    }
    out
}

fn extract_port_after(text: &str) -> Option<u16> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b':' || c == b'=' || c == b' ' {
            // consume run of separators
            let mut j = i;
            while j < bytes.len() && (bytes[j] == b':' || bytes[j] == b'=' || bytes[j] == b' ') {
                j += 1;
            }
            if j < bytes.len() && bytes[j].is_ascii_digit() {
                let mut k = j;
                while k < bytes.len() && bytes[k].is_ascii_digit() {
                    k += 1;
                }
                if let Ok(p) = text[j..k].parse::<u16>() {
                    return Some(p);
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

fn read_tail(path: &Path, max_bytes: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let size = f.metadata().ok()?.len();
    let start = size.saturating_sub(max_bytes);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = String::new();
    f.read_to_string(&mut buf).ok()?;
    Some(buf)
}

/// Discover endpoints by scanning the newest of the candidate logs.
pub fn discover_endpoints(install: &InstallPaths) -> Vec<Endpoint> {
    let mut best: Option<(std::time::SystemTime, Vec<Endpoint>)> = None;
    for log in log_candidates(install) {
        if !log.is_file() {
            continue;
        }
        let mtime = std::fs::metadata(&log)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        if let Some(text) = read_tail(&log, LOG_SCAN_BYTES) {
            let eps = parse_endpoint_from_log(&text);
            if !eps.is_empty() {
                match &best {
                    Some((t, _)) if *t >= mtime => {}
                    _ => best = Some((mtime, eps)),
                }
            }
        }
    }
    best.map(|(_, eps)| eps).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Response parsing (defensive; every field optional)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct QuotaInfo {
    pub model: Option<String>,
    /// 0.0–1.0 remaining fraction.
    pub remaining_fraction: Option<f64>,
    pub reset_time: Option<String>,
}

fn iso_to_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
}

/// Parse a GetUserStatus response into (account, plan, quotas). Tolerant of
/// schema drift — unknown shapes yield empty results, never errors.
pub fn parse_user_status(body: &str) -> Option<(Option<String>, Option<String>, Vec<QuotaInfo>)> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let us = v.get("userStatus").cloned().unwrap_or(v);
    let email = us
        .get("accountEmail")
        .and_then(|e| e.as_str())
        .map(|s| s.to_string());
    let plan = us
        .pointer("/planStatus/planInfo/planName")
        .and_then(|p| p.as_str())
        .map(|s| s.to_string());
    let mut quotas = Vec::new();
    if let Some(configs) = us
        .pointer("/cascadeModelConfigData/clientModelConfigs")
        .and_then(|c| c.as_array())
    {
        for c in configs {
            let model = c
                .get("modelName")
                .or_else(|| c.get("displayName"))
                .and_then(|m| m.as_str())
                .map(|s| s.to_string());
            if let Some(q) = c.get("quotaInfo") {
                let rf = q
                    .get("remainingFraction")
                    .and_then(|f| f.as_f64())
                    .or_else(|| q.get("remaining_fraction").and_then(|f| f.as_f64()));
                let reset = q
                    .get("resetTime")
                    .and_then(|r| r.as_str())
                    .map(|s| s.to_string());
                if rf.is_some() || reset.is_some() {
                    quotas.push(QuotaInfo { model, remaining_fraction: rf, reset_time: reset });
                }
            }
        }
    }
    Some((email, plan, quotas))
}

/// Parse a RetrieveUserQuotaSummary response into per-group quota windows
/// (newer RPC; groups → buckets with remainingFraction).
pub fn parse_quota_summary(body: &str) -> Vec<(String, QuotaInfo)> {
    let mut out = Vec::new();
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return out;
    };
    let Some(groups) = v
        .pointer("/response/groups")
        .or_else(|| v.get("groups"))
        .and_then(|g| g.as_array())
    else {
        return out;
    };
    for g in groups {
        let name = g
            .get("groupName")
            .or_else(|| g.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("额度")
            .to_string();
        if let Some(buckets) = g.get("buckets").and_then(|b| b.as_array()) {
            for b in buckets {
                let bucket_type = b
                    .get("bucketType")
                    .or_else(|| b.get("type"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                let rf = b
                    .pointer("/remaining/remainingFraction")
                    .or_else(|| b.get("remainingFraction"))
                    .and_then(|f| f.as_f64());
                let reset = b
                    .pointer("/remaining/resetTime")
                    .or_else(|| b.get("resetTime").filter(|v| !v.is_null()))
                    .and_then(|r| r.as_str())
                    .map(|s| s.to_string());
                if rf.is_some() || reset.is_some() {
                    out.push((
                        name.clone(),
                        QuotaInfo { model: Some(bucket_type).filter(|s| !s.is_empty()), remaining_fraction: rf, reset_time: reset },
                    ));
                }
            }
        }
    }
    out
}

pub const USER_STATUS_BODY: &str =
    r#"{"metadata":{"ideName":"antigravity","extensionName":"antigravity","locale":"en"}}"#;

/// One provider poll.
pub fn poll(install: &InstallPaths, transport: &dyn LocalTransport, now_ms: i64) -> ProviderSnapshot {
    let mut snap = ProviderSnapshot::empty(super::PROVIDER_ANTIGRAVITY, ProviderStatus::Ok, now_ms);
    snap.source = "Antigravity 本地 language_server RPC(官方客户端,仅 127.0.0.1)".into();

    if install.config_dir.is_none() && install.cli_root.is_none() {
        snap.status = ProviderStatus::NotInstalled;
        snap.error = Some("未检测到 Antigravity 安装(IDE 或 CLI)".into());
        return snap;
    }

    let endpoints = discover_endpoints(install);
    if endpoints.is_empty() {
        snap.status = ProviderStatus::NotConfigured;
        snap.error = Some("已检测到安装,但未找到运行中的本地服务(启动 Antigravity 后自动恢复)".into());
        snap.notes.push("数据来自 Antigravity 官方本地守护进程,无公开远程 API;未运行时无法查询".into());
        return snap;
    }

    let mut last_err = String::new();
    for ep in &endpoints {
        match transport.call(&ep.url(), ep.csrf_token.as_deref(), USER_STATUS_BODY) {
            Ok(body) => {
                if let Some((email, plan, quotas)) = parse_user_status(&body) {
                    snap.account = email;
                    snap.plan_name = plan;
                    let mut merged: Vec<QuotaInfo> = quotas;
                    // Enrich with the quota-summary RPC when available.
                    let summary_url = ep.url().replace("GetUserStatus", "RetrieveUserQuotaSummary");
                    if let Ok(sb) = transport.call(&summary_url, ep.csrf_token.as_deref(), USER_STATUS_BODY) {
                        let extra = parse_quota_summary(&sb);
                        if !extra.is_empty() {
                            snap.notes.push("周额度来自 RetrieveUserQuotaSummary".into());
                        }
                        for (group, q) in extra {
                            let label_model = q.model.clone().unwrap_or_default();
                            let entry = if label_model.is_empty() { None } else { merged.iter_mut().find(|m| m.model.as_deref() == Some(label_model.as_str())) };
                            match entry {
                                Some(existing) => {
                                    if existing.remaining_fraction.is_none() {
                                        existing.remaining_fraction = q.remaining_fraction;
                                    }
                                }
                                None => merged.push(QuotaInfo {
                                    model: Some(format!("{group}·{}", q.model.unwrap_or_default())),
                                    remaining_fraction: q.remaining_fraction,
                                    reset_time: q.reset_time,
                                }),
                            }
                        }
                    }
                    snap.windows = merged
                        .into_iter()
                        .filter_map(|q| {
                            let used = q.remaining_fraction.map(|f| (1.0 - f.clamp(0.0, 1.0)) * 100.0);
                            if used.is_none() && q.reset_time.is_none() {
                                return None;
                            }
                            Some(QuotaWindow {
                                key: format!("model:{}", q.model.clone().unwrap_or_default()),
                                label: q.model.clone().unwrap_or_else(|| "额度".into()),
                                used_percent: used,
                                unit: Some("% 套餐额度".into()),
                                reset_at_ms: q.reset_time.as_deref().and_then(iso_to_ms),
                                ..Default::default()
                            })
                        })
                        .collect();
                    if snap.windows.is_empty() {
                        snap.status = ProviderStatus::NotConfigured;
                        snap.error = Some("本地服务已连接,但未返回额度字段(客户端版本可能不提供)".into());
                    } else {
                        snap.notes.push("剩余比例来自官方客户端返回的 remainingFraction".into());
                    }
                    return snap;
                }
                last_err = "响应无法解析".into();
            }
            Err(e) => last_err = e,
        }
    }
    snap.status = ProviderStatus::Error;
    snap.error = Some(format!("本地服务查询失败:{last_err}"));
    snap
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixTransport<'a> {
        responses: &'a [(&'static str, &'static str)],
    }
    impl LocalTransport for FixTransport<'_> {
        fn call(&self, url: &str, _csrf: Option<&str>, _body: &str) -> Result<String, String> {
            for (frag, body) in self.responses {
                if url.contains(frag) {
                    return Ok((*body).to_string());
                }
            }
            Err("no fixture".into())
        }
    }

    #[test]
    fn endpoint_parsing_from_spawn_line() {
        let log = r#"
[info] spawning language_server --csrf_token AbCdEf123456 --extension_server_port 48071
[info] daemon listening on 127.0.0.1:48071 (https)
"#;
        let eps = parse_endpoint_from_log(log);
        assert!(!eps.is_empty());
        assert_eq!(eps[0].port, 48071);
        assert_eq!(eps[0].csrf_token.as_deref(), Some("AbCdEf123456"));
    }

    #[test]
    fn endpoint_parsing_port_equals_form() {
        let log = "server started port=51234 ready";
        let eps = parse_endpoint_from_log(log);
        assert!(eps.iter().any(|e| e.port == 51234), "{eps:?}");
        let log2 = r#"{"level":"info","port":49999}"#;
        let eps2 = parse_endpoint_from_log(log2);
        assert!(eps2.iter().any(|e| e.port == 49999));
    }

    #[test]
    fn detect_installation_missing() {
        let dir = tempfile::tempdir().unwrap();
        let install = detect_installation(Some(dir.path()));
        assert!(install.config_dir.is_none());
        assert!(install.cli_root.is_none());
        let snap = poll(&install, &FixTransport { responses: &[] }, 0);
        assert_eq!(snap.status, ProviderStatus::NotInstalled);
    }

    #[test]
    fn not_running_degrades_to_not_configured() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".config/Antigravity/User")).unwrap();
        let install = detect_installation(Some(dir.path()));
        assert!(install.config_dir.is_some());
        let snap = poll(&install, &FixTransport { responses: &[] }, 0);
        assert_eq!(snap.status, ProviderStatus::NotConfigured);
        assert!(snap.error.unwrap().contains("未找到运行中的本地服务"));
    }

    #[test]
    fn parses_user_status_quotas() {
        let body = r#"{"userStatus":{
            "accountEmail":"u@example.com",
            "planStatus":{"planInfo":{"planName":"Pro"}},
            "cascadeModelConfigData":{"clientModelConfigs":[
                {"modelName":"Claude Sonnet 4.5","quotaInfo":{"remainingFraction":0.42,"resetTime":"2026-08-31T10:00:00Z"}},
                {"modelName":"Gemini 3 Pro","quotaInfo":{"remainingFraction":0.9}}
            ]}
        }}"#;
        let (email, plan, quotas) = parse_user_status(body).unwrap();
        assert_eq!(email.as_deref(), Some("u@example.com"));
        assert_eq!(plan.as_deref(), Some("Pro"));
        assert_eq!(quotas.len(), 2);
        assert!((quotas[0].remaining_fraction.unwrap() - 0.42).abs() < 1e-9);

        // Full poll path through the transport.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".config/Antigravity/logs")).unwrap();
        std::fs::write(
            dir.path().join(".config/Antigravity/logs/language_server.log"),
            "listening on 127.0.0.1:48077 csrf_token=Zz1234567890",
        )
        .unwrap();
        let install = detect_installation(Some(dir.path()));
        let t = FixTransport {
            responses: &[("GetUserStatus", body), ("RetrieveUserQuotaSummary", "{}")],
        };
        let snap = poll(&install, &t, 1_788_000_000_000);
        assert_eq!(snap.status, ProviderStatus::Ok, "{:?}", snap.error);
        assert_eq!(snap.plan_name.as_deref(), Some("Pro"));
        assert_eq!(snap.windows.len(), 2);
        let w = &snap.windows[0];
        assert!((w.used_percent.unwrap() - 58.0).abs() < 1e-6);
        assert_eq!(w.reset_at_ms, Some(iso_to_ms("2026-08-31T10:00:00Z").unwrap()));
    }

    #[test]
    fn quota_summary_groups_parsed() {
        let body = r#"{"response":{"groups":[
            {"groupName":"Gemini Models","buckets":[{"bucketType":"WEEKLY","remaining":{"remainingFraction":0.65}}]},
            {"groupName":"Claude and GPT models","buckets":[{"bucketType":"FIVE_HOUR","remaining":{"remainingFraction":0.80}}]}
        ]}}"#;
        let groups = parse_quota_summary(body);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "Gemini Models");
        assert!((groups[0].1.remaining_fraction.unwrap() - 0.65).abs() < 1e-9);
        assert_eq!(groups[0].1.model.as_deref(), Some("WEEKLY"));
    }

    #[test]
    fn rpc_failure_degrades_to_error_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".config/Antigravity/logs")).unwrap();
        std::fs::write(
            dir.path().join(".config/Antigravity/logs/language_server.log"),
            "port 48088",
        )
        .unwrap();
        let install = detect_installation(Some(dir.path()));
        struct Fail;
        impl LocalTransport for Fail {
            fn call(&self, _u: &str, _c: Option<&str>, _b: &str) -> Result<String, String> {
                Err("connection refused".into())
            }
        }
        let snap = poll(&install, &Fail, 0);
        assert_eq!(snap.status, ProviderStatus::Error);
        assert!(snap.error.unwrap().contains("查询失败"));
    }
}
