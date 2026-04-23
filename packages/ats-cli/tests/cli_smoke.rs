//! End-to-end smoke tests for the `ats` binary. These drive the actual
//! release-shaped binary (`assert_cmd`), so they exercise the same
//! config-loading and run-folder plumbing the Effort's verification criteria
//! check against the release binary by hand.

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

fn sample_config_json() -> &'static str {
    r#"{
  "openrouter": { "api_key": "secret", "base_url": "https://openrouter.ai/api/v1" },
  "models": {
    "scrape_to_markdown":  { "name": "x/y", "temperature": 0.0, "seed": 42 },
    "keyword_extraction":  { "name": "x/y", "temperature": 0.0, "seed": 42 },
    "resume_optimization": { "name": "x/y", "temperature": 0.2, "seed": 42 }
  },
  "scrape": { "network_idle_timeout_ms": 30000 },
  "retries": {
    "llm_transient_max_attempts": 5,
    "llm_transient_backoff_ms": [1000, 2000, 4000, 8000, 16000],
    "schema_validation_max_attempts": 3
  }
}"#
}

#[test]
fn help_lists_all_subcommands() {
    let mut cmd = Command::cargo_bin("ats").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("render"))
        .stdout(predicate::str::contains("scrape"))
        .stdout(predicate::str::contains("keywords"))
        .stdout(predicate::str::contains("optimize"))
        .stdout(predicate::str::contains("pdf"))
        .stdout(predicate::str::contains("run"))
        .stdout(predicate::str::contains("--log-format"))
        .stdout(predicate::str::contains("--config"));
}

#[test]
fn missing_config_returns_exit_two_with_named_path() {
    let tmp = tempdir().unwrap();
    let missing = tmp.path().join("nope.json");
    Command::cargo_bin("ats")
        .unwrap()
        .args([
            "--config",
            missing.to_str().unwrap(),
            "render",
            "--yaml",
            "whatever.yaml",
        ])
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("config.json not found at"));
}

#[test]
fn pdf_renders_empty_markdown_exits_zero() {
    let tmp = tempdir().unwrap();
    let config_path = tmp.path().join("config.json");
    fs::write(&config_path, sample_config_json()).unwrap();
    let out = tmp.path().join("out.pdf");

    Command::cargo_bin("ats")
        .unwrap()
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "pdf",
            "--out",
            out.to_str().unwrap(),
        ])
        .assert()
        .code(0)
        .stdout(predicate::str::is_empty());
    let bytes = fs::read(&out).expect("read pdf");
    assert!(bytes.len() > 4, "pdf should be non-trivial");
    assert_eq!(&bytes[..5], b"%PDF-");
}

#[test]
fn pretty_log_format_still_succeeds_for_pdf() {
    let tmp = tempdir().unwrap();
    let config_path = tmp.path().join("config.json");
    fs::write(&config_path, sample_config_json()).unwrap();
    let out = tmp.path().join("out.pdf");

    Command::cargo_bin("ats")
        .unwrap()
        .args([
            "--log-format",
            "pretty",
            "--config",
            config_path.to_str().unwrap(),
            "pdf",
            "--out",
            out.to_str().unwrap(),
        ])
        .assert()
        .code(0)
        .stdout(predicate::str::is_empty());
}
