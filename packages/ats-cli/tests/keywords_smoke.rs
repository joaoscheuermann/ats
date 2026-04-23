//! End-to-end smoke tests for `ats keywords` against a local wiremock-backed
//! OpenRouter stand-in. Drives the release-shaped binary via `assert_cmd`
//! while routing HTTP through a mock server so no real API key is required.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::tempdir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn write_config(dir: &Path, base_url: &str) {
    let cfg = format!(
        r#"{{
  "openrouter": {{ "api_key": "test-key", "base_url": "{base_url}" }},
  "models": {{
    "scrape_to_markdown":  {{ "name": "x/y", "temperature": 0.0, "seed": 42 }},
    "keyword_extraction":  {{ "name": "x/y", "temperature": 0.0, "seed": 42 }},
    "resume_optimization": {{ "name": "x/y", "temperature": 0.2, "seed": 42 }}
  }},
  "scrape": {{ "network_idle_timeout_ms": 30000 }},
  "retries": {{
    "llm_transient_max_attempts": 5,
    "llm_transient_backoff_ms": [0, 0, 0, 0, 0],
    "schema_validation_max_attempts": 3
  }}
}}"#
    );
    fs::write(dir.join("config.json"), cfg).unwrap();
}

fn valid_keyword_payload() -> Value {
    json!({
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
            "primary_term": "Senior Software Engineer",
            "importance_score": 8
        }]
    })
}

fn chat_body(content: &str) -> Value {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 834,
            "completion_tokens": 712,
            "total_tokens": 1546
        }
    })
}

fn long_posting_md() -> String {
    // ~60 sentences to stay above the 200-word low-signal threshold.
    "Senior Rust Engineer role with heavy concurrency. We value clear communication. "
        .repeat(30)
}

