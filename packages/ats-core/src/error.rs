//! Single error taxonomy for the `ats` CLI.
//!
//! Exit codes are the external contract — they must match the locked scheme
//! `0/1/2/3/4/5/6/7` in the journal ticket. Each variant stubbed here is
//! fleshed out in later Efforts (e.g. `YamlDiag::line` in Effort 02,
//! `LlmClass::Transient` in Effort 03). Effort 01 only needs the shape and
//! the `exit_code` mapping.

use std::io;

use thiserror::Error;

/// One error type, one exit code per variant. Variants keep their associated
/// payloads (`YamlDiag`, `ScrapeClass`, `LlmClass`) so later Efforts can
/// surface field paths, diagnostic classes, etc.
#[derive(Debug, Error)]
pub enum AtsError {
    /// Missing or malformed `config.json`. Exit code 2.
    #[error("config error: {0}")]
    Config(String),

    /// YAML input failed schema validation or parser. Exit code 3.
    #[error("yaml error: {0}")]
    Yaml(#[from] YamlDiag),

    /// Scrape stage failure (HTTP, network, navigation, offline). Exit code 4.
    #[error("scrape error: {0}")]
    Scrape(ScrapeClass),

    /// LLM provider failure after retries. Exit code 5.
    #[error("llm error: {0}")]
    Llm(LlmClass),

    /// US-3 keyword response failed JSON Schema validation the configured
    /// maximum number of attempts. Exit code 6.
    #[error("schema validation failed: {0}")]
    SchemaInvalid(String),

    /// PDF rendering failed. Exit code 7.
    #[error("pdf error: {0}")]
    Pdf(String),

    /// Catch-all filesystem / IO error. Exit code 1.
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    /// Fallback for unknown / unimplemented failures. Exit code 1.
    #[error("{0}")]
    Other(String),
}

impl From<crate::ports::PdfError> for AtsError {
    fn from(value: crate::ports::PdfError) -> Self {
        AtsError::Pdf(value.to_string())
    }
}

impl AtsError {
    /// Map a variant to its locked exit code.
    pub fn exit_code(&self) -> i32 {
        match self {
            AtsError::Config(_) => 2,
            AtsError::Yaml(_) => 3,
            AtsError::Scrape(_) => 4,
            AtsError::Llm(_) => 5,
            AtsError::SchemaInvalid(_) => 6,
            AtsError::Pdf(_) => 7,
            AtsError::Io(_) | AtsError::Other(_) => 1,
        }
    }

    /// Short, log-friendly tag for the variant — lets later Efforts attach
    /// the classification to a tracing event without re-matching by hand.
    pub fn class(&self) -> &'static str {
        match self {
            AtsError::Config(_) => "config",
            AtsError::Yaml(_) => "yaml",
            AtsError::Scrape(_) => "scrape",
            AtsError::Llm(_) => "llm",
            AtsError::SchemaInvalid(_) => "schema-invalid",
            AtsError::Pdf(_) => "pdf",
            AtsError::Io(_) => "io",
            AtsError::Other(_) => "other",
        }
    }
}

/// Structured YAML diagnostic. `line`/`column` are best-effort — the YAML
/// parser reports positions for syntax errors but not for type/path errors
/// once parsing has already produced a `serde_json::Value`.
///
/// `Display` emits a single line shaped like:
///
/// ```text
/// cv.work_experience[1].start_date: invalid type: integer, expected string (line 12, column 5)
/// ```
///
/// When `path` is `None`, the reason is emitted verbatim. When line/column
/// are `None`, the trailing parenthesised location is omitted.
#[derive(Debug)]
pub struct YamlDiag {
    /// Structural path into the YAML document, e.g. `cv.work_experience[1].start_date`.
    /// `None` when the YAML failed to parse syntactically and no pointer is
    /// recoverable.
    pub path: Option<String>,
    /// Human-readable failure reason.
    pub reason: String,
    /// Line number when known.
    pub line: Option<usize>,
    /// Column number when known.
    pub column: Option<usize>,
}

impl std::fmt::Display for YamlDiag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(path) = &self.path {
            write!(f, "{path}: {}", self.reason)?;
        } else {
            write!(f, "{}", self.reason)?;
        }
        match (self.line, self.column) {
            (Some(l), Some(c)) => write!(f, " (line {l}, column {c})"),
            (Some(l), None) => write!(f, " (line {l})"),
            _ => Ok(()),
        }
    }
}

impl std::error::Error for YamlDiag {}

