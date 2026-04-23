//! US-3 keyword extraction stage.
//!
//! Given posting Markdown, call the configured `keyword_extraction` model via
//! an injected [`crate::ports::LlmClient`], validate the response against the
//! embedded JSON Schema, retry on schema failure up to the configured number
//! of attempts, and return a typed [`KeywordExtractionOutcome`].
//!
//! The client's transient-retry / error-classification policy is separate from
//! this stage's schema-validation loop; `LlmError`s propagate immediately as
//! `AtsError::Llm(...)` (exit 5).

use std::sync::OnceLock;

use jsonschema::JSONSchema;
use serde_json::Value;
use time::OffsetDateTime;
use tracing::{debug, warn};

use crate::assets::{KEYWORD_RESPONSE_FORMAT, PROMPT_KEYWORD_EXTRACTION};
use crate::config::ModelStageConfig;
use crate::domain::KeywordSet;
use crate::error::AtsError;
use crate::ports::{
    AuditSink, ChatMessage, ChatRole, LlmCallRecord, LlmClient, LlmRequest, TokenUsage,
};

/// Stage output: the validated [`KeywordSet`], a pre-rendered Markdown view
/// intended for the downstream optimizer's system prompt, and the sum of token
/// usage across all LLM attempts in this stage.
#[derive(Debug, Clone)]
pub struct KeywordExtractionOutcome {
    pub set: KeywordSet,
    pub markdown: String,
    pub usage_total: TokenUsage,
}

/// Low-signal threshold (AC-3.3). Postings shorter than this get a
/// `tracing::warn!` but extraction proceeds.
pub const LOW_SIGNAL_WORD_THRESHOLD: usize = 200;

/// Run the keyword-extraction stage.
///
/// * `llm` — injected chat completion client (any [`LlmClient`] impl; production
///   uses `ats-llm`'s `OpenRouterClient`, tests use a fake).
/// * `audit` — sink recording schema-invalid attempts at the stage level; the
///   client records HTTP-level attempts (ok / transient / auth / ...).
/// * `posting_md` — job posting Markdown, read verbatim as the user message.
/// * `model` — per-stage model config (`name`, `temperature`, `seed`).
/// * `schema_max_attempts` — maximum number of schema validation retries (total
///   attempts at the stage level, including the first one). Pulled from
///   `cfg.retries.schema_validation_max_attempts`.
pub async fn extract(
    llm: &dyn LlmClient,
    audit: &dyn AuditSink,
    posting_md: &str,
    model: &ModelStageConfig,
    schema_max_attempts: u32,
) -> Result<KeywordExtractionOutcome, AtsError> {
    if schema_max_attempts == 0 {
        return Err(AtsError::Other(
            "schema_validation_max_attempts must be >= 1".into(),
        ));
    }

    let words = posting_md.split_whitespace().count();
    if words < LOW_SIGNAL_WORD_THRESHOLD {
        warn!(
            target: "ats::keywords",
            words,
            low_signal = true,
            "low-signal posting (< {LOW_SIGNAL_WORD_THRESHOLD} words)"
        );
    }

    let response_format = keyword_response_format().clone();
    let messages = vec![
        ChatMessage {
            role: ChatRole::System,
            content: PROMPT_KEYWORD_EXTRACTION.to_string(),
        },
        ChatMessage {
            role: ChatRole::User,
            content: posting_md.to_string(),
        },
    ];

    let mut usage_total = TokenUsage::ZERO;
    let mut last_response_snippet = String::new();
    let mut last_error = String::new();

    for attempt in 1..=schema_max_attempts {
        let request = LlmRequest {
            stage: "keywords",
            model: model.name.clone(),
            temperature: model.temperature,
            seed: Some(model.seed),
            messages: messages.clone(),
            response_format: Some(response_format.clone()),
        };

        let response = llm
            .complete(request)
            .await
            .map_err(|err| AtsError::Llm(err.class()))?;

        usage_total.prompt += response.usage.prompt;
        usage_total.completion += response.usage.completion;
        usage_total.total += response.usage.total;

        match validate_and_parse(&response.content) {
            Ok(set) => {
                debug!(
                    target: "ats::keywords",
                    attempt,
                    "keyword extraction schema-valid"
                );
                let markdown = to_markdown(&set);
                return Ok(KeywordExtractionOutcome {
                    set,
                    markdown,
                    usage_total,
                });
            }
            Err(reason) => {
                warn!(
                    target: "ats::keywords",
                    attempt,
                    error = %reason,
                    "keyword response failed schema validation"
                );
                last_response_snippet = snippet(&response.content, 500);
                last_error = reason.clone();
                record_schema_invalid(
                    audit,
                    model,
                    posting_md,
                    &response.content,
                    response.usage,
                    attempt,
                    &reason,
                );
            }
        }
    }

    Err(AtsError::SchemaInvalid(format!(
        "keyword extraction: {schema_max_attempts} attempts exceeded schema validation (last error: {last_error}; last response: {last_response_snippet})"
    )))
}

