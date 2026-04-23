//! End-to-end smoke tests for `ats scrape` against a wiremock-backed
//! OpenRouter stand-in.
//!
//! Chromium is **not** required — the CLI honours the `ATS_SCRAPE_STUB_HTML_FILE`
//! environment variable and swaps the production `ChromiumScraper` for a
//! stub scraper that returns the file's contents verbatim. When the file
//! contents start with `error:<class>` the stub returns a matching
//! `ScrapeError` so we can test the full error-classification matrix
//! hermetically.
//!
//! The tests cover:
//!
//! * happy path — JSON on stdout, `posting.json` + `posting.md` +
//!   `llm-audit.jsonl` + `run.json` written, run folder renamed with slug.
//! * LLM returns non-JSON — exit 1 / `AtsError::Other`, run folder retains
//!   the unslug name, `run.json.outcome` starts with `"other"`.
//! * scraper returns `NotFound` — exit 4, folder unrenamed,
//!   `run.json.outcome` = `"scrape"` (short-form class tag).

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::tempdir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const STUB_ENV: &str = "ATS_SCRAPE_STUB_HTML_FILE";

fn write_config(dir: &Path, base_url: &str) {
    let cfg = format!(
        r#"{{
  "openrouter": {{ "api_key": "test-key", "base_url": "{base_url}" }},
  "models": {{
    "scrape_to_markdown":  {{ "name": "x/y", "temperature": 0.0, "seed": 42 }},
    "keyword_extraction":  {{ "name": "x/y", "temperature": 0.0, "seed": 42 }},
    "resume_optimization": {{ "name": "x/y", "temperature": 0.2, "seed": 42 }}
  }},
  "scrape": {{ "network_idle_timeout_ms": 1000 }},
  "retries": {{
    "llm_transient_max_attempts": 5,
    "llm_transient_backoff_ms": [0, 0, 0, 0, 0],
    "schema_validation_max_attempts": 3
  }}
}}"#
    );
    fs::write(dir.join("config.json"), cfg).unwrap();
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
            "prompt_tokens": 1200,
            "completion_tokens": 400,
            "total_tokens": 1600
        }
    })
}

