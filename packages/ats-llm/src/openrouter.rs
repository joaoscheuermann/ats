//! OpenRouter (OpenAI-compatible) chat-completion client.
//!
//! Implements [`LlmClient`] against `<base_url>/chat/completions`, mapping
//! provider HTTP status codes + bodies to [`LlmError`] variants, driving the
//! locked transient-retry loop, and recording every attempt through a
//! [`ports::AuditSink`].
//!
//! The [`is_context_exceeded_error`] helper is the *only* place the
//! context-length detection lives, per AC-3.5″; it is covered by its own
//! unit tests so later efforts can change detection rules in one place.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::{header, Client, StatusCode};
use serde_json::{json, Value};
use time::format_description::well_known::Iso8601;
use time::OffsetDateTime;
use tracing::{info, warn};

use ats_core::config::{OpenRouterConfig, RetriesConfig};
use ats_core::ports::{
    AuditSink, ChatMessage, ChatRole, LlmCallRecord, LlmClient, LlmError, LlmRequest, LlmResponse,
    TokenUsage,
};

/// How much of a provider response body to attach to an [`LlmError::Other`] /
/// audit record. Keeps diagnostics readable while preserving enough context
/// to classify failure modes after the fact.
const MAX_BODY_SNIPPET: usize = 500;

/// Upper bound on a single OpenRouter HTTP attempt (send headers → receive
/// full body). Chosen generously to cover large-context models that stream
/// slowly while still converting a stalled provider into a classified
/// [`LlmError::Transient`] that the retry loop can react to, rather than a
/// silent indefinite hang. Not yet configurable — bump here if real traffic
/// ever needs more.
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// OpenAI-compatible chat completions client pointed at OpenRouter.
pub struct OpenRouterClient {
    http: Client,
    api_key: String,
    base_url: String,
    max_attempts: u32,
    backoff_ms: Vec<u64>,
    audit: Arc<dyn AuditSink>,
}

impl OpenRouterClient {
    /// Construct a client bound to `cfg.base_url`. `audit` receives one record
    /// per HTTP attempt (success *and* failure).
    ///
    /// Errors propagate from `reqwest::Client::builder` when TLS backend
    /// initialisation fails — which in practice is never on the supported
    /// platforms, but we surface it rather than panic.
    pub fn new(
        cfg: &OpenRouterConfig,
        retries: &RetriesConfig,
        audit: Arc<dyn AuditSink>,
    ) -> reqwest::Result<Self> {
        let http = Client::builder()
            .user_agent("ats-resume-optimizer/0.1")
            .timeout(HTTP_REQUEST_TIMEOUT)
            .build()?;
        Ok(Self {
            http,
            api_key: cfg.api_key.clone(),
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            max_attempts: retries.llm_transient_max_attempts.max(1),
            backoff_ms: retries.llm_transient_backoff_ms.clone(),
            audit,
        })
    }

    /// Number of attempts the transient loop will make. Exposed for tests.
    pub fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    async fn attempt(
        &self,
        req: &LlmRequest,
        body: &Value,
        attempt: u32,
    ) -> Result<LlmResponse, LlmError> {
        let url = format!("{}/chat/completions", self.base_url);
        info!(
            target: "ats::llm",
            stage = req.stage,
            model = %req.model,
            attempt,
            has_response_format = req.response_format.is_some(),
            timeout_ms = HTTP_REQUEST_TIMEOUT.as_millis() as u64,
            "llm.attempt.start"
        );
        let started = Instant::now();
        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .header(header::CONTENT_TYPE, "application/json")
            .header("HTTP-Referer", "https://github.com/atsresumeoptimizer")
            .header("X-Title", "ats-resume-optimizer")
            .json(body)
            .send()
            .await
            .map_err(|err| {
                let classified = map_reqwest_error(err);
                warn!(
                    target: "ats::llm",
                    stage = req.stage,
                    attempt,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    error = %classified,
                    "llm.attempt.network_error"
                );
                classified
            })?;

        let status = response.status();
        let bytes = response.bytes().await.map_err(|err| {
            let classified = map_reqwest_error(err);
            warn!(
                target: "ats::llm",
                stage = req.stage,
                attempt,
                elapsed_ms = started.elapsed().as_millis() as u64,
                error = %classified,
                "llm.attempt.read_error"
            );
            classified
        })?;
        let body_text = String::from_utf8_lossy(&bytes).to_string();

        if status.is_success() {
            let raw: Value = serde_json::from_str(&body_text).map_err(|_| {
                LlmError::Other(format!(
                    "non-json response: {}",
                    snippet(&body_text, MAX_BODY_SNIPPET)
                ))
            })?;
            let result = parse_success(&raw).ok_or_else(|| {
                LlmError::Other(format!(
                    "unexpected response shape: {}",
                    snippet(&body_text, MAX_BODY_SNIPPET)
                ))
            });
            if let Ok(ref resp) = result {
                info!(
                    target: "ats::llm",
                    stage = req.stage,
                    attempt,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    status = status.as_u16(),
                    prompt_tokens = resp.usage.prompt,
                    completion_tokens = resp.usage.completion,
                    total_tokens = resp.usage.total,
                    "llm.attempt.ok"
                );
            }
            return result;
        }

        let err = classify_status(status, &body_text, &req.model);
        warn!(
            target: "ats::llm",
            stage = req.stage,
            attempt,
            elapsed_ms = started.elapsed().as_millis() as u64,
            status = status.as_u16(),
            outcome = err.outcome_tag(),
            "llm.attempt.http_error"
        );
        Err(err)
    }

