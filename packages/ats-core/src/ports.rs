//! Port traits — the concretion-free seams the pipeline depends on (DIP, ISP).
//!
//! Effort 03 adds the `LlmClient` port plus its request/response/error DTOs.
//! PageScraper / PdfWriter traits land with their respective adapter Efforts
//! (04 / 06). Keeping each trait narrow lets callers depend on just what they
//! use.

use std::io;
use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;

use crate::error::LlmClass;

/// Read the current local time. Abstracted so tests can pin the clock.
pub trait Clock: Send + Sync {
    fn now_local(&self) -> OffsetDateTime;
}

/// Default clock: `time::OffsetDateTime::now_local`, falling back to UTC if
/// the platform refuses to give us a local offset (common inside CI
/// containers).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_local(&self) -> OffsetDateTime {
        OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc())
    }
}

/// Per-call LLM token usage (NFC-17 / NFC-19).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenUsage {
    pub prompt: u64,
    pub completion: u64,
    pub total: u64,
}

impl TokenUsage {
    pub const ZERO: TokenUsage = TokenUsage {
        prompt: 0,
        completion: 0,
        total: 0,
    };
}

/// One LLM call, as it will be written to `llm-audit.jsonl` (NFC-19).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCallRecord {
    pub timestamp: String,
    pub stage: String,
    pub model: String,
    pub temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    pub prompt: String,
    pub response: String,
    pub usage: TokenUsage,
    pub attempt: u32,
    pub outcome: String,
}

/// Sink for LLM call records. Implementations write to `llm-audit.jsonl`
/// inside a run folder; `record` is expected to be infallible short of I/O
/// problems.
pub trait AuditSink: Send + Sync {
    fn record(&self, call: &LlmCallRecord) -> io::Result<()>;
}

/// Buffers LLM call records in memory until a run directory exists (`ats run`).
pub struct VecAuditSink {
    records: Mutex<Vec<LlmCallRecord>>,
}

impl Default for VecAuditSink {
    fn default() -> Self {
        Self::new()
    }
}

impl VecAuditSink {
    pub fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
        }
    }

    /// Drain the buffer, replacing it with an empty vector.
    pub fn take(&self) -> io::Result<Vec<LlmCallRecord>> {
        let mut g = self
            .records
            .lock()
            .map_err(|_| io::Error::other("vec audit sink mutex poisoned"))?;
        Ok(std::mem::take(&mut *g))
    }
}

impl AuditSink for VecAuditSink {
    fn record(&self, call: &LlmCallRecord) -> io::Result<()> {
        let mut g = self
            .records
            .lock()
            .map_err(|_| io::Error::other("vec audit sink mutex poisoned"))?;
        g.push(call.clone());
        Ok(())
    }
}

/// One participant in the chat completion request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

/// OpenAI-compatible chat role. Serializes as lowercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

/// Stage-agnostic LLM request. Adapters forward `messages` and `response_format`
/// verbatim; `stage` is used only for audit / logging.
#[derive(Debug, Clone)]
pub struct LlmRequest {
    /// Short stage tag (e.g. `"keywords"`, `"scrape"`, `"optimize"`).
    pub stage: &'static str,
    pub model: String,
    pub temperature: f32,
    pub seed: Option<i64>,
    pub messages: Vec<ChatMessage>,
    /// OpenAI-compatible `response_format` block, forwarded verbatim when
    /// present.
    pub response_format: Option<serde_json::Value>,
}

/// The subset of the provider response we actually care about.
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: String,
    pub usage: TokenUsage,
    /// Preserved full provider body for audit / debugging.
    pub raw: serde_json::Value,
}

