//! OpenAI-compatible chat-completion client and audit sink.
//!
//! This crate implements the concretions that back the
//! [`ats_core::ports::LlmClient`] and [`ats_core::ports::AuditSink`] traits:
//!
//! - [`OpenRouterClient`] — HTTP adapter targeting OpenRouter's OpenAI-compatible
//!   `/chat/completions` endpoint, with the locked transient-retry policy and
//!   per-attempt audit logging.
//! - [`FileAuditSink`] — append-only JSON-lines writer pointed at
//!   `<run_folder>/llm-audit.jsonl`.
//! - [`CompositeAuditSink`] — observer-style fan-out used by the CLI to tee
//!   audit records into a file sink plus one or more reporters (e.g. stderr
//!   token-usage summaries).
//!
//! Keeping the concretions in a dedicated crate means the stage modules in
//! `ats-core` depend only on the ports, never on `reqwest`.

pub mod composite_audit;
pub mod file_audit;
pub mod openrouter;

pub use composite_audit::CompositeAuditSink;
pub use file_audit::FileAuditSink;
pub use openrouter::{is_context_exceeded_error, OpenRouterClient};