fn runs_dir_keywords_run(root: &Path) -> std::path::PathBuf {
    let runs = root.join("runs");
    let entries: Vec<_> = fs::read_dir(&runs)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .contains("_keywords")
        })
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "expected exactly one keywords run folder in {}",
        runs.display()
    );
    entries[0].path()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keywords_happy_path_writes_artifacts_and_stderr_reports_tokens() {
    let server = MockServer::start().await;
    let response_content = serde_json::to_string(&valid_keyword_payload()).unwrap();
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_body(&response_content)))
        .mount(&server)
        .await;

    let tmp = tempdir().unwrap();
    write_config(tmp.path(), &server.uri());

    let assert = Command::cargo_bin("ats")
        .unwrap()
        .env("ATS_BINARY_DIR", tmp.path())
        .arg("keywords")
        .write_stdin(long_posting_md())
        .assert()
        .code(0);

    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Value = serde_json::from_str(&stdout).expect("stdout should be JSON");
    for key in [
        "hard_skills_and_tools",
        "soft_skills_and_competencies",
        "industry_specific_terminology",
        "certifications_and_credentials",
        "job_titles_and_seniority",
    ] {
        assert!(parsed.get(key).is_some(), "missing key {key}");
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[keywords] tokens:"),
        "stderr should contain token summary: {stderr}"
    );

    let run_dir = runs_dir_keywords_run(tmp.path());
    assert!(run_dir.join("keywords.json").is_file());
    assert!(run_dir.join("keywords.md").is_file());
    assert!(run_dir.join("llm-audit.jsonl").is_file());
    assert!(run_dir.join("run.json").is_file());

    let audit = fs::read_to_string(run_dir.join("llm-audit.jsonl")).unwrap();
    let lines: Vec<&str> = audit.lines().collect();
    assert_eq!(lines.len(), 1, "expected one audit record, got {}", lines.len());
    let parsed: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(parsed["outcome"], "ok");
    assert_eq!(parsed["usage"]["total"], 1546);

    let run_json: Value =
        serde_json::from_str(&fs::read_to_string(run_dir.join("run.json")).unwrap()).unwrap();
    assert_eq!(run_json["token_usage_total"]["total"], 1546);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keywords_three_invalid_responses_exit_six_and_record_schema_invalid() {
    let server = MockServer::start().await;
    // Three malformed responses (valid JSON objects but fails schema).
    let bad = json!({
        "hard_skills_and_tools": "should be an array, not a string",
    });
    let bad_content = serde_json::to_string(&bad).unwrap();
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_body(&bad_content)))
        .mount(&server)
        .await;

    let tmp = tempdir().unwrap();
    write_config(tmp.path(), &server.uri());

    let assert = Command::cargo_bin("ats")
        .unwrap()
        .env("ATS_BINARY_DIR", tmp.path())
        .arg("keywords")
        .write_stdin(long_posting_md())
        .assert()
        .code(6);

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("keyword extraction") || stderr.contains("schema"),
        "stderr should mention schema failure: {stderr}"
    );

    let run_dir = runs_dir_keywords_run(tmp.path());
    let audit = fs::read_to_string(run_dir.join("llm-audit.jsonl")).unwrap();
    let schema_invalid_count = audit
        .lines()
        .filter(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            v["outcome"] == "schema-invalid"
        })
        .count();
    assert_eq!(
        schema_invalid_count, 3,
        "expected three schema-invalid records, got audit:\n{audit}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keywords_auth_failure_exits_five_with_single_audit_record() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_string("invalid api key"))
        .mount(&server)
        .await;

    let tmp = tempdir().unwrap();
    write_config(tmp.path(), &server.uri());

    Command::cargo_bin("ats")
        .unwrap()
        .env("ATS_BINARY_DIR", tmp.path())
        .arg("keywords")
        .write_stdin(long_posting_md())
        .assert()
        .code(5);

    let run_dir = runs_dir_keywords_run(tmp.path());
    let audit = fs::read_to_string(run_dir.join("llm-audit.jsonl")).unwrap();
    let lines: Vec<&str> = audit.lines().collect();
    assert_eq!(lines.len(), 1);
    let parsed: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(parsed["outcome"], "auth");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keywords_context_exceeded_exits_five() {
    let server = MockServer::start().await;
    let body = r#"{"error":{"message":"This model's maximum context length is 4096 tokens","code":"context_length_exceeded"}}"#;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_string(body))
        .mount(&server)
        .await;

    let tmp = tempdir().unwrap();
    write_config(tmp.path(), &server.uri());

    Command::cargo_bin("ats")
        .unwrap()
        .env("ATS_BINARY_DIR", tmp.path())
        .arg("keywords")
        .write_stdin(long_posting_md())
        .assert()
        .code(5);

    let run_dir = runs_dir_keywords_run(tmp.path());
    let audit = fs::read_to_string(run_dir.join("llm-audit.jsonl")).unwrap();
    let lines: Vec<&str> = audit.lines().collect();
    assert_eq!(lines.len(), 1);
    let parsed: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(parsed["outcome"], "context-exceeded");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keywords_transient_then_success_records_three_attempts() {
    let server = MockServer::start().await;
    // First attempt: 429.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(429).set_body_string("rate limit"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    // Second attempt: 429.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(429).set_body_string("rate limit"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    // Third attempt: 200 with valid payload.
    let content = serde_json::to_string(&valid_keyword_payload()).unwrap();
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_body(&content)))
        .mount(&server)
        .await;

    let tmp = tempdir().unwrap();
    write_config(tmp.path(), &server.uri());

    Command::cargo_bin("ats")
        .unwrap()
        .env("ATS_BINARY_DIR", tmp.path())
        .arg("keywords")
        .write_stdin(long_posting_md())
        .assert()
        .code(0);

    let run_dir = runs_dir_keywords_run(tmp.path());
    let audit = fs::read_to_string(run_dir.join("llm-audit.jsonl")).unwrap();
    let outcomes: Vec<String> = audit
        .lines()
        .map(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            v["outcome"].as_str().unwrap().to_string()
        })
        .collect();
    assert_eq!(outcomes, vec!["transient", "transient", "ok"]);
}
