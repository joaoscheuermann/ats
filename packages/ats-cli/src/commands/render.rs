//! `ats render --yaml <file>` handler (US-1 / AC-1.x).
//!
//! Reads YAML bytes, validates them against the embedded JSON Schema, then
//! either loads the matching baseline Markdown from the content-addressed
//! cache or renders it fresh and writes it back. The rendered Markdown is
//! streamed to the caller-provided writer (stdout in production, an
//! in-memory buffer in tests); the `cached` flag and cache path are attached
//! to the invocation's `run.json` so operators can tell at a glance whether
//! a given run re-rendered or simply read from cache.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use ats_core::audit::RunFolder;
use ats_core::render::{cache, validate};
use ats_core::{AtsError, FsLayout, YamlDiag};

/// Parsed `ats render` arguments. Borrows the path so the caller keeps
/// ownership of the [`std::path::PathBuf`] from clap.
pub struct RenderArgs<'a> {
    pub yaml_path: &'a Path,
}

/// Execute `ats render`. `writer` receives the rendered Markdown; the caller
/// maps any returned [`AtsError`] to an exit code and finalises the run
/// folder.
pub fn handle<W: Write>(
    args: &RenderArgs<'_>,
    layout: &dyn FsLayout,
    run_folder: &mut RunFolder,
    mut writer: W,
) -> Result<(), AtsError> {
    let yaml_bytes = read_yaml(args.yaml_path)?;
    let resume = validate::parse_and_validate(&yaml_bytes)?;
    let outcome = cache::load_or_render(layout, &yaml_bytes, &resume)?;

    // Record the cache result in run.json before any stdout IO that could
    // fail — the audit trail stays honest even if the output pipe breaks
    // (e.g. under `ats render | head`).
    run_folder
        .set_extra("cached", outcome.cached)
        .map_err(|e| AtsError::Other(format!("failed to record cache flag: {e}")))?;
    run_folder
        .set_extra("cache_path", outcome.path.display().to_string())
        .map_err(|e| AtsError::Other(format!("failed to record cache path: {e}")))?;

    write_output(&mut writer, outcome.content.as_bytes())?;

    tracing::info!(
        target: "ats::render",
        cached = outcome.cached,
        path = %outcome.path.display(),
        "baseline rendered"
    );
    Ok(())
}

/// Read the YAML file, mapping `NotFound` to a YAML diagnostic (exit 3) so
/// the user sees the offending path rather than a generic IO error.
fn read_yaml(path: &Path) -> Result<Vec<u8>, AtsError> {
    match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err(AtsError::Yaml(YamlDiag {
            path: None,
            reason: format!("yaml file not found: {}", path.display()),
            line: None,
            column: None,
        })),
        Err(err) => Err(AtsError::Io(err)),
    }
}

/// Write the rendered Markdown with a single `write_all` + flush. Broken
/// pipes (consumer closed early, e.g. `| head`) are treated as success —
/// the content was produced; the reader chose to stop.
fn write_output<W: Write>(writer: &mut W, bytes: &[u8]) -> Result<(), AtsError> {
    match writer.write_all(bytes).and_then(|()| writer.flush()) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(err) => Err(AtsError::Io(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ats_core::audit::{RunFolder, RunOutcome};
    use ats_core::{BinaryFsLayout, Clock, SystemClock};
    use std::fs;
    use tempfile::tempdir;

    fn sample_yaml() -> &'static str {
        r#"
cv:
  personal_information:
    full_name: Jane Doe
    email: jane@example.com
    phone: "+1 555-0100"
    linkedin_url: https://linkedin.com/in/jane
    location: Remote
  professional_summary: Experienced engineer.
"#
    }

    fn make_folder(layout: &BinaryFsLayout, clock: &dyn Clock) -> RunFolder {
        RunFolder::new(layout, clock, "render", None).unwrap()
    }

    #[test]
    fn first_call_writes_markdown_and_records_cached_false() {
        let tmp = tempdir().unwrap();
        let layout = BinaryFsLayout::new_rooted_at(tmp.path());
        let yaml = tmp.path().join("resume.yaml");
        fs::write(&yaml, sample_yaml()).unwrap();

        let clock = SystemClock;
        let mut folder = make_folder(&layout, &clock);
        let mut buf: Vec<u8> = Vec::new();
        handle(
            &RenderArgs { yaml_path: &yaml },
            &layout,
            &mut folder,
            &mut buf,
        )
        .unwrap();
        let rendered = String::from_utf8(buf).unwrap();
        assert!(rendered.starts_with("# Jane Doe\n"));
        assert!(rendered.contains("## Professional Summary\nExperienced engineer."));

        folder.finalize(&clock, RunOutcome::Success, 0).unwrap();
        let body: serde_json::Value =
            serde_json::from_slice(&fs::read(folder.path().join("run.json")).unwrap()).unwrap();
        assert_eq!(body["cached"], false);
        assert!(
            body["cache_path"]
                .as_str()
                .unwrap()
                .contains("baseline-"),
            "cache_path must point inside cache/: {body}"
        );
    }

    #[test]
    fn second_call_hits_cache_and_records_cached_true() {
        let tmp = tempdir().unwrap();
        let layout = BinaryFsLayout::new_rooted_at(tmp.path());
        let yaml = tmp.path().join("resume.yaml");
        fs::write(&yaml, sample_yaml()).unwrap();

        let clock = SystemClock;
        let mut first = make_folder(&layout, &clock);
        handle(
            &RenderArgs { yaml_path: &yaml },
            &layout,
            &mut first,
            &mut Vec::new(),
        )
        .unwrap();

        let mut second = make_folder(&layout, &clock);
        let mut out = Vec::new();
        handle(
            &RenderArgs { yaml_path: &yaml },
            &layout,
            &mut second,
            &mut out,
        )
        .unwrap();
        second.finalize(&clock, RunOutcome::Success, 0).unwrap();
        let body: serde_json::Value =
            serde_json::from_slice(&fs::read(second.path().join("run.json")).unwrap()).unwrap();
        assert_eq!(body["cached"], true);
    }

    #[test]
    fn missing_yaml_returns_yaml_error() {
        let tmp = tempdir().unwrap();
        let layout = BinaryFsLayout::new_rooted_at(tmp.path());
        let mut folder = make_folder(&layout, &SystemClock);
        let err = handle(
            &RenderArgs {
                yaml_path: &tmp.path().join("nope.yaml"),
            },
            &layout,
            &mut folder,
            &mut Vec::new(),
        )
        .unwrap_err();
        assert_eq!(err.exit_code(), 3);
        assert!(err.to_string().contains("yaml file not found"));
    }

    #[test]
    fn invalid_yaml_missing_summary_returns_exit_three_and_names_path() {
        let tmp = tempdir().unwrap();
        let layout = BinaryFsLayout::new_rooted_at(tmp.path());
        let yaml = tmp.path().join("bad.yaml");
        fs::write(
            &yaml,
            r#"
cv:
  personal_information:
    full_name: Jane Doe
    email: jane@example.com
    phone: "+1"
    linkedin_url: https://x
    location: Remote
"#,
        )
        .unwrap();
        let mut folder = make_folder(&layout, &SystemClock);
        let err = handle(
            &RenderArgs { yaml_path: &yaml },
            &layout,
            &mut folder,
            &mut Vec::new(),
        )
        .unwrap_err();
        assert_eq!(err.exit_code(), 3);
        assert!(
            err.to_string().contains("cv.professional_summary"),
            "expected `cv.professional_summary` in diagnostic: {err}"
        );
    }
}