/// Diagnostic class for scrape-stage failures (AC-2.3 / AC-2.5).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ScrapeClass {
    /// Auth redirect, 401, 403, etc.
    #[error("auth-required")]
    AuthRequired,
    /// 404 / page not found.
    #[error("not-found")]
    NotFound,
    /// 451 or similar regional blocks.
    #[error("geo-blocked")]
    GeoBlocked,
    /// Transport-level failure mid-navigation (TLS, connection reset).
    #[error("network-timeout")]
    NetworkTimeout,
    /// DNS / connect / unreachable (detected by the reachability probe).
    #[error("offline")]
    Offline,
    /// Navigation exceeded the configured network-idle timeout.
    #[error("timeout")]
    Timeout,
    /// Any other non-2xx main-frame HTTP status.
    #[error("http {0}")]
    Http(u16),
    /// `chromiumoxide` could not find a Chromium-family executable.
    #[error("browser-missing: {0}")]
    BrowserMissing(String),
    /// Escape hatch with free-form detail.
    #[error("other: {0}")]
    Other(String),
}

impl ScrapeClass {
    /// Short, stable tag suitable for use in `run.json`'s `outcome` field
    /// (e.g. `scrape/<tag>` or an audit log). Drops the free-form payload
    /// from [`ScrapeClass::Other`] / [`ScrapeClass::BrowserMissing`] so the
    /// tag stays stable across invocations.
    pub fn class_tag(&self) -> String {
        match self {
            ScrapeClass::AuthRequired => "auth-required".into(),
            ScrapeClass::NotFound => "not-found".into(),
            ScrapeClass::GeoBlocked => "geo-blocked".into(),
            ScrapeClass::NetworkTimeout => "network-timeout".into(),
            ScrapeClass::Offline => "offline".into(),
            ScrapeClass::Timeout => "timeout".into(),
            ScrapeClass::Http(status) => format!("http-{status}"),
            ScrapeClass::BrowserMissing(_) => "browser-missing".into(),
            ScrapeClass::Other(_) => "other".into(),
        }
    }
}

/// Diagnostic class for LLM-stage failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LlmClass {
    /// Transient (rate limit, 5xx, network) after all retries. Carries the
    /// final attempt's short message for logs / `run.json`.
    #[error("transient: {0}")]
    Transient(String),
    /// Auth / credentials.
    #[error("auth: {0}")]
    Auth(String),
    /// OpenRouter reports the prompt exceeds the model's context window
    /// (AC-3.5″). Carries the provider's message.
    #[error("context-exceeded: {0}")]
    ContextExceeded(String),
    /// Everything else.
    #[error("llm-other: {0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_match_locked_scheme() {
        let cases: &[(AtsError, i32)] = &[
            (AtsError::Config("x".into()), 2),
            (
                AtsError::Yaml(YamlDiag {
                    path: Some("cv".into()),
                    reason: "bad".into(),
                    line: None,
                    column: None,
                }),
                3,
            ),
            (AtsError::Scrape(ScrapeClass::NotFound), 4),
            (AtsError::Llm(LlmClass::Transient("x".into())), 5),
            (AtsError::SchemaInvalid("x".into()), 6),
            (AtsError::Pdf("x".into()), 7),
            (AtsError::Io(io::Error::other("x")), 1),
            (AtsError::Other("x".into()), 1),
        ];
        for (err, expected) in cases {
            assert_eq!(
                err.exit_code(),
                *expected,
                "variant {err:?} should map to exit code {expected}"
            );
        }
    }

    #[test]
    fn class_tags_are_stable() {
        assert_eq!(AtsError::Config("".into()).class(), "config");
        assert_eq!(AtsError::Other("".into()).class(), "other");
        assert_eq!(AtsError::SchemaInvalid("x".into()).class(), "schema-invalid");
    }

    #[test]
    fn yaml_diag_renders_position_when_present() {
        let diag = YamlDiag {
            path: Some("cv.work_experience[0].start_date".into()),
            reason: "expected string".into(),
            line: Some(12),
            column: Some(5),
        };
        let rendered = diag.to_string();
        assert!(rendered.contains("cv.work_experience[0].start_date"));
        assert!(rendered.contains("line 12"));
        assert!(rendered.contains("column 5"));
    }

    #[test]
    fn yaml_diag_omits_position_when_absent() {
        let diag = YamlDiag {
            path: Some("cv".into()),
            reason: "bad".into(),
            line: None,
            column: None,
        };
        let rendered = diag.to_string();
        assert!(!rendered.contains("line"));
        assert!(!rendered.contains("column"));
    }

    #[test]
    fn yaml_diag_without_path_emits_reason_only() {
        let diag = YamlDiag {
            path: None,
            reason: "invalid YAML syntax".into(),
            line: Some(3),
            column: Some(1),
        };
        let rendered = diag.to_string();
        assert_eq!(rendered, "invalid YAML syntax (line 3, column 1)");
    }
}