fn find_scrape_run(root: &Path) -> PathBuf {
    let runs = root.join("runs");
    let entries: Vec<_> = fs::read_dir(&runs)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .contains("_scrape")
        })
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "expected exactly one scrape run folder in {}",
        runs.display()
    );
    entries[0].path()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scrape_happy_path_writes_artifacts_and_renames_with_slug() {
    let server = MockServer::start().await;
    let llm_content =
        "{\"title\":\"Senior Rust Engineer\",\"markdown\":\"## About\\n- ship it\"}";
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_body(llm_content)))
        .mount(&server)
        .await;

    let tmp = tempdir().unwrap();
    write_config(tmp.path(), &server.uri());
    let stub = tmp.path().join("stub.html");
    fs::write(&stub, "<html><body><h1>Senior Rust Engineer</h1></body></html>").unwrap();

    let assert = Command::cargo_bin("ats")
        .unwrap()
        .env("ATS_BINARY_DIR", tmp.path())
        .env(STUB_ENV, &stub)
        .arg("scrape")
        .arg("https://example.test/job")
        .assert()
        .code(0);

    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Value = serde_json::from_str(&stdout).expect("stdout should be JSON");
    assert_eq!(parsed["title"], "Senior Rust Engineer");
    assert!(parsed["markdown"]
        .as_str()
        .unwrap()
        .contains("## About"));

    let run_dir = find_scrape_run(tmp.path());
    let name = run_dir.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.contains("_scrape_senior-rust-engineer"),
        "run folder should carry slug, got: {name}"
    );
    assert!(run_dir.join("posting.json").is_file());
    assert!(run_dir.join("posting.md").is_file());
    assert!(run_dir.join("llm-audit.jsonl").is_file());
    assert!(run_dir.join("run.json").is_file());

    let run_json: Value =
        serde_json::from_str(&fs::read_to_string(run_dir.join("run.json")).unwrap()).unwrap();
    assert_eq!(run_json["outcome"], "success");
    assert_eq!(run_json["exit_code"], 0);
    assert_eq!(run_json["token_usage_total"]["total"], 1600);
    assert_eq!(run_json["command"], "scrape");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scrape_non_json_llm_response_is_exit_one_other() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_body("not json at all")))
        .mount(&server)
        .await;

    let tmp = tempdir().unwrap();
    write_config(tmp.path(), &server.uri());
    let stub = tmp.path().join("stub.html");
    fs::write(&stub, "<html>ok</html>").unwrap();

    Command::cargo_bin("ats")
        .unwrap()
        .env("ATS_BINARY_DIR", tmp.path())
        .env(STUB_ENV, &stub)
        .arg("scrape")
        .arg("https://example.test/job")
        .assert()
        .code(1);

    let run_dir = find_scrape_run(tmp.path());
    let run_json: Value =
        serde_json::from_str(&fs::read_to_string(run_dir.join("run.json")).unwrap()).unwrap();
    assert_eq!(run_json["outcome"], "other");
    assert_eq!(run_json["exit_code"], 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scrape_not_found_exits_four_without_slug() {
    let tmp = tempdir().unwrap();
    write_config(tmp.path(), "http://127.0.0.1:1");
    let stub = tmp.path().join("stub.html");
    fs::write(&stub, "error:not-found").unwrap();

    Command::cargo_bin("ats")
        .unwrap()
        .env("ATS_BINARY_DIR", tmp.path())
        .env(STUB_ENV, &stub)
        .arg("scrape")
        .arg("https://example.test/missing")
        .assert()
        .code(4);

    let run_dir = find_scrape_run(tmp.path());
    let name = run_dir.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.ends_with("_scrape"),
        "folder should not carry a slug when scrape fails, got: {name}"
    );

    let run_json: Value =
        serde_json::from_str(&fs::read_to_string(run_dir.join("run.json")).unwrap()).unwrap();
    assert_eq!(run_json["outcome"], "scrape/not-found");
    assert_eq!(run_json["exit_code"], 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scrape_offline_exits_four_with_offline_class() {
    let tmp = tempdir().unwrap();
    write_config(tmp.path(), "http://127.0.0.1:1");
    let stub = tmp.path().join("stub.html");
    fs::write(&stub, "error:offline").unwrap();

    Command::cargo_bin("ats")
        .unwrap()
        .env("ATS_BINARY_DIR", tmp.path())
        .env(STUB_ENV, &stub)
        .arg("scrape")
        .arg("https://example.test")
        .assert()
        .code(4);

    let run_dir = find_scrape_run(tmp.path());
    let run_json: Value =
        serde_json::from_str(&fs::read_to_string(run_dir.join("run.json")).unwrap()).unwrap();
    assert_eq!(run_json["exit_code"], 4);
    assert_eq!(run_json["outcome"], "scrape/offline");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scrape_browser_missing_exits_four() {
    let tmp = tempdir().unwrap();
    write_config(tmp.path(), "http://127.0.0.1:1");
    let stub = tmp.path().join("stub.html");
    fs::write(&stub, "error:browser-missing").unwrap();

    Command::cargo_bin("ats")
        .unwrap()
        .env("ATS_BINARY_DIR", tmp.path())
        .env(STUB_ENV, &stub)
        .arg("scrape")
        .arg("https://example.test")
        .assert()
        .code(4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scrape_http_418_exits_four() {
    let tmp = tempdir().unwrap();
    write_config(tmp.path(), "http://127.0.0.1:1");
    let stub = tmp.path().join("stub.html");
    fs::write(&stub, "error:http-418").unwrap();

    Command::cargo_bin("ats")
        .unwrap()
        .env("ATS_BINARY_DIR", tmp.path())
        .env(STUB_ENV, &stub)
        .arg("scrape")
        .arg("https://example.test")
        .assert()
        .code(4);
}