    fn audit_attempt(
        &self,
        req: &LlmRequest,
        attempt: u32,
        outcome: &str,
        response_text: &str,
        usage: TokenUsage,
    ) {
        let record = LlmCallRecord {
            timestamp: format_now(),
            stage: req.stage.to_string(),
            model: req.model.clone(),
            temperature: req.temperature,
            seed: req.seed,
            prompt: serialise_messages(&req.messages),
            response: response_text.to_string(),
            usage,
            attempt,
            outcome: outcome.to_string(),
        };
        if let Err(err) = self.audit.record(&record) {
            tracing::warn!(target: "ats::llm", %err, "audit sink failed to write record");
        }
    }

    fn sleep_for(&self, idx: usize) -> Duration {
        let ms = self
            .backoff_ms
            .get(idx)
            .copied()
            .or_else(|| self.backoff_ms.last().copied())
            .unwrap_or(0);
        Duration::from_millis(ms)
    }
}

#[async_trait]
impl LlmClient for OpenRouterClient {
    async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
        let body = build_body(&req);
        let mut last_err: Option<LlmError> = None;

        for attempt in 1..=self.max_attempts {
            match self.attempt(&req, &body, attempt).await {
                Ok(resp) => {
                    self.audit_attempt(
                        &req,
                        attempt,
                        "ok",
                        &resp.content,
                        resp.usage,
                    );
                    return Ok(resp);
                }
                Err(err) => {
                    self.audit_attempt(
                        &req,
                        attempt,
                        err.outcome_tag(),
                        &err.to_string(),
                        TokenUsage::ZERO,
                    );
                    if !matches!(err, LlmError::Transient(_)) {
                        return Err(err);
                    }
                    last_err = Some(err);
                    if attempt < self.max_attempts {
                        let delay = self.sleep_for((attempt - 1) as usize);
                        info!(
                            target: "ats::llm",
                            stage = req.stage,
                            attempt,
                            next_attempt = attempt + 1,
                            backoff_ms = delay.as_millis() as u64,
                            "llm.retry.sleep"
                        );
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        Err(last_err.unwrap_or_else(|| LlmError::Other("retry loop exited without error".into())))
    }
}

/// Serialise an [`LlmRequest`] into an OpenAI-compatible chat completions
/// body. Omits `seed` / `response_format` when absent.
fn build_body(req: &LlmRequest) -> Value {
    let messages: Vec<_> = req.messages.iter().map(serialise_message).collect();
    let mut body = json!({
        "model": req.model,
        "temperature": req.temperature,
        "messages": messages,
    });
    if let Some(seed) = req.seed {
        body["seed"] = json!(seed);
    }
    if let Some(fmt) = &req.response_format {
        body["response_format"] = fmt.clone();
    }
    body
}

fn serialise_message(msg: &ChatMessage) -> Value {
    json!({
        "role": match msg.role {
            ChatRole::System => "system",
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
        },
        "content": msg.content,
    })
}

fn serialise_messages(messages: &[ChatMessage]) -> String {
    let arr: Vec<Value> = messages.iter().map(serialise_message).collect();
    serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into())
}

fn parse_success(body: &Value) -> Option<LlmResponse> {
    let content = body
        .get("choices")?
        .get(0)?
        .get("message")?
        .get("content")?
        .as_str()?
        .to_string();
    let usage = body
        .get("usage")
        .map(parse_usage)
        .unwrap_or_default();
    Some(LlmResponse {
        content,
        usage,
        raw: body.clone(),
    })
}

fn parse_usage(v: &Value) -> TokenUsage {
    let get = |key: &str| -> u64 {
        v.get(key)
            .and_then(|n| n.as_u64())
            .unwrap_or(0)
    };
    let prompt = get("prompt_tokens");
    let completion = get("completion_tokens");
    let total = v
        .get("total_tokens")
        .and_then(|n| n.as_u64())
        .unwrap_or(prompt + completion);
    TokenUsage {
        prompt,
        completion,
        total,
    }
}

fn map_reqwest_error(err: reqwest::Error) -> LlmError {
    if err.is_timeout() || err.is_connect() || err.is_request() {
        LlmError::Transient(format!("network: {err}"))
    } else {
        LlmError::Other(format!("reqwest: {err}"))
    }
}

/// Map an HTTP status + response body to the right [`LlmError`] variant.
fn classify_status(status: StatusCode, body: &str, model: &str) -> LlmError {
    if status == StatusCode::TOO_MANY_REQUESTS {
        return LlmError::Transient(format!(
            "rate limited ({status}): {}",
            snippet(body, MAX_BODY_SNIPPET)
        ));
    }
    if status.is_server_error() {
        return LlmError::Transient(format!("server error {status}"));
    }
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return LlmError::Auth(format!(
            "{status}: {}",
            snippet(body, MAX_BODY_SNIPPET)
        ));
    }
    if status == StatusCode::BAD_REQUEST && is_context_exceeded_error(status, body) {
        let prompt_tokens = extract_prompt_tokens(body);
        return LlmError::ContextExceeded {
            model: model.to_string(),
            message: snippet(body, MAX_BODY_SNIPPET),
            prompt_tokens,
        };
    }
    LlmError::Other(format!(
        "{status}: {}",
        snippet(body, MAX_BODY_SNIPPET)
    ))
}

/// Detect an OpenAI / OpenRouter "context length exceeded" 400 response.
/// Matches any of:
/// - OpenAI-style error code `context_length_exceeded`.
/// - Body mentions `"context length"` (case-insensitive).
/// - Body mentions `"maximum context"` (case-insensitive).
/// - Body mentions `"context window"` (case-insensitive).
pub fn is_context_exceeded_error(status: StatusCode, body: &str) -> bool {
    if status != StatusCode::BAD_REQUEST {
        return false;
    }
    if body.contains("context_length_exceeded") {
        return true;
    }
    let lowered = body.to_ascii_lowercase();
    lowered.contains("context length")
        || lowered.contains("maximum context")
        || lowered.contains("context window")
}

fn extract_prompt_tokens(body: &str) -> Option<u32> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    let direct = parsed
        .get("error")
        .and_then(|e| e.get("prompt_tokens"))
        .and_then(|n| n.as_u64());
    if let Some(n) = direct {
        return u32::try_from(n).ok();
    }
    let usage = parsed
        .get("usage")
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(|n| n.as_u64())?;
    u32::try_from(usage).ok()
}

