//! Three-pass YAML validation pipeline for `ats render` (AC-1.3 / AC-1.4).
//!
//! 1. Parse the YAML bytes into a `serde_json::Value`. Syntax errors surface
//!    here with best-effort `line`/`column` from the parser.
//! 2. Run every JSON Schema error through
//!    [`jsonschema::JSONSchema::iter_errors`]; emit the **first** error with
//!    its JSON pointer rewritten into the dotted form used in diagnostics
//!    (e.g. `/cv/work_experience/1/start_date` → `cv.work_experience[1].start_date`).
//! 3. Only if the Value validated cleanly, re-run `serde_path_to_error` to
//!    drive the strongly-typed [`Resume`] deserialisation. Errors here also
//!    produce a [`YamlDiag`], but without line/column (the YAML source
//!    positions are not recoverable past step 1).
//!
//! All three passes return the same diagnostic type so the CLI's error
//! plumbing stays uniform.

use std::sync::OnceLock;

use jsonschema::error::ValidationErrorKind;
use jsonschema::{JSONSchema, ValidationError};
use serde_json::Value;

use crate::assets::YAML_SCHEMA;
use crate::domain::{Resume, ResumeYaml};
use crate::error::YamlDiag;

/// Compiled JSON Schema validator, lazily compiled on first validation. The
/// schema is embedded so this never performs IO.
fn schema_validator() -> &'static JSONSchema {
    static VALIDATOR: OnceLock<JSONSchema> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        let schema_value: Value = serde_json::from_str(YAML_SCHEMA)
            .expect("embedded yaml_schema.json must be valid JSON");
        JSONSchema::options()
            .compile(&schema_value)
            .expect("embedded yaml_schema.json must compile as a JSON Schema")
    })
}

/// Parse `bytes` as YAML and produce a typed [`Resume`]. On failure, return a
/// structured [`YamlDiag`] naming the offending field, a reason, and
/// (when recoverable) line/column.
pub fn parse_and_validate(bytes: &[u8]) -> Result<Resume, YamlDiag> {
    let value = parse_yaml_to_value(bytes)?;
    validate_value_against_schema(&value)?;
    deserialize_value(value)
}

/// Pass 1 — raw YAML syntax / document parse.
fn parse_yaml_to_value(bytes: &[u8]) -> Result<Value, YamlDiag> {
    serde_yaml_ng::from_slice::<Value>(bytes).map_err(|err| {
        let location = err.location();
        YamlDiag {
            path: None,
            reason: format!("invalid YAML: {err}"),
            line: location.as_ref().map(|l| l.line()),
            column: location.as_ref().map(|l| l.column()),
        }
    })
}

/// Pass 2 — JSON Schema validation. Return the first structural violation.
fn validate_value_against_schema(value: &Value) -> Result<(), YamlDiag> {
    let validator = schema_validator();
    let Err(errors) = validator.validate(value) else {
        return Ok(());
    };
    let first = errors
        .map(validation_error_to_diag)
        .next()
        .expect("validator returned Err without any errors");
    Err(first)
}

/// Map a single [`ValidationError`] into a [`YamlDiag`], applying
/// kind-specific path enrichment:
///
/// - `Required { property }` — the schema error points at the parent object
///   missing the property; we append `.<property>` so the diagnostic shows
///   e.g. `cv.professional_summary` instead of just `cv`.
/// - `AdditionalProperties { unexpected }` — emit the parent path and list
///   the unexpected keys in the reason.
/// - Everything else — use the JSON-pointer → dotted form verbatim.
fn validation_error_to_diag(err: ValidationError<'_>) -> YamlDiag {
    let base_path = json_pointer_to_dotted(&err.instance_path.to_string());
    let (path, reason) = match &err.kind {
        ValidationErrorKind::Required { property } => {
            let prop = property.as_str().unwrap_or("").to_string();
            let full = if base_path.is_empty() {
                prop.clone()
            } else {
                format!("{base_path}.{prop}")
            };
            (Some(full), format!("`{prop}` is a required property"))
        }
        ValidationErrorKind::AdditionalProperties { unexpected } => {
            let listed = unexpected.join(", ");
            (
                option_path(&base_path),
                format!("unexpected additional properties: {listed}"),
            )
        }
        _ => (option_path(&base_path), err.to_string()),
    };
    YamlDiag {
        path,
        reason,
        line: None,
        column: None,
    }
}