fn record_schema_invalid(
    audit: &dyn AuditSink,
    model: &ModelStageConfig,
    prompt_md: &str,
    response_content: &str,
    usage: TokenUsage,
    attempt: u32,
    reason: &str,
) {
    let record = LlmCallRecord {
        timestamp: iso_now(),
        stage: "keywords".into(),
        model: model.name.clone(),
        temperature: model.temperature,
        seed: Some(model.seed),
        prompt: snippet(prompt_md, 2_000),
        response: format!(
            "[schema-invalid: {reason}] {}",
            snippet(response_content, 4_000)
        ),
        usage,
        attempt,
        outcome: "schema-invalid".into(),
    };
    if let Err(err) = audit.record(&record) {
        warn!(
            target: "ats::keywords",
            %err,
            "failed to record schema-invalid audit entry"
        );
    }
}

fn iso_now() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .unwrap_or_else(|_| "unknown".into())
}

fn snippet(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{truncated}…")
}

fn validate_and_parse(content: &str) -> Result<KeywordSet, String> {
    let value: Value = serde_json::from_str(content)
        .map_err(|err| format!("content is not valid JSON: {err}"))?;
    parse_keywords_from_value(&value)
}

/// Validate a JSON [`Value`] against the embedded `ats_keyword_extraction` inner
/// schema and deserialize to [`KeywordSet`].
pub fn parse_keywords_from_value(value: &Value) -> Result<KeywordSet, String> {
    let validator = keyword_schema_validator();
    if let Err(mut errors) = validator.validate(value) {
        let first = errors
            .next()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "unknown schema violation".into());
        return Err(format!("schema violation: {first}"));
    }

    serde_json::from_value::<KeywordSet>(value.clone())
        .map_err(|err| format!("deserialise KeywordSet: {err}"))
}

/// Parsed `KEYWORD_RESPONSE_FORMAT` JSON (the whole `{type, json_schema:{...}}`
/// envelope), cached across calls.
fn keyword_response_format() -> &'static Value {
    static CACHED: OnceLock<Value> = OnceLock::new();
    CACHED.get_or_init(|| {
        serde_json::from_str::<Value>(KEYWORD_RESPONSE_FORMAT)
            .expect("embedded ats_keyword_extraction.json must be valid JSON")
    })
}

/// Compiled [`jsonschema::JSONSchema`] validator over the inner
/// `json_schema.schema` object, cached across calls.
fn keyword_schema_validator() -> &'static JSONSchema {
    static VALIDATOR: OnceLock<JSONSchema> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        let envelope = keyword_response_format();
        let inner = envelope
            .pointer("/json_schema/schema")
            .cloned()
            .expect("ats_keyword_extraction.json must contain /json_schema/schema");
        JSONSchema::options()
            .compile(&inner)
            .expect("embedded ats_keyword_extraction.json schema must compile")
    })
}

