//! Per-invocation audit folder. Every subcommand materialises a
//! `<binary_dir>/runs/<YYYYMMDD-HHMMSS>_<command>[_<slug>]/` directory at
//! startup, and calls `finalize(outcome, exit_code)` just before process exit
//! so a `run.json` is always written (NFC-19).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value};
use time::format_description::FormatItem;
use time::macros::format_description;
use time::OffsetDateTime;

use crate::fs_layout::FsLayout;
use crate::ports::{Clock, TokenUsage};

const TS_FORMAT: &[FormatItem<'_>] =
    format_description!("[year][month][day]-[hour][minute][second]");

/// `YYYYMMDD-HHMMSS` segment shared by run folder names and final resume artefacts.
pub fn format_run_dir_ts(t: OffsetDateTime) -> io::Result<String> {
    t.format(&TS_FORMAT)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Owns the `runs/<ts>_<command>[_<slug>]/` directory and the metadata that
/// `run.json` will serialise. Constructed once per subcommand invocation.
pub struct RunFolder {
    path: PathBuf,
    runs_dir: PathBuf,
    started_at: OffsetDateTime,
    command: &'static str,
    slug: Option<String>,
    args_summary: Value,
    config_snapshot: Value,
    token_usage_total: TokenUsage,
    extras: Map<String, Value>,
}

/// What the run produced, from the CLI's perspective. Serialised verbatim into
/// `run.json`.
#[derive(Debug, Clone, Copy)]
pub enum RunOutcome<'a> {
    Success,
    Unimplemented,
    Failure(&'a str),
}

impl<'a> RunOutcome<'a> {
    fn as_tag(&self) -> &'a str {
        match self {
            RunOutcome::Success => "success",
            RunOutcome::Unimplemented => "unimplemented",
            RunOutcome::Failure(tag) => tag,
        }
    }
}

impl RunFolder {
    /// Create the run folder. `command` is the static subcommand name
    /// (`"render"`, `"scrape"`, …); `slug` is optional and is appended as
    /// `_<slug>` when present (later Efforts rename to add a slug once the
    /// scraped title is known).
    pub fn new(
        layout: &dyn FsLayout,
        clock: &dyn Clock,
        command: &'static str,
        slug: Option<&str>,
    ) -> io::Result<Self> {
        let started_at = clock.now_local();
        Self::new_with_started_at(layout, started_at, command, slug)
    }

    /// Create the run folder with an explicit `started_at` (used by `ats run`
    /// so the directory timestamp matches the start of the pipeline, not the
    /// time the folder is materialised after the scrape step).
    pub fn new_with_started_at(
        layout: &dyn FsLayout,
        started_at: OffsetDateTime,
        command: &'static str,
        slug: Option<&str>,
    ) -> io::Result<Self> {
        layout.ensure_dirs()?;
        let runs_dir = layout.runs_dir();
        let slug_owned = slug.map(|s| s.to_string());
        let path = runs_dir.join(folder_name(started_at, command, slug_owned.as_deref())?);
        fs::create_dir_all(&path)?;

        Ok(Self {
            path,
            runs_dir,
            started_at,
            command,
            slug: slug_owned,
            args_summary: Value::Null,
            config_snapshot: Value::Null,
            token_usage_total: TokenUsage::ZERO,
            extras: Map::new(),
        })
    }

    /// Absolute path to the run directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Instant the folder was created (`started_at` in `run.json`).
    pub fn started_at(&self) -> OffsetDateTime {
        self.started_at
    }

    /// Attach a structured args summary (serde value) — the CLI calls this
    /// right after parsing so `run.json` records how the user invoked things
    /// without leaking secrets.
    pub fn set_args_summary(&mut self, value: Value) {
        self.args_summary = value;
    }

    /// Attach the redacted config snapshot (`api_key` must already be
    /// masked; callers use `Config::redacted_snapshot`).
    pub fn set_config_snapshot(&mut self, value: Value) {
        self.config_snapshot = value;
    }

    /// Attach an arbitrary metadata field that will be merged into the
    /// top level of `run.json` on [`Self::finalize`]. Later keys shadow
    /// earlier ones. Intended for stage-specific flags such as
    /// `cached: true` on `render`.
    pub fn set_extra<K, V>(&mut self, key: K, value: V) -> Result<(), serde_json::Error>
    where
        K: Into<String>,
        V: Serialize,
    {
        self.extras.insert(key.into(), serde_json::to_value(value)?);
        Ok(())
    }

    /// Fold another LLM call's token usage into the running total. Effort 03
    /// wires this up via `AuditSink`.
    pub fn add_token_usage(&mut self, usage: &TokenUsage) {
        self.token_usage_total.prompt += usage.prompt;
        self.token_usage_total.completion += usage.completion;
        self.token_usage_total.total += usage.total;
    }

    /// Aggregated token usage for `run.json` and pipeline summary lines.
    pub fn aggregated_token_usage(&self) -> TokenUsage {
        self.token_usage_total
    }

    /// Rename the folder to include a slug. Effort 04 calls this once the
    /// scraped job title is known so that `runs/<ts>_scrape/` becomes
    /// `runs/<ts>_scrape_<slug>/` atomically. If the target already exists,
    /// we fall back to the original name — never clobber.
    pub fn rename_with_slug(&mut self, new_slug: &str) -> io::Result<()> {
        if new_slug.is_empty() {
            return Ok(());
        }
        let new_path = self
            .runs_dir
            .join(folder_name(self.started_at, self.command, Some(new_slug))?);
        if new_path == self.path {
            self.slug = Some(new_slug.to_string());
            return Ok(());
        }
        if new_path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "refusing to rename run folder: {} already exists",
                    new_path.display()
                ),
            ));
        }
        fs::rename(&self.path, &new_path)?;
        self.path = new_path;
        self.slug = Some(new_slug.to_string());
        Ok(())
    }

    /// Write `run.json` with outcome + exit code. Called from the CLI's
    /// error/exit plumbing so every invocation leaves behind a summary.
    pub fn finalize(&self, clock: &dyn Clock, outcome: RunOutcome<'_>, exit_code: i32) -> io::Result<()> {
        let finished_at = clock.now_local();
        let body = RunJson {
            started_at: format_timestamp(self.started_at),
            finished_at: format_timestamp(finished_at),
            command: self.command,
            slug: self.slug.as_deref(),
            args_summary: &self.args_summary,
            outcome: outcome.as_tag(),
            exit_code,
            config_snapshot: &self.config_snapshot,
            token_usage_total: self.token_usage_total,
        };
        let mut value = serde_json::to_value(&body)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if !self.extras.is_empty() {
            if let Value::Object(map) = &mut value {
                for (k, v) in &self.extras {
                    map.insert(k.clone(), v.clone());
                }
            }
        }
        let bytes = serde_json::to_vec_pretty(&value)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(self.path.join("run.json"), bytes)
    }
}

