//! Golden-file tests: parse each fixture YAML, render Markdown, and compare
//! to the matching `.md` fixture. Set `UPDATE_GOLDENS=1` to regenerate the
//! expected files after intentional template changes.

use std::fs;
use std::path::{Path, PathBuf};

use ats_core::render::{render_baseline, validate::parse_and_validate};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn golden(name_yaml: &str, name_md: &str) {
    let dir = fixtures_dir();
    let yaml_bytes = fs::read(dir.join(name_yaml))
        .unwrap_or_else(|e| panic!("read {name_yaml}: {e}"));
    let resume = parse_and_validate(&yaml_bytes)
        .unwrap_or_else(|e| panic!("parse+validate {name_yaml}: {e}"));
    let actual = render_baseline(&resume);
    let expected_path = dir.join(name_md);

    if std::env::var("UPDATE_GOLDENS").is_ok() {
        fs::write(&expected_path, &actual).expect("write golden");
        return;
    }

    // Fixtures may be checked out with either LF or CRLF line endings
    // depending on `core.autocrlf`. The renderer always emits LF, so we
    // normalise the expected text before comparing.
    let expected_raw = fs::read_to_string(&expected_path)
        .unwrap_or_else(|e| panic!("read {name_md}: {e}"));
    let expected = expected_raw.replace("\r\n", "\n");
    assert_eq!(
        actual, expected,
        "render output differs from {name_md}; rerun with UPDATE_GOLDENS=1 to regenerate"
    );
}

#[test]
fn minimal_golden() {
    golden("minimal.yaml", "minimal.md");
}

#[test]
fn full_golden() {
    golden("full.yaml", "full.md");
}

#[test]
fn with_present_golden() {
    golden("with-present.yaml", "with-present.md");
}

#[test]
fn invalid_missing_summary_is_rejected_with_path() {
    let dir = fixtures_dir();
    let bytes = fs::read(dir.join("invalid-missing-summary.yaml")).unwrap();
    let err = parse_and_validate(&bytes).unwrap_err();
    let rendered = err.to_string();
    assert!(
        rendered.contains("cv.professional_summary")
            || rendered.contains("professional_summary"),
        "expected path to name cv.professional_summary, got: {rendered}"
    );
}

#[test]
fn invalid_bad_date_type_is_rejected_with_indexed_path() {
    let dir = fixtures_dir();
    let bytes = fs::read(dir.join("invalid-bad-date-type.yaml")).unwrap();
    let err = parse_and_validate(&bytes).unwrap_err();
    let rendered = err.to_string();
    assert!(
        rendered.contains("cv.work_experience[1].start_date"),
        "expected path cv.work_experience[1].start_date, got: {rendered}"
    );
}
