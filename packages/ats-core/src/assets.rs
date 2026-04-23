//! Locked, binary-embedded assets: the resume template, the YAML JSON Schema,
//! the three LLM system prompts, and the OpenAI-compatible structured-output
//! blocks for the scrape and keyword stages. Their bytes feed the baseline cache
//! hash (AC-1.5), so any edit here naturally invalidates the cache.
//!
//! All items here are frozen for this version — later Efforts consume these
//! constants verbatim. See `## Research` in the journal ticket for provenance.

/// Frozen Markdown template used by `ats render`.
pub const RESUME_TEMPLATE: &str = include_str!("../assets/resume_template.md");

/// JSON Schema enforcing the YAML contract. Effort 02 validates input with
/// this schema; Effort 01 only verifies it parses as JSON and compiles as a
/// JSON Schema validator at build-test time.
pub const YAML_SCHEMA: &str = include_str!("../assets/yaml_schema.json");

/// System prompt forwarded verbatim to the scrape-to-Markdown LLM call
/// (AC-2.1a).
pub const PROMPT_SCRAPE_TO_MARKDOWN: &str =
    include_str!("../assets/prompts/scrape_to_markdown.md");

/// System prompt forwarded verbatim to the keyword-extraction LLM call
/// (US-3).
pub const PROMPT_KEYWORD_EXTRACTION: &str =
    include_str!("../assets/prompts/keyword_extraction.md");

/// System prompt forwarded verbatim to the resume-optimization LLM call
/// (US-4).
pub const PROMPT_RESUME_OPTIMIZATION: &str =
    include_str!("../assets/prompts/resume_optimization.md");

/// OpenAI-compatible `response_format` block used for the US-3 keyword call.
pub const KEYWORD_RESPONSE_FORMAT: &str =
    include_str!("../assets/schemas/ats_keyword_extraction.json");

/// OpenAI-compatible `response_format` block used for the US-2 scrape-to-Markdown
/// call. Forces the model to return `{title, markdown}` under
/// `strict: true`, removing the need for a prose "output format" section in
/// the system prompt.
pub const SCRAPE_RESPONSE_FORMAT: &str =
    include_str!("../assets/schemas/job_posting_extraction.json");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_template_is_non_empty() {
        assert!(RESUME_TEMPLATE.contains("## Professional Summary"));
        assert!(RESUME_TEMPLATE.contains("## Work Experience"));
    }

    #[test]
    fn yaml_schema_parses_as_json() {
        let value: serde_json::Value =
            serde_json::from_str(YAML_SCHEMA).expect("yaml_schema.json must be valid JSON");
        assert_eq!(value["type"], "object");
    }

    #[test]
    fn yaml_schema_compiles_as_jsonschema_validator() {
        let value: serde_json::Value = serde_json::from_str(YAML_SCHEMA).unwrap();
        let validator = jsonschema::JSONSchema::options()
            .compile(&value)
            .expect("yaml_schema.json must compile as a JSON Schema validator");
        let ok = serde_json::json!({
            "cv": {
                "personal_information": {
                    "full_name": "Jane Doe",
                    "email": "jane@example.com",
                    "phone": "+1 555-0100",
                    "linkedin_url": "https://linkedin.com/in/jane",
                    "location": "Remote"
                },
                "professional_summary": "Experienced engineer."
            }
        });
        assert!(
            validator.is_valid(&ok),
            "minimally-compliant document should validate"
        );
    }

    #[test]
    fn keyword_response_format_parses_as_json() {
        let value: serde_json::Value = serde_json::from_str(KEYWORD_RESPONSE_FORMAT)
            .expect("ats_keyword_extraction.json must be valid JSON");
        assert_eq!(value["type"], "json_schema");
    }

    #[test]
    fn scrape_response_format_parses_as_json_and_inner_schema_compiles() {
        let value: serde_json::Value = serde_json::from_str(SCRAPE_RESPONSE_FORMAT)
            .expect("job_posting_extraction.json must be valid JSON");
        assert_eq!(value["type"], "json_schema");
        assert_eq!(value["json_schema"]["strict"], true);
        let inner = value
            .pointer("/json_schema/schema")
            .cloned()
            .expect("job_posting_extraction.json must contain /json_schema/schema");
        let validator = jsonschema::JSONSchema::options()
            .compile(&inner)
            .expect("inner schema must compile as a JSON Schema validator");
        let ok = serde_json::json!({
            "title": "Senior Rust Engineer",
            "markdown": "## About\n- build things"
        });
        assert!(validator.is_valid(&ok));
        let missing_title = serde_json::json!({ "markdown": "body" });
        assert!(!validator.is_valid(&missing_title));
    }

    #[test]
    fn prompts_are_non_empty() {
        for (name, prompt) in [
            ("scrape_to_markdown", PROMPT_SCRAPE_TO_MARKDOWN),
            ("keyword_extraction", PROMPT_KEYWORD_EXTRACTION),
            ("resume_optimization", PROMPT_RESUME_OPTIMIZATION),
        ] {
            assert!(
                !prompt.trim().is_empty(),
                "prompt `{name}` must not be empty"
            );
        }
    }
}