fn option_path(path: &str) -> Option<String> {
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

/// Pass 3 — strongly-typed deserialise (only reached when schema validation
/// succeeds). `serde_path_to_error` builds a dotted path automatically; the
/// root of that path is `ResumeYaml`, so paths are already `cv.*`.
fn deserialize_value(value: Value) -> Result<Resume, YamlDiag> {
    serde_path_to_error::deserialize::<_, ResumeYaml>(value)
        .map(|wrapper| wrapper.cv)
        .map_err(|err| {
            let path_str = err.path().to_string();
            let path = if path_str.is_empty() {
                None
            } else {
                Some(path_str)
            };
            YamlDiag {
                path,
                reason: err.inner().to_string(),
                line: None,
                column: None,
            }
        })
}

/// Convert an RFC-6901 JSON pointer (`/cv/work_experience/1/start_date`) into
/// the dotted field form used by [`YamlDiag`]
/// (`cv.work_experience[1].start_date`).
///
/// Numeric segments become `[N]` attached to the previous segment. Non-numeric
/// segments are joined with `.`. JSON-pointer escapes (`~0`, `~1`) are
/// unescaped so that keys containing `/` or `~` round-trip correctly.
pub fn json_pointer_to_dotted(pointer: &str) -> String {
    if pointer.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for segment in pointer.trim_start_matches('/').split('/') {
        let unescaped = segment.replace("~1", "/").replace("~0", "~");
        if !unescaped.is_empty() && unescaped.chars().all(|c| c.is_ascii_digit()) {
            out.push('[');
            out.push_str(&unescaped);
            out.push(']');
            continue;
        }
        if !out.is_empty() {
            out.push('.');
        }
        out.push_str(&unescaped);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_pointer_empty_maps_to_empty_string() {
        assert_eq!(json_pointer_to_dotted(""), "");
    }

    #[test]
    fn json_pointer_root_with_field() {
        assert_eq!(json_pointer_to_dotted("/cv"), "cv");
    }

    #[test]
    fn json_pointer_nested_object() {
        assert_eq!(
            json_pointer_to_dotted("/cv/personal_information/full_name"),
            "cv.personal_information.full_name"
        );
    }

    #[test]
    fn json_pointer_array_index_is_bracketed_and_attached() {
        assert_eq!(
            json_pointer_to_dotted("/cv/work_experience/1/start_date"),
            "cv.work_experience[1].start_date"
        );
    }

    #[test]
    fn json_pointer_trailing_array_index() {
        assert_eq!(json_pointer_to_dotted("/cv/skills/0"), "cv.skills[0]");
    }

    #[test]
    fn json_pointer_unescapes_slash_and_tilde() {
        // ~1 encodes '/', ~0 encodes '~' per RFC 6901.
        assert_eq!(json_pointer_to_dotted("/foo~1bar"), "foo/bar");
        assert_eq!(json_pointer_to_dotted("/foo~0bar"), "foo~bar");
    }

    #[test]
    fn json_pointer_leading_array_index_is_bracketed_standalone() {
        // No previous segment — index still becomes `[0]` with nothing before.
        assert_eq!(json_pointer_to_dotted("/0"), "[0]");
    }

    fn happy_yaml() -> Vec<u8> {
        br#"
cv:
  personal_information:
    full_name: Jane Doe
    email: jane@example.com
    phone: "+1 555-0100"
    linkedin_url: https://linkedin.com/in/jane
    location: Remote
  professional_summary: "Experienced engineer."
"#
        .to_vec()
    }

    #[test]
    fn happy_path_returns_resume() {
        let resume = parse_and_validate(&happy_yaml()).expect("valid YAML must parse");
        assert_eq!(resume.personal_information.full_name, "Jane Doe");
        assert!(resume.skills.is_empty());
        assert!(resume.work_experience.is_empty());
    }

    #[test]
    fn missing_required_field_names_dotted_path() {
        let yaml = br#"
cv:
  personal_information:
    full_name: Jane Doe
    email: jane@example.com
    phone: "+1 555-0100"
    linkedin_url: https://linkedin.com/in/jane
    location: Remote
"#;
        let err = parse_and_validate(yaml).unwrap_err();
        let path = err.path.as_deref().unwrap_or("");
        assert!(
            path.contains("professional_summary"),
            "expected path to name professional_summary, got {err}"
        );
    }

    #[test]
    fn wrong_type_at_nested_array_index_maps_to_bracket_path() {
        // cv.work_experience[1].start_date is an integer.
        let yaml = br#"
cv:
  personal_information:
    full_name: Jane Doe
    email: jane@example.com
    phone: "+1"
    linkedin_url: https://example.com
    location: Remote
  professional_summary: "ok"
  work_experience:
    - job_title: One
      company_name: First
      location: NYC
      start_date: "01/2020"
      end_date: "Present"
    - job_title: Two
      company_name: Second
      location: NYC
      start_date: 42
      end_date: "01/2024"
"#;
        let err = parse_and_validate(yaml).unwrap_err();
        let path = err.path.as_deref().unwrap_or("");
        assert_eq!(
            path, "cv.work_experience[1].start_date",
            "expected bracketed index path, got `{path}`: {err}"
        );
    }

    #[test]
    fn syntax_error_sets_line_and_column() {
        // A tab under a mapping key is a classic YAML parse error.
        let yaml = b"cv:\n\tpersonal_information: {}\n";
        let err = parse_and_validate(yaml).unwrap_err();
        assert!(err.path.is_none(), "syntax errors have no structural path");
        assert!(
            err.line.is_some(),
            "syntax errors must carry a line number when the parser provides one"
        );
    }
}