/// Provider-agnostic LLM error surface. Adapters map provider-specific errors
/// onto these four variants; the stage / CLI converts via [`LlmError::class`]
/// into an [`LlmClass`] and then into [`crate::error::AtsError::Llm`].
#[derive(Debug, Clone, thiserror::Error)]
pub enum LlmError {
    /// Network, 429, 5xx — eligible for the transient-retry loop.
    #[error("transient: {0}")]
    Transient(String),
    /// 401 / 403. Never retried.
    #[error("auth: {0}")]
    Auth(String),
    /// Prompt exceeded the model's context window (AC-3.5″). Never retried.
    #[error("context-exceeded (model `{model}`): {message}")]
    ContextExceeded {
        model: String,
        message: String,
        prompt_tokens: Option<u32>,
    },
    /// 4xx other than the above, or response parse errors. Never retried.
    #[error("llm-other: {0}")]
    Other(String),
}

impl LlmError {
    /// Map an [`LlmError`] to the coarser [`LlmClass`] used by
    /// [`crate::error::AtsError::Llm`].
    pub fn class(&self) -> LlmClass {
        match self {
            LlmError::Transient(msg) => LlmClass::Transient(msg.clone()),
            LlmError::Auth(msg) => LlmClass::Auth(msg.clone()),
            LlmError::ContextExceeded { model, message, .. } => {
                LlmClass::ContextExceeded(format!("{model}: {message}"))
            }
            LlmError::Other(msg) => LlmClass::Other(msg.clone()),
        }
    }

    /// Short tag suitable for an audit record's `outcome` field.
    pub fn outcome_tag(&self) -> &'static str {
        match self {
            LlmError::Transient(_) => "transient",
            LlmError::Auth(_) => "auth",
            LlmError::ContextExceeded { .. } => "context-exceeded",
            LlmError::Other(_) => "other",
        }
    }
}

/// Abstract OpenAI-compatible chat completion client. Narrow by design (ISP):
/// stages build an [`LlmRequest`] and consume an [`LlmResponse`].
#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError>;
}

/// Synchronous Markdown → PDF port (US-5). Invoked on a blocking pool from the
/// async CLI so the runtime is not held during rendering.
#[derive(Debug, Error)]
pub enum PdfError {
    #[error("render: {0}")]
    Render(String),
}

/// Markdown-to-PDF writer — implemented by the `ats-pdf` adapter.
pub trait PdfWriter: Send + Sync {
    /// Render `markdown` to a PDF at `out`. Implementations should write
    /// atomically when possible (`*.tmp` then rename to `out`).
    fn render(&self, markdown: &str, out: &Path) -> Result<(), PdfError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_produces_something_reasonable() {
        let clock = SystemClock;
        let t = clock.now_local();
        // Year check keeps this from silently using epoch-0 or similar bugs.
        assert!(t.year() >= 2024);
    }

    #[test]
    fn chat_role_serializes_lowercase() {
        let msg = ChatMessage {
            role: ChatRole::System,
            content: "s".into(),
        };
        let s = serde_json::to_string(&msg).unwrap();
        assert!(s.contains("\"role\":\"system\""), "got: {s}");
    }

    #[test]
    fn llm_error_maps_to_llm_class() {
        assert_eq!(
            LlmError::Transient("rate-limit".into()).class(),
            LlmClass::Transient("rate-limit".into())
        );
        assert_eq!(
            LlmError::Auth("401".into()).class(),
            LlmClass::Auth("401".into())
        );
        let ctx = LlmError::ContextExceeded {
            model: "m/x".into(),
            message: "too long".into(),
            prompt_tokens: Some(1234),
        };
        match ctx.class() {
            LlmClass::ContextExceeded(msg) => assert!(
                msg.contains("m/x") && msg.contains("too long"),
                "got: {msg}"
            ),
            other => panic!("expected ContextExceeded, got {other:?}"),
        }
    }

    #[test]
    fn llm_error_outcome_tags_are_stable() {
        assert_eq!(LlmError::Transient("".into()).outcome_tag(), "transient");
        assert_eq!(LlmError::Auth("".into()).outcome_tag(), "auth");
        assert_eq!(
            LlmError::ContextExceeded {
                model: "".into(),
                message: "".into(),
                prompt_tokens: None,
            }
            .outcome_tag(),
            "context-exceeded"
        );
        assert_eq!(LlmError::Other("".into()).outcome_tag(), "other");
    }
}
