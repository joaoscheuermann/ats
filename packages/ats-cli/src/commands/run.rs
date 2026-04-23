//! `ats run --yaml <yml> <URL>` — full pipeline (US-6).

use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ats_core::audit::RunOutcome;
use ats_core::config::Config;
use ats_core::scrape_port::{PageScraper, ScrapeError};
use ats_core::{
    parse_and_validate, run as run_pipeline, AtsError, BinaryFsLayout, Clock, FsLayout, LlmClass,
    RunPipelineError, RunPipelineInput, StagesBundle, SystemClock, VecAuditSink, YamlDiag,
};
use ats_llm::{CompositeAuditSink, FileAuditSink, OpenRouterClient};
use ats_pdf::Markdown2PdfWriter;
use ats_scrape::ChromiumScraper;
use serde_json::Value;
use time::OffsetDateTime;

use crate::observer::StderrUsageReporter;

const STUB_HTML_ENV: &str = "ATS_SCRAPE_STUB_HTML_FILE";

/// `ats run` with its own `run.json` (no folder is created on scrape failure).
pub async fn handle(
    config: &Config,
    layout: &BinaryFsLayout,
    args_summary: Value,
    config_redacted: Value,
    yaml: &Path,
    url: &str,
) -> i32 {
    if let Err(e) = layout.ensure_dirs() {
        let err = AtsError::from(e);
        tracing::error!(class = err.class(), exit_code = err.exit_code(), error = %err, "ensure_dirs failed");
        return err.exit_code();
    }

    let yaml_bytes: Vec<u8> = match std::fs::read(yaml) {
        Ok(b) => b,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            let e = AtsError::Yaml(YamlDiag {
                path: None,
                reason: format!("yaml file not found: {}", yaml.display()),
                line: None,
                column: None,
            });
            tracing::error!(class = e.class(), exit_code = e.exit_code(), error = %e, "run failed (yaml missing)");
            return e.exit_code();
        }
        Err(e) => {
            let err = AtsError::Io(e);
            tracing::error!(class = err.class(), exit_code = err.exit_code(), error = %err, "run failed (yaml read)");
            return err.exit_code();
        }
    };
    let resume = match parse_and_validate(&yaml_bytes) {
        Ok(r) => r,
        Err(d) => {
            let e = AtsError::Yaml(d);
            tracing::error!(class = e.class(), exit_code = e.exit_code(), error = %e, "run failed (yaml validation)");
            return e.exit_code();
        }
    };

    let clock = SystemClock;
    let started_at: OffsetDateTime = clock.now_local();
    tracing::info!(
        stage = "run",
        url = %url,
        yaml = %yaml.display(),
        "run.started (pre-scrape)"
    );

    let idle = Duration::from_millis(config.scrape.network_idle_timeout_ms);
    let reporter_scrape = Arc::new(StderrUsageReporter::new(
        "scrape",
        Box::new(io::stderr()) as Box<dyn io::Write + Send>,
    ));
    let scrape_buffer = Arc::new(VecAuditSink::new());
    let scrape_sinks: Vec<Arc<dyn ats_core::AuditSink>> = vec![
        scrape_buffer.clone() as Arc<dyn ats_core::AuditSink>,
        reporter_scrape,
    ];
    let scrape_composite = Arc::new(CompositeAuditSink::new(scrape_sinks));
    let scrape_client = match OpenRouterClient::new(
        &config.openrouter,
        &config.retries,
        scrape_composite,
    ) {
        Ok(c) => c,
        Err(e) => {
            let err = AtsError::Other(format!("openrouter (scrape): {e}"));
            tracing::error!(class = err.class(), exit_code = err.exit_code(), error = %err, "run failed (openrouter init)");
            return err.exit_code();
        }
    };

    let scraper: Box<dyn PageScraper + Send + Sync> = if let Some(p) = std::env::var_os(STUB_HTML_ENV) {
        Box::new(StubScraper { path: p.into() })
    } else {
        Box::new(ChromiumScraper::new(idle))
    };

    let input = RunPipelineInput {
        url: url.to_string(),
        yaml_bytes,
        resume,
    };
    let pdf = Markdown2PdfWriter::new();
    let cfg = config.clone();

    let res = run_pipeline(
        &input,
        config,
        started_at,
        layout as &dyn ats_core::FsLayout,
        scraper.as_ref(),
        &scrape_client,
        scrape_buffer.as_ref(),
        &pdf,
        move |dir, initial_records| {
            let audit_path = dir.join("llm-audit.jsonl");
            let file_sink = match FileAuditSink::create_with_records(&audit_path, initial_records) {
                Ok(s) => s,
                Err(e) => return Err(AtsError::Io(e)),
            };
            let rep = Arc::new(StderrUsageReporter::new(
                "stages",
                Box::new(io::stderr()) as Box<dyn io::Write + Send>,
            ));
            let sinks: Vec<Arc<dyn ats_core::AuditSink>> = vec![
                Arc::new(file_sink),
                rep,
            ];
            let composite: Arc<CompositeAuditSink> = Arc::new(CompositeAuditSink::new(sinks));
            let for_client = composite.clone() as Arc<dyn ats_core::AuditSink>;
            let client = match OpenRouterClient::new(&cfg.openrouter, &cfg.retries, for_client) {
                Ok(c) => c,
                Err(e) => return Err(AtsError::Other(format!("openrouter (stages): {e}"))),
            };
            Ok(StagesBundle {
                client: Box::new(client),
                keyword_audit: composite,
            })
        },
    )
    .await;

    let mut tag_buf = String::new();
    match res {
        Ok((mut run, pipeline)) => {
            run.set_args_summary(args_summary);
            run.set_config_snapshot(config_redacted);
            if let Err(e) = run
                .set_extra("total_tokens", pipeline.total_tokens)
                .and_then(|_| run.set_extra("final_pdf", pipeline.output_pdf.display().to_string()))
            {
                tracing::error!(%e, "set_extra total_tokens/final_pdf");
            }
            if let Err(e) = run.finalize(&clock, RunOutcome::Success, 0) {
                tracing::error!(%e, "run.json finalize");
            } else {
                tracing::info!(
                    stage = "run",
                    total_tokens = pipeline.total_tokens.total,
                    "run.finished"
                );
            }
            0
        }
        Err(RunPipelineError::Early(e)) => {
            let code = e.exit_code();
            tracing::error!(class = e.class(), code, error = %e, "run failed (early)");
            code
        }
        Err(RunPipelineError::Late { mut run, err }) => {
            run.set_args_summary(args_summary);
            run.set_config_snapshot(config_redacted);
            let code = err.exit_code();
            let o = classify_outcome(&err, &mut tag_buf);
            if let Err(fe) = run.finalize(&clock, o, code) {
                tracing::error!(%fe, "run.json finalize (failed run)");
            }
            tracing::error!(class = err.class(), code, error = %err, "run failed (late)");
            code
        }
    }
}

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

fn classify_outcome<'a>(err: &'a AtsError, tag_buf: &'a mut String) -> RunOutcome<'a> {
    match err {
        AtsError::Other(msg) if msg == "not implemented in Effort 1" => RunOutcome::Unimplemented,
        AtsError::Scrape(class) => {
            *tag_buf = format!("scrape/{}", class.class_tag());
            RunOutcome::Failure(tag_buf.as_str())
        }
        AtsError::Llm(class) => {
            *tag_buf = format!("llm/{}", llm_class_tag(class));
            RunOutcome::Failure(tag_buf.as_str())
        }
        _ => RunOutcome::Failure(err.class()),
    }
}

fn llm_class_tag(class: &LlmClass) -> &'static str {
    match class {
        LlmClass::Transient(_) => "transient",
        LlmClass::Auth(_) => "auth",
        LlmClass::ContextExceeded(_) => "context-exceeded",
        LlmClass::Other(_) => "other",
    }
}
