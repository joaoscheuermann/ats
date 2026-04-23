//! End-to-end smoke tests for `ats render`. We copy the test binary into a
//! scratch directory so the run folders and cache files land under our
//! tempdir (the CLI resolves everything relative to `current_exe()`), then
//! drive the real binary via `assert_cmd`.

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::{tempdir, TempDir};

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

fn core_fixtures_dir() -> PathBuf {
    // `CARGO_MANIFEST_DIR` for this test is `packages/ats-cli`. The YAML/MD
    // goldens live under the sibling `ats-core` crate's test fixtures.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("ats-core/tests/fixtures")
}

struct StagedBinary {
    _dir: TempDir,
    exe: PathBuf,
    root: PathBuf,
}

fn stage_binary() -> StagedBinary {
    let src = assert_cmd::cargo::cargo_bin("ats");
    let dir = tempdir().unwrap();
    let file_name = src.file_name().expect("ats binary must have a filename");
    let dest = dir.path().join(file_name);
    fs::copy(&src, &dest).expect("copy ats binary into tempdir");
    fs::write(dir.path().join("config.json"), sample_config_json()).unwrap();
    StagedBinary {
        root: dir.path().to_path_buf(),
        exe: dest,
        _dir: dir,
    }
}

fn latest_run_dir(stage: &StagedBinary, suffix: &str) -> PathBuf {
    let runs = stage.root.join("runs");
    let mut matches: Vec<_> = fs::read_dir(&runs)
        .expect("runs dir must exist after a run")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with(suffix))
                .unwrap_or(false)
        })
        .collect();
    matches.sort();
    matches
        .into_iter()
        .last()
        .unwrap_or_else(|| panic!("no run directory ending in {suffix}"))
}

fn read_run_json(run_dir: &Path) -> serde_json::Value {
    let bytes = fs::read(run_dir.join("run.json")).expect("run.json must be present");
    serde_json::from_slice(&bytes).expect("run.json must be valid JSON")
}

/// Fixtures may be checked out with CRLF depending on `core.autocrlf`; the
/// renderer always emits LF, so we normalise the expected bytes before
/// byte-level comparison.
fn lf(s: String) -> String {
    s.replace("\r\n", "\n")
}

#[test]
fn render_full_yaml_matches_golden_and_caches() {
    let stage = stage_binary();
    let fixtures = core_fixtures_dir();
    let yaml_path = fixtures.join("full.yaml");
    let expected_md = lf(fs::read_to_string(fixtures.join("full.md")).unwrap());

    // First invocation: cache miss.
    let assert1 = Command::new(&stage.exe)
        .args(["render", "--yaml", yaml_path.to_str().unwrap()])
        .assert()
        .success();
    let stdout1 = String::from_utf8(assert1.get_output().stdout.clone()).unwrap();
    // Normalise potential CRLF injected by Windows shells; our renderer uses
    // `\n` only, and `assert_cmd` preserves stream bytes, so just compare.
    assert_eq!(stdout1, expected_md, "stdout must match full.md byte-for-byte");

    let run1 = latest_run_dir(&stage, "_render");
    let body1 = read_run_json(&run1);
    assert_eq!(body1["cached"], false, "first run must record cached=false");
    assert_eq!(body1["command"], "render");

    let cache_dir = stage.root.join("cache");
    let mut cache_files: Vec<_> = fs::read_dir(&cache_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    cache_files.sort();
    assert_eq!(cache_files.len(), 1, "expected one cache file after first render");

    // Second invocation: cache hit.
    std::thread::sleep(std::time::Duration::from_millis(1100)); // ensure a distinct ts.
    let assert2 = Command::new(&stage.exe)
        .args(["render", "--yaml", yaml_path.to_str().unwrap()])
        .assert()
        .success();
    let stdout2 = String::from_utf8(assert2.get_output().stdout.clone()).unwrap();
    assert_eq!(stdout2, expected_md, "stdout must still match full.md on cache hit");

    let run2 = latest_run_dir(&stage, "_render");
    assert_ne!(run1, run2, "each invocation must create its own run folder");
    let body2 = read_run_json(&run2);
    assert_eq!(body2["cached"], true, "second run must record cached=true");

    let cache_files_after: Vec<_> = fs::read_dir(&cache_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    assert_eq!(
        cache_files_after.len(),
        1,
        "cache must still have exactly one file after cache hit"
    );
}

#[test]
fn render_minimal_yaml_omits_optional_sections() {
    let stage = stage_binary();
    let fixtures = core_fixtures_dir();
    let yaml_path = fixtures.join("minimal.yaml");
    let expected_md = lf(fs::read_to_string(fixtures.join("minimal.md")).unwrap());

    let assert = Command::new(&stage.exe)
        .args(["render", "--yaml", yaml_path.to_str().unwrap()])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert_eq!(stdout, expected_md);
    assert!(!stdout.contains("## Skills"));
    assert!(!stdout.contains("## Work Experience"));
    assert!(!stdout.contains("## Education"));
    assert!(!stdout.contains("## Certifications"));
}

#[test]
fn render_missing_summary_yaml_exits_three_with_path_diagnostic() {
    let stage = stage_binary();
    let fixtures = core_fixtures_dir();
    let yaml_path = fixtures.join("invalid-missing-summary.yaml");

    Command::new(&stage.exe)
        .args(["render", "--yaml", yaml_path.to_str().unwrap()])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("professional_summary"));
}

#[test]
fn render_bad_date_type_yaml_exits_three_with_indexed_path() {
    let stage = stage_binary();
    let fixtures = core_fixtures_dir();
    let yaml_path = fixtures.join("invalid-bad-date-type.yaml");

    Command::new(&stage.exe)
        .args(["render", "--yaml", yaml_path.to_str().unwrap()])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("cv.work_experience[1].start_date"));
}
