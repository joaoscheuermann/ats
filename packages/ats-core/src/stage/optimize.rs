//! US-4 resume optimization — one LLM call with the locked system prompt, plain
//! Markdown in the assistant response.

use crate::assets::PROMPT_RESUME_OPTIMIZATION;
use crate::config::ModelStageConfig;
use crate::error::AtsError;
use crate::ports::{AuditSink, ChatMessage, ChatRole, LlmClient, LlmRequest, TokenUsage};

/// Successful optimization: rendered Markdown and token usage for that call.
#[derive(Debug, Clone)]
pub struct OptimizeOutcome {
    pub markdown: String,
    pub usage: TokenUsage,
}

/// Build the optimizer [`LlmRequest`], call [`LlmClient::complete`] once, and
/// return the assistant Markdown. Transient retry policy lives in the client.
pub async fn run(
    llm: &dyn LlmClient,
    _audit: &dyn AuditSink,
    baseline_md: &str,
    keywords_md: &str,
    model: &ModelStageConfig,
) -> Result<OptimizeOutcome, AtsError> {
    let user = format!("=== RESUME ===\n{baseline_md}\n\n=== KEYWORDS ===\n{keywords_md}\n");
    let request = LlmRequest {
        stage: "optimize",
        model: model.name.clone(),
        temperature: model.temperature,
        seed: Some(model.seed),
        messages: vec![
            ChatMessage {
                role: ChatRole::System,
                content: PROMPT_RESUME_OPTIMIZATION.to_string(),
            },
            ChatMessage {
                role: ChatRole::User,
                content: user,
            },
        ],
        response_format: None,
    };

    let response = llm
        .complete(request)
        .await
        .map_err(|err| AtsError::Llm(err.class()))?;

    Ok(OptimizeOutcome {
        markdown: response.content,
        usage: response.usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::io;
    use std::sync::Mutex;

    use serde_json::Value;

    use crate::domain::KeywordSet;
    use crate::ports::{LlmError, LlmResponse};
    use crate::stage::keywords::{parse_keywords_from_value, to_markdown};

    struct OneShotLlm {
        content: String,
        usage: TokenUsage,
        last_req: Mutex<Option<LlmRequest>>,
    }

    impl OneShotLlm {
        fn new(content: String) -> Self {
            Self {
                content,
                usage: TokenUsage {
                    prompt: 10,
                    completion: 20,
                    total: 30,
                },
                last_req: Mutex::new(None),
            }
        }
    }

    #[async_trait]
    impl LlmClient for OneShotLlm {
        async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
            *self.last_req.lock().unwrap() = Some(req);
            Ok(LlmResponse {
                content: self.content.clone(),
                usage: self.usage,
                raw: serde_json::json!({}),
            })
        }
    }

    struct NopAudit;

    impl crate::ports::AuditSink for NopAudit {
        fn record(&self, _call: &crate::ports::LlmCallRecord) -> io::Result<()> {
            Ok(())
        }
    }

    fn model() -> ModelStageConfig {
        ModelStageConfig {
            name: "m/x".into(),
            temperature: 0.1,
            seed: 1,
        }
    }

    fn load_fixture_json() -> KeywordSet {
        let raw = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/optimize/keywords.json"
        ));
        let v: Value = serde_json::from_str(raw).unwrap();
        parse_keywords_from_value(&v).expect("valid fixture keywords")
    }

    #[tokio::test]
    async fn runs_happy_path_with_canned_output() {
        let set = load_fixture_json();
        let keywords_md = to_markdown(&set);
        let baseline = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/optimize/baseline.md"
        ));
        let canned = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/optimize/optimized-low.md"
        ));
        let llm = OneShotLlm::new(canned.to_string());
        let audit = NopAudit;
        let outcome = run(
            &llm,
            &audit,
            baseline,
            &keywords_md,
            &model(),
        )
        .await
        .expect("optimize succeeds");
        assert!(!outcome.markdown.is_empty());
        assert_eq!(outcome.usage.total, 30);
        let req = llm.last_req.lock().unwrap().clone().expect("one request");
        assert_eq!(req.stage, "optimize");
        assert!(req.response_format.is_none());
        assert_eq!(req.model, "m/x");
        assert_eq!(
            req.messages[0].content.as_str(),
            crate::assets::PROMPT_RESUME_OPTIMIZATION
        );
        let user = &req.messages[1].content;
        assert!(user.contains("=== RESUME ==="));
        assert!(user.contains("=== KEYWORDS ==="));
    }
}