/// Render a human-readable Markdown view of [`KeywordSet`], bucketed by
/// category and sorted descending by `importance_score`. Used by US-4's
/// optimizer prompt as a token-efficient summary.
pub fn to_markdown(set: &KeywordSet) -> String {
    let mut out = String::new();

    push_section(&mut out, "Hard Skills and Tools", |buf| {
        let mut items = set.hard_skills_and_tools.clone();
        items.sort_by(|a, b| b.importance_score.cmp(&a.importance_score));
        for item in items {
            let mut bullet = format!(
                "- {term} (score {score}",
                term = item.primary_term,
                score = item.importance_score,
            );
            push_optional(&mut bullet, "cluster", &item.semantic_cluster);
            push_optional(&mut bullet, "acronym", &item.acronym);
            bullet.push(')');
            buf.push_str(&bullet);
            buf.push('\n');
        }
    });

    push_section(&mut out, "Soft Skills and Competencies", |buf| {
        let mut items = set.soft_skills_and_competencies.clone();
        items.sort_by(|a, b| b.importance_score.cmp(&a.importance_score));
        for item in items {
            let mut bullet = format!(
                "- {term} (score {score}",
                term = item.primary_term,
                score = item.importance_score,
            );
            push_optional(&mut bullet, "cluster", &item.semantic_cluster);
            bullet.push(')');
            buf.push_str(&bullet);
            buf.push('\n');
        }
    });

    push_section(&mut out, "Industry-Specific Terminology", |buf| {
        let mut items = set.industry_specific_terminology.clone();
        items.sort_by(|a, b| b.importance_score.cmp(&a.importance_score));
        for item in items {
            let mut bullet = format!(
                "- {term} (score {score}",
                term = item.primary_term,
                score = item.importance_score,
            );
            push_optional(&mut bullet, "acronym", &item.acronym);
            bullet.push(')');
            buf.push_str(&bullet);
            buf.push('\n');
        }
    });

    push_section(&mut out, "Certifications and Credentials", |buf| {
        let mut items = set.certifications_and_credentials.clone();
        items.sort_by(|a, b| b.importance_score.cmp(&a.importance_score));
        for item in items {
            buf.push_str(&format!(
                "- {term} (score {score})\n",
                term = item.primary_term,
                score = item.importance_score,
            ));
        }
    });

    push_section(&mut out, "Job Titles and Seniority", |buf| {
        let mut items = set.job_titles_and_seniority.clone();
        items.sort_by(|a, b| b.importance_score.cmp(&a.importance_score));
        for item in items {
            buf.push_str(&format!(
                "- {term} (score {score})\n",
                term = item.primary_term,
                score = item.importance_score,
            ));
        }
    });

    // Trim trailing blank line from the last section's separator.
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

fn push_section(out: &mut String, heading: &str, body: impl FnOnce(&mut String)) {
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&format!("## {heading}\n"));
    body(out);
}

