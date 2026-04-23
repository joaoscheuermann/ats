//! `ats scrape <URL>` handler (US-2).
//!
//! Launches the chromiumoxide scraper, hands the rendered HTML to the LLM
//! for structured `{title, markdown}` extraction, writes
//! `posting.json` into the run folder, renames the run folder to
//! `<ts>_scrape_<slug>/` once the title is known, and mirrors the same
//! JSON to stdout.

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ats_core::audit::RunFolder;
use ats_core::config::Config;
use ats_core::sanitize_title;
use ats_core::scrape_port::{PageScraper, ScrapeError};
use ats_core::stage::scrape::{fetch_and_convert, ScrapeStageConfig};
use ats_core::{AtsError, AuditSink};
use ats_llm::{CompositeAuditSink, FileAuditSink, OpenRouterClient};
use ats_scrape::ChromiumScraper;

use crate::observer::StderrUsageReporter;

/// Test-only environment variable pointing at a file whose contents are
/// returned verbatim by a stub scraper in place of a real Chromium launch.
/// Lets end-to-end tests drive the full CLI pipeline without Chrome.
const STUB_HTML_ENV: &str = "ATS_SCRAPE_STUB_HTML_FILE";

/// Entry point dispatched from `main.rs`. Uses the production
/// [`ChromiumScraper`] unless the [`STUB_HTML_ENV`] test seam is set.
pub async fn handle(
    cfg: &Config,
    run_folder: &mut RunFolder,
    url: &str,
    stdout: &mut dyn Write,
) -> Result<(), AtsError> {
    let idle_timeout = Duration::from_millis(cfg.scrape.network_idle_timeout_ms);

    if let Some(path) = std::env::var_os(STUB_HTML_ENV) {
        let stub = StubScraper { path: path.into() };
        return handle_with_scraper(
            cfg,
            run_folder,
            &stub,
            url,
            stdout,
            Box::new(io::stderr()) as Box<dyn Write + Send>,
        )
        .await;
    }

    let scraper = ChromiumScraper::new(idle_timeout);
    handle_with_scraper(
        cfg,
        run_folder,
        &scraper,
        url,
        stdout,
        Box::new(io::stderr()) as Box<dyn Write + Send>,
    )
    .await
}

/// Stub scraper used by integration tests via the [`STUB_HTML_ENV`] seam.
/// Returns the file contents verbatim (or a `ScrapeError` derived from the
/// path prefix `error:<class>` for negative-path tests).
struct StubScraper {
    path: PathBuf,
}

#[async_trait]
impl PageScraper for StubScraper {
    async fn fetch_html(
        &self,
        _url: &str,
        _idle_timeout: Duration,
    ) -> Result<String, ScrapeError> {
        let raw = std::fs::read_to_string(&self.path)
            .map_err(|err| ScrapeError::Other(format!("stub html file read failed: {err}")))?;
        if let Some(rest) = raw.strip_prefix("error:") {
            return Err(stub_error(rest.trim()));
        }
        Ok(raw)
    }
}

fn stub_error(tag: &str) -> ScrapeError {
    match tag {
        "offline" => ScrapeError::Offline("stub: offline".into()),
        "network-timeout" => ScrapeError::NetworkTimeout("stub: network-timeout".into()),
        "timeout" => ScrapeError::Timeout("stub: timeout".into()),
        "auth-required" => ScrapeError::AuthRequired { status: 401 },
        "not-found" => ScrapeError::NotFound { status: 404 },
        "geo-blocked" => ScrapeError::GeoBlocked { status: 451 },
        "http-418" => ScrapeError::Http { status: 418 },
        "browser-missing" => ScrapeError::BrowserMissing("stub: no Chrome".into()),
        other => ScrapeError::Other(format!("stub: {other}")),
    }
}

/// Scraper-injectable variant used by tests that stub `PageScraper` to keep
/// the full command path hermetic.
pub async fn handle_with_scraper(
    cfg: &Config,
    run_folder: &mut RunFolder,
    scraper: &dyn PageScraper,
    url: &str,
    stdout: &mut dyn Write,
    reporter_writer: Box<dyn Write + Send>,
) -> Result<(), AtsError> {
    let audit_path = run_folder.path().join("llm-audit.jsonl");

    // Scope the audit sinks + OpenRouter client so their file handles are
    // released (and Windows is willing to rename the directory) before we
    // touch the run folder's name again. `fetch_and_convert` has already
    // flushed the audit log at this point.
    let stage_outcome = {
        let file_sink = Arc::new(FileAuditSink::create(&audit_path).map_err(AtsError::Io)?);
        let reporter = Arc::new(StderrUsageReporter::new("scrape", reporter_writer));
        let sinks: Vec<Arc<dyn AuditSink>> = vec![file_sink, reporter];
        let composite = Arc::new(CompositeAuditSink::new(sinks));

        let client = OpenRouterClient::new(
            &cfg.openrouter,
            &cfg.retries,
            composite.clone() as Arc<dyn AuditSink>,
        )
        .map_err(|err| AtsError::Other(format!("failed to build OpenRouter client: {err}")))?;

        let stage_cfg = ScrapeStageConfig {
            idle_timeout: Duration::from_millis(cfg.scrape.network_idle_timeout_ms),
            model: cfg.models.scrape_to_markdown.clone(),
        };

        fetch_and_convert(scraper, &client, url, &stage_cfg).await?
    };

    run_folder.add_token_usage(&stage_outcome.usage);
    run_folder
        .set_extra("token_usage_total", stage_outcome.usage)
        .map_err(|e| AtsError::Other(format!("failed to record token_usage_total: {e}")))?;

    let slug = sanitize_title(&stage_outcome.posting.title);
    if let Err(err) = run_folder.rename_with_slug(&slug) {
        tracing::warn!(%err, slug=%slug, "failed to rename scrape run folder with slug");
    }

    let posting_json =
        serde_json::to_string_pretty(&PostingPayload::from(&stage_outcome.posting))
            .map_err(|e| AtsError::Other(format!("failed to serialise posting: {e}")))?;

    std::fs::write(run_folder.path().join("posting.json"), &posting_json).map_err(AtsError::Io)?;
    std::fs::write(
        run_folder.path().join("posting.md"),
        stage_outcome.posting.markdown.as_bytes(),
    )
    .map_err(AtsError::Io)?;

    write_output(stdout, posting_json.as_bytes())
}

#[derive(serde::Serialize)]
struct PostingPayload<'a> {
    title: &'a str,
    markdown: &'a str,
}

impl<'a> From<&'a ats_core::JobPosting> for PostingPayload<'a> {
    fn from(p: &'a ats_core::JobPosting) -> Self {
        Self {
            title: &p.title,
            markdown: &p.markdown,
        }
    }
}

fn write_output(writer: &mut dyn Write, bytes: &[u8]) -> Result<(), AtsError> {
    match writer
        .write_all(bytes)
        .and_then(|()| writer.write_all(b"\n"))
        .and_then(|()| writer.flush())
    {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(err) => Err(AtsError::Io(err)),
    }
}