fn folder_name(
    started_at: OffsetDateTime,
    command: &str,
    slug: Option<&str>,
) -> io::Result<String> {
    let ts = started_at
        .format(&TS_FORMAT)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(match slug {
        Some(s) if !s.is_empty() => format!("{ts}_{command}_{s}"),
        _ => format!("{ts}_{command}"),
    })
}

fn format_timestamp(t: OffsetDateTime) -> String {
    // ISO-8601 with offset is round-trip-safe for humans and machines.
    t.format(&time::format_description::well_known::Iso8601::DEFAULT)
        .unwrap_or_else(|_| t.unix_timestamp().to_string())
}

#[derive(Serialize)]
struct RunJson<'a> {
    started_at: String,
    finished_at: String,
    command: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    slug: Option<&'a str>,
    args_summary: &'a Value,
    outcome: &'a str,
    exit_code: i32,
    config_snapshot: &'a Value,
    token_usage_total: TokenUsage,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs_layout::BinaryFsLayout;
    use serde_json::json;
    use std::sync::Mutex;
    use tempfile::tempdir;
    use time::macros::datetime;

    struct FixedClock(Mutex<OffsetDateTime>);

    impl Clock for FixedClock {
        fn now_local(&self) -> OffsetDateTime {
            *self.0.lock().unwrap()
        }
    }

    fn pinned(t: OffsetDateTime) -> FixedClock {
        FixedClock(Mutex::new(t))
    }

    #[test]
    fn creates_folder_without_slug() {
        let tmp = tempdir().unwrap();
        let layout = BinaryFsLayout::new_rooted_at(tmp.path());
        let clock = pinned(datetime!(2026-04-22 14:30:15 UTC));
        let run = RunFolder::new(&layout, &clock, "render", None).unwrap();
        assert!(run.path().ends_with("20260422-143015_render"));
        assert!(run.path().is_dir());
    }

    #[test]
    fn creates_folder_with_slug() {
        let tmp = tempdir().unwrap();
        let layout = BinaryFsLayout::new_rooted_at(tmp.path());
        let clock = pinned(datetime!(2026-04-22 14:30:15 UTC));
        let run = RunFolder::new(&layout, &clock, "scrape", Some("acme-engineer")).unwrap();
        assert!(run.path().ends_with("20260422-143015_scrape_acme-engineer"));
    }

    #[test]
    fn finalize_writes_run_json_with_snapshot_and_exit_code() {
        let tmp = tempdir().unwrap();
        let layout = BinaryFsLayout::new_rooted_at(tmp.path());
        let clock = pinned(datetime!(2026-04-22 14:30:15 UTC));
        let mut run = RunFolder::new(&layout, &clock, "render", None).unwrap();
        run.set_args_summary(json!({"yaml": "x.yaml"}));
        run.set_config_snapshot(json!({
            "openrouter": {"api_key": "***", "base_url": "https://openrouter.ai/api/v1"}
        }));
        run.finalize(&clock, RunOutcome::Unimplemented, 1).unwrap();

        let body: serde_json::Value =
            serde_json::from_slice(&fs::read(run.path().join("run.json")).unwrap()).unwrap();
        assert_eq!(body["outcome"], "unimplemented");
        assert_eq!(body["exit_code"], 1);
        assert_eq!(body["command"], "render");
        assert_eq!(body["args_summary"]["yaml"], "x.yaml");
        assert_eq!(body["config_snapshot"]["openrouter"]["api_key"], "***");
        assert_eq!(body["token_usage_total"]["total"], 0);
        assert!(body["started_at"].as_str().unwrap().starts_with("2026-04-22"));
    }

    #[test]
    fn rename_with_slug_round_trip() {
        let tmp = tempdir().unwrap();
        let layout = BinaryFsLayout::new_rooted_at(tmp.path());
        let clock = pinned(datetime!(2026-04-22 14:30:15 UTC));
        let mut run = RunFolder::new(&layout, &clock, "scrape", None).unwrap();
        let original = run.path().to_path_buf();
        run.rename_with_slug("acme-engineer").unwrap();
        assert!(!original.exists(), "old directory should be gone");
        assert!(run
            .path()
            .ends_with("20260422-143015_scrape_acme-engineer"));
        assert!(run.path().is_dir());
    }

    #[test]
    fn rename_is_idempotent_when_slug_unchanged() {
        let tmp = tempdir().unwrap();
        let layout = BinaryFsLayout::new_rooted_at(tmp.path());
        let clock = pinned(datetime!(2026-04-22 14:30:15 UTC));
        let mut run = RunFolder::new(&layout, &clock, "scrape", Some("acme")).unwrap();
        let path_before = run.path().to_path_buf();
        run.rename_with_slug("acme").unwrap();
        assert_eq!(run.path(), path_before);
    }

    #[test]
    fn extras_merge_into_run_json_top_level() {
        let tmp = tempdir().unwrap();
        let layout = BinaryFsLayout::new_rooted_at(tmp.path());
        let clock = pinned(datetime!(2026-04-22 14:30:15 UTC));
        let mut run = RunFolder::new(&layout, &clock, "render", None).unwrap();
        run.set_extra("cached", true).unwrap();
        run.set_extra("cache_path", "/tmp/baseline-abc.md").unwrap();
        run.finalize(&clock, RunOutcome::Success, 0).unwrap();
        let body: serde_json::Value =
            serde_json::from_slice(&fs::read(run.path().join("run.json")).unwrap()).unwrap();
        assert_eq!(body["cached"], true);
        assert_eq!(body["cache_path"], "/tmp/baseline-abc.md");
        assert_eq!(body["command"], "render");
    }

    #[test]
    fn token_usage_accumulates() {
        let tmp = tempdir().unwrap();
        let layout = BinaryFsLayout::new_rooted_at(tmp.path());
        let clock = pinned(datetime!(2026-04-22 14:30:15 UTC));
        let mut run = RunFolder::new(&layout, &clock, "keywords", None).unwrap();
        run.add_token_usage(&TokenUsage {
            prompt: 10,
            completion: 5,
            total: 15,
        });
        run.add_token_usage(&TokenUsage {
            prompt: 1,
            completion: 2,
            total: 3,
        });
        run.finalize(&clock, RunOutcome::Success, 0).unwrap();
        let body: serde_json::Value =
            serde_json::from_slice(&fs::read(run.path().join("run.json")).unwrap()).unwrap();
        assert_eq!(body["token_usage_total"]["prompt"], 11);
        assert_eq!(body["token_usage_total"]["completion"], 7);
        assert_eq!(body["token_usage_total"]["total"], 18);
    }
}
