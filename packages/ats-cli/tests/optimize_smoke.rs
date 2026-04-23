//! `ats optimize` end-to-end tests with wiremock; invalid inputs use exit 2/6.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use assert_cmd::Command;
use serde_json::json;
use serde_json::Value;
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

fn chat_body_markdown() -> String {
    let content = r#"# Jane Doe
## Optimized
Using Widget tools."#;
    serde_json::to_string(&json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 20,
            "total_tokens": 30
        }
    }))
    .unwrap()
}

fn copy_fixture_to(dir: &Path, name: &str) -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let src = Path::new(manifest)
        .join("..")
        .join("ats-core")
        .join("tests")
        .join("fixtures")
        .join("optimize")
        .join(name);
    let dest = dir.join(name);
    fs::copy(&src, &dest).unwrap_or_else(|e| {
        panic!("copy {} -> {}: {}", src.display(), dest.display(), e);
    });
    dest
}

fn find_optimize_run(root: &Path) -> std::path::PathBuf {
    let runs = root.join("runs");
    let entries: Vec<_> = fs::read_dir(&runs)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains("_optimize"))
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "expected exactly one optimize run folder in {}",
        runs.display()
    );
    entries[0].path()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn optimize_happy_path_writes_artifacts() {
    let server = MockServer::start().await;
    let body: Value = serde_json::from_str(&chat_body_markdown()).unwrap();
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&body))
        .mount(&server)
        .await;

    let tmp = tempdir().unwrap();
    write_config(tmp.path(), &server.uri());
    let resume = copy_fixture_to(tmp.path(), "baseline.md");
    let keywords = copy_fixture_to(tmp.path(), "keywords.json");

    let assert = Command::cargo_bin("ats")
        .unwrap()
        .env("ATS_BINARY_DIR", tmp.path())
        .arg("optimize")
        .arg("--resume")
        .arg(resume)
        .arg("--keywords")
        .arg(keywords)
        .assert()
        .code(0);

    let out = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(out.contains("Jane Doe"), "stdout should be optimized md: {out}");
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("[optimize] tokens:"),
        "expected stderr token line: {stderr}"
    );
    assert!(
        stderr.contains("Keyword density:"),
        "expected human density line: {stderr}"
    );

    let run_dir = find_optimize_run(tmp.path());
    assert!(run_dir.join("optimized.md").is_file());
    assert!(run_dir.join("llm-audit.jsonl").is_file());
    let run_json: Value =
        serde_json::from_str(&fs::read_to_string(run_dir.join("run.json")).unwrap()).unwrap();
    assert_eq!(run_json["token_usage_total"]["total"], 30);
    let kd = &run_json["keyword_density"];
    assert!(kd["value"].is_number());
    assert!(kd["numerator"].is_number());
    assert!(kd["denominator"].is_number());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_keywords_exits_six() {
    let tmp = tempdir().unwrap();
    write_config(tmp.path(), "https://example.com");
    let resume = copy_fixture_to(tmp.path(), "baseline.md");
    let bad = tmp.path().join("bad.json");
    fs::write(&bad, json!({ "nope": 1 }).to_string()).unwrap();

    let assert = Command::cargo_bin("ats")
        .unwrap()
        .env("ATS_BINARY_DIR", tmp.path())
        .arg("optimize")
        .arg("--resume")
        .arg(resume)
        .arg("--keywords")
        .arg(&bad)
        .assert()
        .code(6);

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.to_lowercase().contains("ats_keyword_extraction")
            || stderr.to_lowercase().contains("schema"),
        "expected schema hint: {stderr}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_stdin_sources_exit_two() {
    let tmp = tempdir().unwrap();
    write_config(
        tmp.path(),
        "https://example.com",
    );

    Command::cargo_bin("ats")
        .unwrap()
        .env("ATS_BINARY_DIR", tmp.path())
        .arg("optimize")
        .arg("--resume")
        .arg("-")
        .arg("--keywords")
        .arg("-")
        .assert()
        .code(2);
}