fn push_optional(out: &mut String, label: &str, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    out.push_str(&format!(", {label} {trimmed}"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        CertificationKeyword, HardSkill, IndustryTerm, JobTitle, KeywordSet, SoftSkill,
    };
    use crate::ports::{LlmError, LlmResponse};
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::io;
    use std::sync::Mutex;

    struct ScriptedLlm {
        scripted: Mutex<VecDeque<Result<LlmResponse, LlmError>>>,
        calls: Mutex<Vec<LlmRequest>>,
    }

    impl ScriptedLlm {
        fn new(responses: Vec<Result<LlmResponse, LlmError>>) -> Self {
            Self {
                scripted: Mutex::new(responses.into_iter().collect()),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl LlmClient for ScriptedLlm {
        async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
            self.calls.lock().unwrap().push(req);
            self.scripted
                .lock()
                .unwrap()
                .pop_front()
                .expect("no more scripted responses")
        }
    }

    #[derive(Default)]
    struct MemAudit(Mutex<Vec<LlmCallRecord>>);

    impl AuditSink for MemAudit {
        fn record(&self, call: &LlmCallRecord) -> io::Result<()> {
            self.0.lock().unwrap().push(call.clone());
            Ok(())
        }
    }

    fn model_cfg() -> ModelStageConfig {
        ModelStageConfig {
            name: "openai/gpt-x".into(),
            temperature: 0.0,
            seed: 42,
        }
    }

    fn valid_json() -> String {
        serde_json::to_string(&serde_json::json!({
            "hard_skills_and_tools": [{
                "primary_term": "Rust",
                "acronym": "",
                "semantic_cluster": "systems",
                "importance_score": 9
            }],
            "soft_skills_and_competencies": [{
                "primary_term": "Communication",
                "semantic_cluster": "collaboration",
                "importance_score": 5
            }],
            "industry_specific_terminology": [{
                "primary_term": "HIPAA",
                "acronym": "",
                "importance_score": 7
            }],
            "certifications_and_credentials": [{
                "primary_term": "AWS SAA",
                "importance_score": 6
            }],
            "job_titles_and_seniority": [{
                "primary_term": "Staff Engineer",
                "importance_score": 8
            }]
        }))
        .unwrap()
    }

    fn response(body: &str) -> Result<LlmResponse, LlmError> {
        Ok(LlmResponse {
            content: body.to_string(),
            usage: TokenUsage {
                prompt: 100,
                completion: 50,
                total: 150,
            },
            raw: serde_json::json!({"choices": [{"message": {"content": body}}]}),
        })
    }

    fn long_posting() -> String {
        "Senior Rust Engineer posting. ".repeat(60)
    }

    #[tokio::test]
    async fn schema_valid_first_try() {
        let llm = ScriptedLlm::new(vec![response(&valid_json())]);
        let audit = MemAudit::default();
        let outcome = extract(&llm, &audit, &long_posting(), &model_cfg(), 3)
            .await
            .unwrap();
        assert_eq!(outcome.set.hard_skills_and_tools[0].primary_term, "Rust");
        assert_eq!(outcome.usage_total.total, 150);
        assert_eq!(audit.0.lock().unwrap().len(), 0);
        assert!(outcome.markdown.contains("## Hard Skills and Tools"));
    }

    #[tokio::test]
    async fn schema_invalid_twice_then_valid() {
        // 1st: non-JSON. 2nd: valid JSON but schema-violating (importance_score wrong type).
        // 3rd: valid.
        let bad_non_json = "not-json {oops";
        let bad_schema = serde_json::to_string(&serde_json::json!({
            "hard_skills_and_tools": [{
                "primary_term": "Rust",
                "acronym": "",
                "semantic_cluster": "systems",
                "importance_score": "nine"
            }],
            "soft_skills_and_competencies": [],
            "industry_specific_terminology": [],
            "certifications_and_credentials": [],
            "job_titles_and_seniority": []
        }))
        .unwrap();
        let llm = ScriptedLlm::new(vec![
            response(bad_non_json),
            response(&bad_schema),
            response(&valid_json()),
        ]);
        let audit = MemAudit::default();
        let outcome = extract(&llm, &audit, &long_posting(), &model_cfg(), 3)
            .await
            .unwrap();
        assert_eq!(outcome.set.hard_skills_and_tools[0].primary_term, "Rust");
        let records = audit.0.lock().unwrap();
        assert_eq!(records.len(), 2, "two schema-invalid attempts recorded");
        for rec in records.iter() {
            assert_eq!(rec.outcome, "schema-invalid");
        }
        assert_eq!(outcome.usage_total.total, 450);
    }

    #[tokio::test]
    async fn three_strikes_returns_schema_invalid() {
        let llm = ScriptedLlm::new(vec![
            response("not json"),
            response("still not json"),
            response("nope"),
        ]);
        let audit = MemAudit::default();
        let err = extract(&llm, &audit, &long_posting(), &model_cfg(), 3)
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 6);
        let msg = err.to_string();
        assert!(msg.contains("keyword extraction"), "got: {msg}");
        assert_eq!(audit.0.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn llm_error_propagates_without_schema_retry() {
        let llm = ScriptedLlm::new(vec![Err(LlmError::Auth("401".into()))]);
        let audit = MemAudit::default();
        let err = extract(&llm, &audit, &long_posting(), &model_cfg(), 3)
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 5);
    }

    #[tokio::test]
    async fn low_signal_short_posting_still_proceeds() {
        let llm = ScriptedLlm::new(vec![response(&valid_json())]);
        let audit = MemAudit::default();
        let outcome = extract(&llm, &audit, "short posting", &model_cfg(), 3)
            .await
            .unwrap();
        assert_eq!(outcome.set.hard_skills_and_tools[0].primary_term, "Rust");
    }

    #[test]
    fn keyword_set_markdown_view_sorts_and_formats() {
        let set = KeywordSet {
            hard_skills_and_tools: vec![
                HardSkill {
                    primary_term: "Python".into(),
                    acronym: "".into(),
                    semantic_cluster: "languages".into(),
                    importance_score: 5,
                },
                HardSkill {
                    primary_term: "Rust".into(),
                    acronym: "".into(),
                    semantic_cluster: "systems".into(),
                    importance_score: 9,
                },
            ],
            soft_skills_and_competencies: vec![SoftSkill {
                primary_term: "Leadership".into(),
                semantic_cluster: "".into(),
                importance_score: 6,
            }],
            industry_specific_terminology: vec![IndustryTerm {
                primary_term: "KPI".into(),
                acronym: "Key Performance Indicator".into(),
                importance_score: 4,
            }],
            certifications_and_credentials: vec![CertificationKeyword {
                primary_term: "AWS SAA".into(),
                importance_score: 3,
            }],
            job_titles_and_seniority: vec![JobTitle {
                primary_term: "Staff Engineer".into(),
                importance_score: 8,
            }],
        };
        let md = to_markdown(&set);
        let expected = "## Hard Skills and Tools\n\
            - Rust (score 9, cluster systems)\n\
            - Python (score 5, cluster languages)\n\
            \n## Soft Skills and Competencies\n\
            - Leadership (score 6)\n\
            \n## Industry-Specific Terminology\n\
            - KPI (score 4, acronym Key Performance Indicator)\n\
            \n## Certifications and Credentials\n\
            - AWS SAA (score 3)\n\
            \n## Job Titles and Seniority\n\
            - Staff Engineer (score 8)\n";
        assert_eq!(md, expected);
    }
}
