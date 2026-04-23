//! US-2 scrape stage: fetch rendered HTML and convert it to a `JobPosting`
//! via a single LLM call.
//!
//! The stage is split into two port-driven steps:
//!
//! 1. [`PageScraper::fetch_html`] — the adapter (`ats-scrape`'s
//!    `ChromiumScraper` in production) returns the rendered HTML or a
//!    classified [`ScrapeError`].
//! 2. [`LlmClient::complete`] — convert the HTML to structured JSON using
//!    the locked `PROMPT_SCRAPE_TO_MARKDOWN` system prompt.
//!
//! No schema-retry loop (US-3 keeps that pattern). A non-JSON reply collapses
//! into [`AtsError::Other`] (exit 1) — the raw response body has already
//! been recorded by the client's audit sink (NFC-19).

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::Value;
use tracing::info;

use crate::assets::{PROMPT_SCRAPE_TO_MARKDOWN, SCRAPE_RESPONSE_FORMAT};
use crate::config::ModelStageConfig;
use crate::domain::JobPosting;
use crate::error::{AtsError, ScrapeClass};
use crate::ports::{ChatMessage, ChatRole, LlmClient, LlmRequest, TokenUsage};
use crate::scrape_port::{PageScraper, ScrapeError};

/// Stage output: the `JobPosting` plus the LLM token usage of this call so
/// the caller can aggregate it into `run.json`'s `token_usage_total` field.
#[derive(Debug, Clone)]
pub struct ScrapeStageOutcome {
    pub posting: JobPosting,
    pub usage: TokenUsage,
}

/// Per-stage knobs that come from `config.json` (`scrape` + `models`).
#[derive(Debug, Clone)]
pub struct ScrapeStageConfig {
    pub idle_timeout: Duration,
    pub model: ModelStageConfig,
}

/// Shape expected from the LLM — mirrors the JSON contract in
/// `prompts/scrape_to_markdown.md`.
#[derive(Debug, Deserialize)]
struct ScrapeLlmOutput {
    #[serde(default)]
    title: String,
    markdown: String,
}

/// Run the scrape stage end-to-end.
///
/// All dependencies are passed as trait objects so tests can inject fakes
/// without touching the network.
pub async fn fetch_and_convert(
    scraper: &dyn PageScraper,
    llm: &dyn LlmClient,
    url: &str,
    cfg: &ScrapeStageConfig,
) -> Result<ScrapeStageOutcome, AtsError> {
    let html = scraper
        .fetch_html(url, cfg.idle_timeout)
        .await
        .map_err(scrape_error_to_ats)?;

    let messages = vec![
        ChatMessage {
            role: ChatRole::System,
            content: PROMPT_SCRAPE_TO_MARKDOWN.to_string(),
        },
        ChatMessage {
            role: ChatRole::User,
            content: html,
        },
    ];

    let request = LlmRequest {
        stage: "scrape",
        model: cfg.model.name.clone(),
        temperature: cfg.model.temperature,
        seed: Some(cfg.model.seed),
        messages,
        response_format: Some(scrape_response_format().clone()),
    };

    info!(
        target: "ats::stage::scrape",
        model = %cfg.model.name,
        "scrape.llm.start"
    );
    let llm_started = Instant::now();
    let response = llm
        .complete(request)
        .await
        .map_err(|err| AtsError::Llm(err.class()))?;

    info!(
        target: "ats::stage::scrape",
        model = %cfg.model.name,
        elapsed_ms = llm_started.elapsed().as_millis() as u64,
        prompt_tokens = response.usage.prompt,
        completion_tokens = response.usage.completion,
        total_tokens = response.usage.total,
        "scrape.llm.ok"
    );

    let parsed = parse_llm_output(&response.content)?;
    Ok(ScrapeStageOutcome {
        posting: JobPosting {
            title: parsed.title,
            markdown: parsed.markdown,
        },
        usage: response.usage,
    })
}

fn scrape_error_to_ats(err: ScrapeError) -> AtsError {
    AtsError::Scrape(ScrapeClass::from(err))
}

