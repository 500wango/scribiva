//! Scribiva service API (docs/05 §8). The pure part (URL validation, response
//! parsing, error mapping) is platform independent and unit-tested; the HTTP
//! client is only compiled for the native Windows core.

use crate::core::{ErrorInfo, FailureClass};
use serde::{Deserialize, Serialize};
use url::{Host, Url};

pub const CLIENT_NAME: &str = "scribiva-windows";
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Hosts that may be opened in the browser for billing (FR-010).
pub const TRUSTED_BILLING_HOSTS: &[&str] = &["checkout.stripe.com", "billing.stripe.com"];

// ---------------------------------------------------------------- URLs

fn is_loopback(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(d) => d.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(ip) => ip.is_loopback(),
        Host::Ipv6(ip) => ip.is_loopback(),
    }
}

/// Validate and normalise the service base URL. HTTPS is required unless the
/// host is loopback; credentials, query and fragment are rejected. Returns the
/// URL without a trailing slash.
pub fn validate_base_url(input: &str) -> Result<String, String> {
    let value = input.trim();
    if value.is_empty() {
        return Err("Service URL is required".into());
    }
    if value.len() > 512 || value.chars().any(char::is_whitespace) {
        return Err("Service URL is invalid".into());
    }
    let url = Url::parse(value).map_err(|_| "Service URL is invalid".to_string())?;
    let host = url
        .host()
        .ok_or_else(|| "Service URL must include a host".to_string())?;
    match url.scheme() {
        "https" => {}
        "http" if is_loopback(&host) => {}
        "http" => return Err("Service URL must use HTTPS unless it points to loopback".into()),
        _ => return Err("Service URL must be an http(s) URL".into()),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Service URL must not embed credentials".into());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("Service URL must not contain a query or fragment".into());
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

/// Only Stripe-hosted Checkout / Billing Portal pages may be opened.
pub fn is_trusted_billing_url(input: &str) -> bool {
    let Ok(url) = Url::parse(input.trim()) else {
        return false;
    };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && matches!(url.host(), Some(Host::Domain(h)) if TRUSTED_BILLING_HOSTS.contains(&h))
}

/// The device-login page must be a web page served over HTTPS (or loopback
/// HTTP for local development) without embedded credentials.
pub fn is_safe_verification_url(input: &str) -> bool {
    let Ok(url) = Url::parse(input.trim()) else {
        return false;
    };
    let Some(host) = url.host() else {
        return false;
    };
    let scheme_ok = match url.scheme() {
        "https" => true,
        "http" => is_loopback(&host),
        _ => false,
    };
    scheme_ok && url.username().is_empty() && url.password().is_none()
}

/// `scr_` tokens: ASCII URL-safe characters only, bounded length.
pub fn is_valid_token(token: &str) -> bool {
    token.len() > 8
        && token.len() <= 256
        && token.starts_with("scr_")
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

// ---------------------------------------------------------------- payloads

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct DeviceStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    #[serde(default = "default_expires")]
    pub expires_in: u64,
    #[serde(default = "default_interval")]
    pub interval: u64,
}
fn default_expires() -> u64 {
    900
}
fn default_interval() -> u64 {
    3
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct DeviceToken {
    pub access_token: String,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// One step of device-flow polling (§8.1 step 4).
#[derive(Debug, Clone, PartialEq)]
pub enum PollStep {
    Token(DeviceToken),
    Pending,
    /// Increase the interval by 5 seconds.
    SlowDown,
    Expired,
    Failed(String),
}

pub fn parse_poll(status: u16, body: &str) -> PollStep {
    if status == 200 {
        return match serde_json::from_str::<DeviceToken>(body) {
            Ok(t) if is_valid_token(&t.access_token) => PollStep::Token(t),
            _ => PollStep::Failed("INVALID_TOKEN_RESPONSE".into()),
        };
    }
    let err = parse_error(body);
    match err.as_ref().map(|e| e.code.as_str()) {
        Some("AUTHORIZATION_PENDING") => PollStep::Pending,
        Some("SLOW_DOWN") => PollStep::SlowDown,
        Some("EXPIRED_TOKEN") => PollStep::Expired,
        Some(code) => PollStep::Failed(code.to_string()),
        None if status == 429 => PollStep::SlowDown,
        None => PollStep::Failed(format!("HTTP_{status}")),
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Billing {
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub charged_audio_seconds: Option<f64>,
    #[serde(default)]
    pub remaining_seconds: Option<i64>,
    #[serde(default)]
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct DictationResponse {
    #[serde(default)]
    pub job_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub raw_text: Option<String>,
    #[serde(default)]
    pub corrected_text: Option<String>,
    #[serde(default)]
    pub output_text: Option<String>,
    #[serde(default)]
    pub detected_language: Option<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub suspected_hallucination: bool,
    #[serde(default)]
    pub billing: Option<Billing>,
    #[serde(default)]
    pub poll_after_ms: Option<u64>,
}

impl DictationResponse {
    pub fn is_processing(&self) -> bool {
        self.status.as_deref() == Some("processing")
    }
    /// Text to deliver: output, else corrected, else raw.
    pub fn final_text(&self) -> String {
        self.output_text
            .clone()
            .or_else(|| self.corrected_text.clone())
            .or_else(|| self.raw_text.clone())
            .unwrap_or_default()
    }
    /// FR-017: either the flag or a `SUSPECTED_HALLUCINATION` warning.
    pub fn is_suspected_noise(&self) -> bool {
        self.suspected_hallucination
            || self
                .warnings
                .iter()
                .any(|w| w.starts_with("SUSPECTED_HALLUCINATION"))
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Usage {
    #[serde(default)]
    pub used_seconds: i64,
    #[serde(default)]
    pub reserved_seconds: i64,
    #[serde(default)]
    pub limit_seconds: i64,
    #[serde(default)]
    pub remaining_seconds: i64,
    #[serde(default)]
    pub resets_at: Option<String>,
    #[serde(default)]
    pub grace_seconds: Option<i64>,
    #[serde(default)]
    pub plan: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct UserInfo {
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Subscription {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub current_period_end: Option<String>,
}

/// `GET /v1/me`, reduced to the fields the UI shows. No ids or tokens.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Me {
    #[serde(default)]
    pub user: UserInfo,
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub subscription: Subscription,
    #[serde(default)]
    pub usage: Usage,
    #[serde(default)]
    pub billing_configured: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Dictionary {
    #[serde(default)]
    pub terms: Vec<String>,
    #[serde(default = "default_max_terms")]
    pub max_terms: usize,
    #[serde(default = "default_max_term_len")]
    pub max_term_length: usize,
}
fn default_max_terms() -> usize {
    200
}
fn default_max_term_len() -> usize {
    64
}

/// Client-side pre-check before `PUT /v1/me/dictionary` (server stays authoritative).
pub fn clean_terms(
    terms: &[String],
    max_terms: usize,
    max_len: usize,
) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for t in terms {
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        if t.chars().count() > max_len {
            return Err(format!("Term longer than {max_len} characters"));
        }
        if t.chars().any(char::is_control) {
            return Err("Terms must not contain control characters".into());
        }
        if !out.iter().any(|o| o.to_lowercase() == t.to_lowercase()) {
            out.push(t.to_string());
        }
    }
    if out.len() > max_terms {
        return Err(format!("At most {max_terms} terms"));
    }
    Ok(out)
}

// ---------------------------------------------------------------- errors

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ApiErrorBody {
    pub code: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub retryable: bool,
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ApiErrorBody,
}

pub fn parse_error(body: &str) -> Option<ApiErrorBody> {
    serde_json::from_str::<ErrorEnvelope>(body)
        .ok()
        .map(|e| e.error)
        .filter(|e| !e.code.is_empty())
}

/// Map an HTTP failure of `POST /v1/dictations` (or the job poll) to the
/// state-machine error. `retryable` here means "the retained audio may be
/// retried by the user"; the class decides whether the same key is reused.
pub fn map_dictation_error(status: u16, body: &str) -> ErrorInfo {
    let parsed = parse_error(body);
    let code = parsed
        .as_ref()
        .map(|e| e.code.clone())
        .unwrap_or_else(|| format!("HTTP_{status}"));
    let message = parsed
        .as_ref()
        .map(|e| e.message.clone())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| format!("Service returned HTTP {status}"));
    let (class, retryable) = match (status, code.as_str()) {
        (401, _) => (FailureClass::Auth, true),
        (402, _) | (403, "QUOTA_EXCEEDED") | (403, "PLAN_INACTIVE") => (FailureClass::Quota, false),
        // Another job is still processing: our job was not created, same key is fine.
        (409, "BUSY") => (FailureClass::Transport, true),
        // Original job failed or was cancelled: a retry needs a new key.
        (409, _) => (FailureClass::Server, true),
        (410, _) => (FailureClass::Server, true),
        (413, _) | (422, _) => (FailureClass::Server, false),
        (429, _) => (FailureClass::Transport, true),
        // Gateway errors without our JSON body: the job may still be running,
        // so the same key is reused and the server replays or reports it.
        (502..=504, c) if parsed.is_none() || c.starts_with("HTTP_") => {
            (FailureClass::Transport, true)
        }
        (500..=599, _) => (FailureClass::Server, true),
        (400..=499, _) => (FailureClass::Server, false),
        _ => (FailureClass::Server, true),
    };
    ErrorInfo {
        code,
        message,
        class,
        retryable,
    }
}

/// Network-level failure (DNS, TLS, timeout): the request may have arrived.
pub fn transport_error(message: impl Into<String>) -> ErrorInfo {
    ErrorInfo {
        code: "NETWORK".into(),
        message: message.into(),
        class: FailureClass::Transport,
        retryable: true,
    }
}

/// Parse `Retry-After` seconds, bounded so a hostile value cannot stall us.
pub fn retry_after_seconds(header: Option<&str>) -> u64 {
    header
        .and_then(|h| h.trim().parse::<u64>().ok())
        .unwrap_or(2)
        .clamp(1, 30)
}

/// Delay before polling a `202` job again: server hint, exponential backoff, capped.
pub fn poll_delay_ms(hint: Option<u64>, attempt: u32) -> u64 {
    let base = hint.unwrap_or(500).clamp(200, 5_000);
    base.saturating_mul(1u64 << attempt.min(4)).min(5_000)
}

#[cfg(windows)]
pub use client::*;

#[cfg(windows)]
mod client {
    use super::*;
    use reqwest::{header, multipart, Client, StatusCode};
    use std::time::Duration;

    #[derive(Debug, Clone)]
    pub struct DictationRequest {
        pub job_id: String,
        pub wav: Vec<u8>,
        pub duration_ms: u64,
        pub language: String,
        pub grammar_mode: String,
        pub output_script: String,
        pub dictation_commands: bool,
    }

    #[derive(Debug, Clone)]
    pub enum CallError {
        Http(ErrorInfo),
        Cancelled,
    }

    pub struct ApiClient {
        http: Client,
        base: String,
    }

    impl ApiClient {
        pub fn new(base_url: &str) -> Result<Self, String> {
            let base = validate_base_url(base_url)?;
            let http = Client::builder()
                .tls_backend_rustls()
                .https_only(!base.starts_with("http://"))
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(90))
                .user_agent(format!("{CLIENT_NAME}/{CLIENT_VERSION}"))
                .build()
                .map_err(|e| format!("HTTP client: {e}"))?;
            Ok(Self { http, base })
        }

        fn url(&self, path: &str) -> String {
            format!("{}{}", self.base, path)
        }

        async fn read(resp: reqwest::Response) -> (u16, Option<String>, String) {
            let status = resp.status().as_u16();
            let retry = resp
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            // Bodies are small JSON; cap what we keep in memory.
            let body = resp.text().await.unwrap_or_default();
            let body = if body.len() > 1_000_000 {
                String::new()
            } else {
                body
            };
            (status, retry, body)
        }

        fn json_error(status: u16, body: &str) -> String {
            match parse_error(body) {
                Some(e) => format!("{}: {}", e.code, e.message),
                None => format!("HTTP_{status}"),
            }
        }

        pub async fn device_start(&self) -> Result<DeviceStart, String> {
            let resp = self
                .http
                .post(self.url("/v1/auth/device"))
                .json(&serde_json::json!({ "client_name": CLIENT_NAME }))
                .send()
                .await
                .map_err(|e| format!("NETWORK: {e}"))?;
            let (status, _, body) = Self::read(resp).await;
            if status != 200 {
                return Err(Self::json_error(status, &body));
            }
            serde_json::from_str(&body).map_err(|_| "INVALID_RESPONSE".to_string())
        }

        pub async fn device_poll(&self, device_code: &str) -> PollStep {
            match self
                .http
                .post(self.url("/v1/auth/device/token"))
                .json(&serde_json::json!({ "device_code": device_code }))
                .send()
                .await
            {
                Ok(resp) => {
                    let (status, _, body) = Self::read(resp).await;
                    parse_poll(status, &body)
                }
                // Transient network failure while polling: keep waiting.
                Err(_) => PollStep::Pending,
            }
        }

        async fn authed_json<T: serde::de::DeserializeOwned>(
            &self,
            req: reqwest::RequestBuilder,
            token: &str,
        ) -> Result<T, ErrorInfo> {
            let resp = req
                .bearer_auth(token)
                .send()
                .await
                .map_err(|e| transport_error(e.to_string()))?;
            let (status, _, body) = Self::read(resp).await;
            if !(200..300).contains(&status) {
                return Err(map_dictation_error(status, &body));
            }
            serde_json::from_str(&body).map_err(|_| ErrorInfo {
                code: "INVALID_RESPONSE".into(),
                message: "Unexpected response from the service".into(),
                class: FailureClass::Server,
                retryable: false,
            })
        }

        pub async fn me(&self, token: &str) -> Result<Me, ErrorInfo> {
            self.authed_json(self.http.get(self.url("/v1/me")), token)
                .await
        }

        pub async fn dictionary(&self, token: &str) -> Result<Dictionary, ErrorInfo> {
            self.authed_json(self.http.get(self.url("/v1/me/dictionary")), token)
                .await
        }

        pub async fn put_dictionary(
            &self,
            token: &str,
            terms: &[String],
        ) -> Result<Dictionary, ErrorInfo> {
            let req = self
                .http
                .put(self.url("/v1/me/dictionary"))
                .json(&serde_json::json!({ "terms": terms }));
            self.authed_json(req, token).await
        }

        pub async fn checkout(&self, token: &str, plan: &str) -> Result<String, ErrorInfo> {
            let req = self
                .http
                .post(self.url("/v1/billing/checkout-session"))
                .json(&serde_json::json!({ "plan": plan }));
            let v: serde_json::Value = self.authed_json(req, token).await?;
            Ok(v["url"].as_str().unwrap_or_default().to_string())
        }

        pub async fn portal(&self, token: &str) -> Result<String, ErrorInfo> {
            let req = self
                .http
                .post(self.url("/v1/billing/portal-session"))
                .json(&serde_json::json!({}));
            let v: serde_json::Value = self.authed_json(req, token).await?;
            Ok(v["url"].as_str().unwrap_or_default().to_string())
        }

        pub async fn delete_data(&self, token: &str) -> Result<serde_json::Value, ErrorInfo> {
            let req = self
                .http
                .delete(self.url("/v1/me/data"))
                .json(&serde_json::json!({ "confirm": "DELETE" }));
            self.authed_json(req, token).await
        }

        /// Best-effort remote cancel of a superseded or cancelled job.
        pub async fn cancel_job(&self, token: &str, job_id: &str) {
            let _ = self
                .http
                .post(self.url(&format!("/v1/dictations/{job_id}/cancel")))
                .bearer_auth(token)
                .timeout(Duration::from_secs(10))
                .send()
                .await;
        }

        /// Upload one recording and wait for its result. `still_current` is
        /// checked between network steps so cancelled jobs stop early.
        pub async fn dictate(
            &self,
            token: &str,
            req: &DictationRequest,
            still_current: impl Fn() -> bool,
        ) -> Result<DictationResponse, CallError> {
            let mut rate_limited = 0u32;
            let first = loop {
                if !still_current() {
                    return Err(CallError::Cancelled);
                }
                let audio = multipart::Part::bytes(req.wav.clone())
                    .file_name("audio.wav")
                    .mime_str("audio/wav")
                    .map_err(|e| CallError::Http(transport_error(e.to_string())))?;
                let mut form = multipart::Form::new()
                    .part("audio", audio)
                    .text("language", req.language.clone())
                    .text("grammar_mode", req.grammar_mode.clone())
                    .text("client_job_id", req.job_id.clone())
                    .text("client_version", CLIENT_VERSION.to_string())
                    .text("duration_ms", req.duration_ms.to_string())
                    .text(
                        "dictation_commands",
                        if req.dictation_commands { "1" } else { "0" },
                    );
                if !req.output_script.is_empty() {
                    form = form.text("output_script", req.output_script.clone());
                }
                let resp = self
                    .http
                    .post(self.url("/v1/dictations"))
                    .bearer_auth(token)
                    .header("Idempotency-Key", req.job_id.as_str())
                    .multipart(form)
                    .send()
                    .await
                    .map_err(|e| CallError::Http(transport_error(e.to_string())))?;
                let (status, retry_after, body) = Self::read(resp).await;
                if status == StatusCode::TOO_MANY_REQUESTS.as_u16() && rate_limited < 2 {
                    rate_limited += 1;
                    tokio::time::sleep(Duration::from_secs(retry_after_seconds(
                        retry_after.as_deref(),
                    )))
                    .await;
                    continue;
                }
                break (status, body);
            };
            let (mut status, mut body) = first;
            let mut attempt = 0u32;
            let started = std::time::Instant::now();
            loop {
                if !(200..300).contains(&status) {
                    return Err(CallError::Http(map_dictation_error(status, &body)));
                }
                let parsed: DictationResponse = serde_json::from_str(&body).map_err(|_| {
                    CallError::Http(ErrorInfo {
                        code: "INVALID_RESPONSE".into(),
                        message: "Unexpected response from the service".into(),
                        class: FailureClass::Server,
                        retryable: true,
                    })
                })?;
                if status != 202 && !parsed.is_processing() {
                    return Ok(parsed);
                }
                if started.elapsed() > Duration::from_secs(150) {
                    return Err(CallError::Http(ErrorInfo {
                        code: "PROVIDER_TIMEOUT".into(),
                        message: "The service is still processing; try again".into(),
                        class: FailureClass::Transport,
                        retryable: true,
                    }));
                }
                let Some(job_id) = parsed.job_id.clone() else {
                    return Err(CallError::Http(transport_error("Missing job id")));
                };
                tokio::time::sleep(Duration::from_millis(poll_delay_ms(
                    parsed.poll_after_ms,
                    attempt,
                )))
                .await;
                attempt += 1;
                if !still_current() {
                    return Err(CallError::Cancelled);
                }
                let resp = self
                    .http
                    .get(self.url(&format!("/v1/dictations/{job_id}")))
                    .bearer_auth(token)
                    .send()
                    .await
                    .map_err(|e| CallError::Http(transport_error(e.to_string())))?;
                (status, _, body) = {
                    let (s, r, b) = Self::read(resp).await;
                    (s, r, b)
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_rules() {
        assert_eq!(
            validate_base_url("https://api.example.com/").unwrap(),
            "https://api.example.com"
        );
        assert_eq!(
            validate_base_url(" https://api.example.com/scribiva ").unwrap(),
            "https://api.example.com/scribiva"
        );
        assert!(validate_base_url("http://127.0.0.1:8787").is_ok());
        assert!(validate_base_url("http://localhost:8787").is_ok());
        assert!(validate_base_url("http://[::1]:8787").is_ok());
        assert!(validate_base_url("http://api.example.com").is_err());
        assert!(validate_base_url("http://127.0.0.1.evil.com").is_err());
        assert!(validate_base_url("ftp://api.example.com").is_err());
        assert!(validate_base_url("https://user:pw@api.example.com").is_err());
        assert!(validate_base_url("https://api.example.com/?x=1").is_err());
        assert!(validate_base_url("https://api.example.com/#f").is_err());
        assert!(validate_base_url("").is_err());
        assert!(validate_base_url("https://exa mple.com").is_err());
        assert!(validate_base_url("javascript:alert(1)").is_err());
    }

    #[test]
    fn billing_hosts_are_strict() {
        assert!(is_trusted_billing_url(
            "https://checkout.stripe.com/c/pay/cs_test_1"
        ));
        assert!(is_trusted_billing_url(
            "https://billing.stripe.com/p/session/x"
        ));
        assert!(!is_trusted_billing_url("http://checkout.stripe.com/x"));
        assert!(!is_trusted_billing_url(
            "https://checkout.stripe.com.evil.com/x"
        ));
        assert!(!is_trusted_billing_url(
            "https://evil.com/checkout.stripe.com"
        ));
        assert!(!is_trusted_billing_url(
            "https://user@checkout.stripe.com/x"
        ));
        assert!(!is_trusted_billing_url(
            "https://checkout.stripe.com:8443/x"
        ));
        assert!(!is_trusted_billing_url("https://stripe.com/x"));
        assert!(!is_trusted_billing_url("not a url"));
    }

    #[test]
    fn verification_url_rules() {
        assert!(is_safe_verification_url(
            "https://scribiva.app/login?code=ABCD-EFGH"
        ));
        assert!(is_safe_verification_url(
            "http://127.0.0.1:8787/login?code=X"
        ));
        assert!(!is_safe_verification_url("http://scribiva.app/login"));
        assert!(!is_safe_verification_url("file:///C:/x"));
        assert!(!is_safe_verification_url("https://a:b@scribiva.app/login"));
    }

    #[test]
    fn token_format() {
        assert!(is_valid_token("scr_abcDEF123-_xyz"));
        assert!(!is_valid_token("scr_"));
        assert!(!is_valid_token("abc_abcdefghijkl"));
        assert!(!is_valid_token("scr_abc def ghij"));
        assert!(!is_valid_token(&format!("scr_{}", "a".repeat(300))));
    }

    #[test]
    fn device_poll_parsing() {
        let ok = r#"{"access_token":"scr_0123456789abcdef","token_type":"Bearer","expires_at":"2026-11-27T00:00:00Z"}"#;
        match parse_poll(200, ok) {
            PollStep::Token(t) => assert_eq!(t.access_token, "scr_0123456789abcdef"),
            other => panic!("{other:?}"),
        }
        let err = |code: &str| {
            format!(
                r#"{{"error":{{"code":"{code}","message":"m","retryable":true,"request_id":"r"}}}}"#
            )
        };
        assert_eq!(
            parse_poll(400, &err("AUTHORIZATION_PENDING")),
            PollStep::Pending
        );
        assert_eq!(parse_poll(400, &err("SLOW_DOWN")), PollStep::SlowDown);
        assert_eq!(parse_poll(400, &err("EXPIRED_TOKEN")), PollStep::Expired);
        assert_eq!(
            parse_poll(400, &err("INVALID_DEVICE_CODE")),
            PollStep::Failed("INVALID_DEVICE_CODE".into())
        );
        assert_eq!(parse_poll(500, "oops"), PollStep::Failed("HTTP_500".into()));
        // A 200 with a malformed token is not accepted.
        assert!(matches!(
            parse_poll(200, r#"{"access_token":"bad token"}"#),
            PollStep::Failed(_)
        ));
    }

    #[test]
    fn device_start_parsing() {
        let body = r#"{"device_code":"d","user_code":"ABCD-EFGH","verification_uri":"https://h/login","verification_uri_complete":"https://h/login?code=ABCD-EFGH","expires_in":900,"interval":3}"#;
        let s: DeviceStart = serde_json::from_str(body).unwrap();
        assert_eq!(s.user_code, "ABCD-EFGH");
        assert_eq!(s.interval, 3);
        let minimal: DeviceStart = serde_json::from_str(
            r#"{"device_code":"d","user_code":"U","verification_uri":"https://h/login"}"#,
        )
        .unwrap();
        assert_eq!((minimal.expires_in, minimal.interval), (900, 3));
        assert!(minimal.verification_uri_complete.is_none());
    }

    #[test]
    fn dictation_response_parsing() {
        let body = r#"{"job_id":"job_1","status":"completed","raw_text":"i has sent","corrected_text":"I sent.","output_text":"I sent.","detected_language":"en","grammar_mode":"standard","billing":{"plan":"free","charged_audio_seconds":12,"remaining_seconds":900,"resets_at":"2026-11-01T00:00:00Z"},"warnings":[],"suspected_hallucination":false,"processing":{"asr_ms":1}}"#;
        let r: DictationResponse = serde_json::from_str(body).unwrap();
        assert_eq!(r.final_text(), "I sent.");
        assert!(!r.is_processing());
        assert!(!r.is_suspected_noise());
        assert_eq!(r.billing.unwrap().remaining_seconds, Some(900));

        let noisy: DictationResponse = serde_json::from_str(
            r#"{"status":"completed","raw_text":"Thanks for watching","output_text":"Thanks for watching","suspected_hallucination":true}"#,
        )
        .unwrap();
        assert!(noisy.is_suspected_noise());
        let warned: DictationResponse = serde_json::from_str(
            r#"{"status":"completed","output_text":"x","warnings":["SUSPECTED_HALLUCINATION: phrase"]}"#,
        )
        .unwrap();
        assert!(warned.is_suspected_noise());

        let pending: DictationResponse =
            serde_json::from_str(r#"{"job_id":"job_1","status":"processing","poll_after_ms":500}"#)
                .unwrap();
        assert!(pending.is_processing());
        assert_eq!(pending.poll_after_ms, Some(500));

        let raw_only: DictationResponse =
            serde_json::from_str(r#"{"status":"completed","raw_text":"hi","output_text":null}"#)
                .unwrap();
        assert_eq!(raw_only.final_text(), "hi");
    }

    #[test]
    fn me_parsing() {
        let body = r#"{"user":{"id":"u1","email":"a@example.com"},"plan":"free","subscription":{"status":"inactive","current_period_end":null},"usage":{"period_start":"2026-10-01","used_seconds":100,"reserved_seconds":0,"limit_seconds":1800,"remaining_seconds":1700,"resets_at":"2026-11-01T00:00:00Z","grace_seconds":180,"plan":"free"},"limits":{},"billing_configured":true}"#;
        let me: Me = serde_json::from_str(body).unwrap();
        assert_eq!(me.usage.remaining_seconds, 1700);
        assert_eq!(me.usage.grace_seconds, Some(180));
        assert_eq!(me.usage.resets_at.as_deref(), Some("2026-11-01T00:00:00Z"));
        assert_eq!(me.user.email.as_deref(), Some("a@example.com"));
        // The serialised form passed to the UI carries no user id.
        assert!(!serde_json::to_string(&me).unwrap().contains("u1"));
    }

    fn err_body(code: &str) -> String {
        format!(
            r#"{{"error":{{"code":"{code}","message":"msg","retryable":false,"request_id":"req_1"}}}}"#
        )
    }

    #[test]
    fn error_mapping() {
        let e = map_dictation_error(401, &err_body("AUTH_REQUIRED"));
        assert_eq!(
            (e.class, e.code.as_str()),
            (FailureClass::Auth, "AUTH_REQUIRED")
        );
        let e = map_dictation_error(402, &err_body("QUOTA_EXCEEDED"));
        assert_eq!(e.class, FailureClass::Quota);
        assert!(!e.retryable);
        assert_eq!(
            map_dictation_error(403, &err_body("PLAN_INACTIVE")).class,
            FailureClass::Quota
        );
        let e = map_dictation_error(409, &err_body("BUSY"));
        assert_eq!((e.class, e.retryable), (FailureClass::Transport, true));
        let e = map_dictation_error(409, &err_body("IDEMPOTENCY_CONFLICT"));
        assert_eq!((e.class, e.retryable), (FailureClass::Server, true));
        let e = map_dictation_error(413, &err_body("AUDIO_TOO_LARGE"));
        assert!(!e.retryable);
        let e = map_dictation_error(422, &err_body("AUDIO_DURATION_INVALID"));
        assert!(!e.retryable);
        let e = map_dictation_error(400, &err_body("UNSUPPORTED_LANGUAGE"));
        assert_eq!((e.class, e.retryable), (FailureClass::Server, false));
        let e = map_dictation_error(429, &err_body("RATE_LIMITED"));
        assert_eq!(e.class, FailureClass::Transport);
        let e = map_dictation_error(502, &err_body("PROVIDER_ERROR"));
        assert_eq!((e.class, e.retryable), (FailureClass::Server, true));
        // Proxy 504 with an HTML body: job may still run, same key.
        let e = map_dictation_error(504, "<html>Gateway Timeout</html>");
        assert_eq!(
            (e.class, e.code.as_str()),
            (FailureClass::Transport, "HTTP_504")
        );
        let e = map_dictation_error(410, &err_body("RESULT_EXPIRED"));
        assert_eq!(e.class, FailureClass::Server);
        assert_eq!(e.message, "msg");
    }

    #[test]
    fn retry_and_poll_bounds() {
        assert_eq!(retry_after_seconds(Some("7")), 7);
        assert_eq!(retry_after_seconds(Some("99999")), 30);
        assert_eq!(retry_after_seconds(Some("soon")), 2);
        assert_eq!(retry_after_seconds(None), 2);
        assert_eq!(poll_delay_ms(Some(500), 0), 500);
        assert_eq!(poll_delay_ms(Some(500), 1), 1000);
        assert_eq!(poll_delay_ms(Some(500), 10), 5000);
        assert_eq!(poll_delay_ms(Some(1), 0), 200);
    }

    #[test]
    fn dictionary_cleaning() {
        let terms = vec![
            " Kubernetes ".into(),
            "kubernetes".into(),
            "".into(),
            "Scribiva".into(),
        ];
        assert_eq!(
            clean_terms(&terms, 200, 64).unwrap(),
            vec!["Kubernetes".to_string(), "Scribiva".to_string()]
        );
        assert!(clean_terms(&["a".repeat(65)], 200, 64).is_err());
        assert!(clean_terms(&["a\u{0007}".into()], 200, 64).is_err());
        let many: Vec<String> = (0..201).map(|i| format!("t{i}")).collect();
        assert!(clean_terms(&many, 200, 64).is_err());
        let d: Dictionary =
            serde_json::from_str(r#"{"terms":["A"],"max_terms":200,"max_term_length":64}"#)
                .unwrap();
        assert_eq!(d.terms, vec!["A"]);
    }
}