fn snippet(body: &str, max: usize) -> String {
    if body.chars().count() <= max {
        return body.to_string();
    }
    let truncated: String = body.chars().take(max).collect();
    format!("{truncated}…")
}

fn format_now() -> String {
    OffsetDateTime::now_utc()
        .format(&Iso8601::DEFAULT)
        .unwrap_or_else(|_| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ats_core::ports::{ChatMessage, ChatRole};
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemSink {
        records: Mutex<Vec<LlmCallRecord>>,
    }

    impl AuditSink for MemSink {
        fn record(&self, call: &LlmCallRecord) -> std::io::Result<()> {
            self.records.lock().unwrap().push(call.clone());
            Ok(())
        }
    }

    fn sample_request() -> LlmRequest {
        LlmRequest {
            stage: "keywords",
            model: "x/y".into(),
            temperature: 0.0,
            seed: Some(42),
            messages: vec![
                ChatMessage {
                    role: ChatRole::System,
                    content: "sys".into(),
                },
                ChatMessage {
                    role: ChatRole::User,
                    content: "hello".into(),
                },
            ],
            response_format: Some(json!({"type": "json_schema"})),
        }
    }

    #[test]
    fn build_body_includes_seed_and_response_format() {
        let body = build_body(&sample_request());
        assert_eq!(body["model"], "x/y");
        assert_eq!(body["seed"], 42);
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["messages"][0]["role"], "system");
    }

    #[test]
    fn build_body_omits_optional_fields_when_absent() {
        let mut req = sample_request();
        req.seed = None;
        req.response_format = None;
        let body = build_body(&req);
        assert!(body.get("seed").is_none(), "seed should be omitted");
        assert!(
            body.get("response_format").is_none(),
            "response_format should be omitted"
        );
    }

    #[test]
    fn parse_success_extracts_content_and_usage() {
        let raw = json!({
            "choices": [{
                "message": {"role": "assistant", "content": "hi"}
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });
        let resp = parse_success(&raw).unwrap();
        assert_eq!(resp.content, "hi");
        assert_eq!(resp.usage.total, 15);
    }

    #[test]
    fn parse_success_derives_total_when_missing() {
        let raw = json!({
            "choices": [{"message": {"content": "ok"}}],
            "usage": {"prompt_tokens": 7, "completion_tokens": 3}
        });
        let resp = parse_success(&raw).unwrap();
        assert_eq!(resp.usage.total, 10);
    }

    #[test]
    fn classify_429_is_transient() {
        let err = classify_status(
            StatusCode::TOO_MANY_REQUESTS,
            "rate limit reached",
            "x/y",
        );
        assert!(matches!(err, LlmError::Transient(_)));
    }

    #[test]
    fn classify_500_series_is_transient() {
        for code in [500u16, 502, 503, 504] {
            let err = classify_status(
                StatusCode::from_u16(code).unwrap(),
                "boom",
                "x/y",
            );
            assert!(
                matches!(err, LlmError::Transient(_)),
                "status {code} should be transient, got {err:?}"
            );
        }
    }

    #[test]
    fn classify_401_and_403_is_auth() {
        for code in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
            let err = classify_status(code, "nope", "x/y");
            assert!(matches!(err, LlmError::Auth(_)));
        }
    }

    #[test]
    fn classify_400_context_exceeded_matches_openai_format() {
        let body = r#"{"error":{"message":"too long","code":"context_length_exceeded"}}"#;
        let err = classify_status(StatusCode::BAD_REQUEST, body, "x/y");
        match err {
            LlmError::ContextExceeded { model, .. } => assert_eq!(model, "x/y"),
            other => panic!("expected ContextExceeded, got {other:?}"),
        }
    }

    #[test]
    fn classify_400_context_exceeded_matches_openrouter_plain_text() {
        let body = "This model's maximum context length is 4096 tokens.";
        let err = classify_status(StatusCode::BAD_REQUEST, body, "x/y");
        assert!(matches!(err, LlmError::ContextExceeded { .. }));
    }

    #[test]
    fn classify_400_unrelated_is_other() {
        let err = classify_status(
            StatusCode::BAD_REQUEST,
            "bad parameter: foo",
            "x/y",
        );
        assert!(matches!(err, LlmError::Other(_)));
    }

    #[test]
    fn classify_404_is_other() {
        let err = classify_status(StatusCode::NOT_FOUND, "nope", "x/y");
        assert!(matches!(err, LlmError::Other(_)));
    }

    #[test]
    fn is_context_exceeded_detects_all_patterns() {
        let status = StatusCode::BAD_REQUEST;
        assert!(is_context_exceeded_error(
            status,
            "error code context_length_exceeded happened"
        ));
        assert!(is_context_exceeded_error(
            status,
            "maximum context length is 4096"
        ));
        assert!(is_context_exceeded_error(
            status,
            "Context Window exceeded"
        ));
        assert!(!is_context_exceeded_error(status, "bad argument: foo"));
        assert!(!is_context_exceeded_error(
            StatusCode::OK,
            "context length"
        ));
    }

    #[test]
    fn extract_prompt_tokens_reads_error_field() {
        let body = r#"{"error":{"message":"too long","prompt_tokens":9001}}"#;
        assert_eq!(extract_prompt_tokens(body), Some(9001));
    }

    #[test]
    fn extract_prompt_tokens_falls_back_to_usage_field() {
        let body = r#"{"error":{"message":"x"},"usage":{"prompt_tokens":1234}}"#;
        assert_eq!(extract_prompt_tokens(body), Some(1234));
    }

    #[test]
    fn extract_prompt_tokens_returns_none_when_absent() {
        assert_eq!(extract_prompt_tokens("{}"), None);
        assert_eq!(extract_prompt_tokens("not json"), None);
    }

    #[test]
    fn snippet_truncates_long_bodies() {
        let long: String = "a".repeat(MAX_BODY_SNIPPET + 50);
        let s = snippet(&long, MAX_BODY_SNIPPET);
        assert!(s.chars().count() <= MAX_BODY_SNIPPET + 1, "got {}", s.len());
        assert!(s.ends_with('…'));
    }

    #[test]
    fn snippet_passes_short_bodies_through() {
        assert_eq!(snippet("hi", MAX_BODY_SNIPPET), "hi");
    }

    fn sample_config() -> (OpenRouterConfig, RetriesConfig) {
        let cfg = OpenRouterConfig {
            api_key: "k".into(),
            base_url: "http://127.0.0.1".into(),
        };
        let retries = RetriesConfig {
            llm_transient_max_attempts: 3,
            llm_transient_backoff_ms: vec![0, 0, 0],
            schema_validation_max_attempts: 3,
        };
        (cfg, retries)
    }

    #[tokio::test]
    async fn transient_then_success_records_three_attempts() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let body_ok = json!({
            "choices": [{"message": {"role": "assistant", "content": "hello"}}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });
        // First attempt → 429
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        // Second attempt → 500
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("server error"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        // Third attempt → 200
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body_ok))
            .mount(&server)
            .await;

        let (mut cfg, retries) = sample_config();
        cfg.base_url = server.uri();
        let audit = Arc::new(MemSink::default());
        let client =
            OpenRouterClient::new(&cfg, &retries, audit.clone() as Arc<dyn AuditSink>).unwrap();

        let resp = client.complete(sample_request()).await.unwrap();
        assert_eq!(resp.content, "hello");
        assert_eq!(resp.usage.total, 15);

        let records = audit.records.lock().unwrap().clone();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].outcome, "transient");
        assert_eq!(records[1].outcome, "transient");
        assert_eq!(records[2].outcome, "ok");
        assert_eq!(records[2].usage.total, 15);
    }

    #[tokio::test]
    async fn exhausts_retries_returns_transient_error() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;

        let (mut cfg, mut retries) = sample_config();
        cfg.base_url = server.uri();
        retries.llm_transient_max_attempts = 5;
        retries.llm_transient_backoff_ms = vec![0, 0, 0, 0, 0];
        let audit = Arc::new(MemSink::default());
        let client =
            OpenRouterClient::new(&cfg, &retries, audit.clone() as Arc<dyn AuditSink>).unwrap();

        let err = client.complete(sample_request()).await.unwrap_err();
        assert!(matches!(err, LlmError::Transient(_)), "got {err:?}");
        assert_eq!(audit.records.lock().unwrap().len(), 5);
        let outcomes: Vec<String> = audit
            .records
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.outcome.clone())
            .collect();
        assert!(outcomes.iter().all(|o| o == "transient"));
    }

    #[tokio::test]
    async fn auth_fails_fast_records_one_attempt() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_string("invalid key"))
            .mount(&server)
            .await;

        let (mut cfg, retries) = sample_config();
        cfg.base_url = server.uri();
        let audit = Arc::new(MemSink::default());
        let client =
            OpenRouterClient::new(&cfg, &retries, audit.clone() as Arc<dyn AuditSink>).unwrap();

        let err = client.complete(sample_request()).await.unwrap_err();
        assert!(matches!(err, LlmError::Auth(_)), "got {err:?}");
        assert_eq!(audit.records.lock().unwrap().len(), 1);
        assert_eq!(audit.records.lock().unwrap()[0].outcome, "auth");
    }

    #[tokio::test]
    async fn context_exceeded_fails_fast() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let body = r#"{"error":{"message":"Maximum context length is 4096 tokens","code":"context_length_exceeded"}}"#;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(400).set_body_string(body))
            .mount(&server)
            .await;

        let (mut cfg, retries) = sample_config();
        cfg.base_url = server.uri();
        let audit = Arc::new(MemSink::default());
        let client =
            OpenRouterClient::new(&cfg, &retries, audit.clone() as Arc<dyn AuditSink>).unwrap();

        let err = client.complete(sample_request()).await.unwrap_err();
        match err {
            LlmError::ContextExceeded { model, .. } => assert_eq!(model, "x/y"),
            other => panic!("expected ContextExceeded, got {other:?}"),
        }
        assert_eq!(audit.records.lock().unwrap().len(), 1);
        assert_eq!(
            audit.records.lock().unwrap()[0].outcome,
            "context-exceeded"
        );
    }

    #[tokio::test]
    async fn non_json_success_body_becomes_other() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not-json"))
            .mount(&server)
            .await;

        let (mut cfg, retries) = sample_config();
        cfg.base_url = server.uri();
        let audit = Arc::new(MemSink::default());
        let client =
            OpenRouterClient::new(&cfg, &retries, audit.clone() as Arc<dyn AuditSink>).unwrap();

        let err = client.complete(sample_request()).await.unwrap_err();
        assert!(matches!(err, LlmError::Other(_)), "got {err:?}");
        assert_eq!(audit.records.lock().unwrap().len(), 1);
        assert_eq!(audit.records.lock().unwrap()[0].outcome, "other");
    }

}