fn parse_llm_output(content: &str) -> Result<ScrapeLlmOutput, AtsError> {
    let trimmed = content.trim();

    if trimmed.is_empty() {
        return Err(AtsError::Other(
            "scrape-to-markdown LLM returned empty content".into(),
        ));
    }
    serde_json::from_str::<ScrapeLlmOutput>(trimmed).map_err(|err| {
        AtsError::Other(format!(
            "scrape-to-markdown LLM returned non-JSON content: {err}"
        ))
    })
}

/// Parsed `SCRAPE_RESPONSE_FORMAT` JSON (the whole `{type, json_schema:{...}}`
/// envelope), cached across calls. Forwarded verbatim as the OpenAI-compatible
/// `response_format` for every scrape LLM request so providers that honour
/// structured outputs (with `strict: true`) cannot return malformed JSON.
fn scrape_response_format() -> &'static Value {
    static CACHED: OnceLock<Value> = OnceLock::new();
    CACHED.get_or_init(|| {
        serde_json::from_str::<Value>(SCRAPE_RESPONSE_FORMAT)
            .expect("embedded job_posting_extraction.json must be valid JSON")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{LlmError, LlmResponse};
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct FakeScraper {
        html: Mutex<Option<Result<String, ScrapeError>>>,
    }

    impl FakeScraper {
        fn ok(html: impl Into<String>) -> Self {
            Self {
                html: Mutex::new(Some(Ok(html.into()))),
            }
        }
        fn err(err: ScrapeError) -> Self {
            Self {
                html: Mutex::new(Some(Err(err))),
            }
        }
    }

    #[async_trait]
    impl PageScraper for FakeScraper {
        async fn fetch_html(
            &self,
            _url: &str,
            _idle_timeout: Duration,
        ) -> Result<String, ScrapeError> {
            self.html
                .lock()
                .unwrap()
                .take()
                .expect("fake scraper called twice")
        }
    }

    struct ScriptedLlm {
        next: Mutex<Option<Result<LlmResponse, LlmError>>>,
        last_request: Mutex<Option<LlmRequest>>,
    }

    impl ScriptedLlm {
        fn ok(content: &str) -> Self {
            Self {
                next: Mutex::new(Some(Ok(LlmResponse {
                    content: content.to_string(),
                    usage: TokenUsage {
                        prompt: 42,
                        completion: 10,
                        total: 52,
                    },
                    raw: serde_json::json!({}),
                }))),
                last_request: Mutex::new(None),
            }
        }
        fn err(err: LlmError) -> Self {
            Self {
                next: Mutex::new(Some(Err(err))),
                last_request: Mutex::new(None),
            }
        }
    }

    #[async_trait]
    impl LlmClient for ScriptedLlm {
        async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
            *self.last_request.lock().unwrap() = Some(req);
            self.next
                .lock()
                .unwrap()
                .take()
                .expect("scripted llm called twice")
        }
    }

    fn config() -> ScrapeStageConfig {
        ScrapeStageConfig {
            idle_timeout: Duration::from_millis(100),
            model: ModelStageConfig {
                name: "openai/test".into(),
                temperature: 0.0,
                seed: 42,
            },
        }
    }

    #[tokio::test]
    async fn happy_path_returns_job_posting() {
        let scraper = FakeScraper::ok("<html><body>hi</body></html>");
        let llm = ScriptedLlm::ok(
            "{\"title\":\"Senior Rust Engineer\",\"markdown\":\"## About\\n- do things\"}",
        );
        let outcome = fetch_and_convert(&scraper, &llm, "https://example.test", &config())
            .await
            .unwrap();
        assert_eq!(outcome.posting.title, "Senior Rust Engineer");
        assert!(outcome.posting.markdown.contains("## About"));
        assert_eq!(outcome.usage.total, 52);
    }

    #[tokio::test]
    async fn request_carries_structured_output_response_format() {
        let scraper = FakeScraper::ok("<html/>");
        let llm = ScriptedLlm::ok(
            "{\"title\":\"t\",\"markdown\":\"m\"}",
        );
        fetch_and_convert(&scraper, &llm, "https://example.test", &config())
            .await
            .unwrap();

        let sent = llm
            .last_request
            .lock()
            .unwrap()
            .clone()
            .expect("llm.complete was not called");
        let fmt = sent
            .response_format
            .expect("scrape stage must set response_format (structured outputs)");
        assert_eq!(fmt["type"], "json_schema");
        assert_eq!(fmt["json_schema"]["strict"], true);
        assert_eq!(fmt["json_schema"]["name"], "job_posting_extraction");
        let props = &fmt["json_schema"]["schema"]["properties"];
        assert!(props.get("title").is_some());
        assert!(props.get("markdown").is_some());
    }

    #[tokio::test]
    async fn scrape_offline_propagates_as_ats_scrape_offline() {
        let scraper = FakeScraper::err(ScrapeError::Offline("dns fail".into()));
        let llm = ScriptedLlm::ok("unused");
        let err = fetch_and_convert(&scraper, &llm, "https://example.test", &config())
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 4);
        match err {
            AtsError::Scrape(ScrapeClass::Offline) => {}
            other => panic!("expected Scrape(Offline), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn scrape_not_found_maps_to_not_found_class() {
        let scraper = FakeScraper::err(ScrapeError::NotFound { status: 404 });
        let llm = ScriptedLlm::ok("unused");
        let err = fetch_and_convert(&scraper, &llm, "https://example.test", &config())
            .await
            .unwrap_err();
        match err {
            AtsError::Scrape(ScrapeClass::NotFound) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn scrape_http_preserves_status_code() {
        let scraper = FakeScraper::err(ScrapeError::Http { status: 418 });
        let llm = ScriptedLlm::ok("unused");
        let err = fetch_and_convert(&scraper, &llm, "https://example.test", &config())
            .await
            .unwrap_err();
        match err {
            AtsError::Scrape(ScrapeClass::Http(418)) => {}
            other => panic!("expected Http(418), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn scrape_browser_missing_surfaces_as_scrape_class() {
        let scraper = FakeScraper::err(ScrapeError::BrowserMissing("no Chrome".into()));
        let llm = ScriptedLlm::ok("unused");
        let err = fetch_and_convert(&scraper, &llm, "https://example.test", &config())
            .await
            .unwrap_err();
        match err {
            AtsError::Scrape(ScrapeClass::BrowserMissing(msg)) => {
                assert!(msg.contains("no Chrome"))
            }
            other => panic!("expected BrowserMissing, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn llm_auth_error_becomes_ats_llm() {
        let scraper = FakeScraper::ok("<html/>");
        let llm = ScriptedLlm::err(LlmError::Auth("401".into()));
        let err = fetch_and_convert(&scraper, &llm, "https://example.test", &config())
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 5);
    }

    #[tokio::test]
    async fn non_json_llm_response_is_other_error() {
        let scraper = FakeScraper::ok("<html/>");
        let llm = ScriptedLlm::ok("not json at all");
        let err = fetch_and_convert(&scraper, &llm, "https://example.test", &config())
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 1);
        let msg = err.to_string();
        assert!(msg.contains("non-JSON"), "got: {msg}");
    }

    #[tokio::test]
    async fn missing_markdown_field_is_other_error() {
        let scraper = FakeScraper::ok("<html/>");
        let llm = ScriptedLlm::ok(r#"{"title":"only title"}"#);
        let err = fetch_and_convert(&scraper, &llm, "https://example.test", &config())
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 1);
    }

    #[tokio::test]
    async fn absent_title_defaults_to_empty_string() {
        let scraper = FakeScraper::ok("<html/>");
        let llm = ScriptedLlm::ok(r#"{"markdown":"body"}"#);
        let outcome = fetch_and_convert(&scraper, &llm, "https://example.test", &config())
            .await
            .unwrap();
        assert_eq!(outcome.posting.title, "");
        assert_eq!(outcome.posting.markdown, "body");
    }
}
