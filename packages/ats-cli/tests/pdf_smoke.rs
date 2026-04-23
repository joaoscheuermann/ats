//! End-to-end smoke tests for the `ats pdf` subcommand (US-5, Effort 06).

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
fn pdf_happy_path_writes_magic_bytes_and_exits_zero() {
    let tmp = tempdir().unwrap();
    let config_path = tmp.path().join("config.json");
    fs::write(&config_path, sample_config_json()).unwrap();
    let out = tmp.path().join("ok.pdf");

    Command::cargo_bin("ats")
        .unwrap()
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "pdf",
            "--out",
            out.to_str().unwrap(),
        ])
        .write_stdin("# Hello\n\nWorld.\n")
        .assert()
        .code(0)
        .stdout(predicate::str::is_empty());
    let bytes = fs::read(&out).expect("pdf file");
    assert!(bytes.len() > 200, "pdf should be non-trivial");
    assert_eq!(&bytes[..5], b"%PDF-");
}

#[test]
fn pdf_unwritable_parent_exits_seven_and_class_is_pdf() {
    let tmp = tempdir().unwrap();
    let config_path = tmp.path().join("config.json");
    fs::write(&config_path, sample_config_json()).unwrap();
    // Parent directory does not exist — `markdown2pdf::parse_into_file`
    // surfaces this as an IoError that we map to `PdfError::Render` →
    // `AtsError::Pdf` → exit 7.
    let out = tmp.path().join("no/such/dir/out.pdf");

    Command::cargo_bin("ats")
        .unwrap()
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "pdf",
            "--out",
            out.to_str().unwrap(),
        ])
        .write_stdin("# Hello\n")
        .assert()
        .code(7)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("class=\"pdf\"").or(predicate::str::contains("pdf error")));
    assert!(!out.exists(), "no pdf should have been written");
}
