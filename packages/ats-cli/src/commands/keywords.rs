//! `ats keywords` handler (US-3).
//!
//! Reads posting Markdown from stdin, calls the keyword-extraction LLM stage
//! against OpenRouter, writes the validated keyword JSON to stdout plus a
//! JSON and Markdown copy into the run folder, and emits one
//! `[keywords] tokens: ...` line per LLM attempt via an audit observer.

use std::io::{self, Read, Write};
use std::sync::Arc;

use ats_core::audit::RunFolder;
use ats_core::config::Config;
use ats_core::stage::keywords;
use ats_core::{AtsError, AuditSink};
use ats_llm::{CompositeAuditSink, FileAuditSink, OpenRouterClient};

use crate::observer::StderrUsageReporter;

/// Entry point dispatched from `main.rs`.
///
/// Stdin and stdout are passed as `&mut dyn` so integration tests can drive
/// the handler hermetically. The stderr reporter writes to `std::io::stderr()`
/// directly — production code paths only.
pub async fn handle(
    cfg: &Config,
    run_folder: &mut RunFolder,
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
) -> Result<(), AtsError> {
    handle_with_reporter(
        cfg,
        run_folder,
        stdin,
        stdout,
        Box::new(io::stderr()) as Box<dyn Write + Send>,
    )
    .await
}

async fn handle_with_reporter(
    cfg: &Config,
    run_folder: &mut RunFolder,
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    reporter_writer: Box<dyn Write + Send>,
) -> Result<(), AtsError> {
    let posting_md = read_stdin(stdin)?;

    let audit_path = run_folder.path().join("llm-audit.jsonl");
    let file_sink = Arc::new(FileAuditSink::create(&audit_path).map_err(AtsError::Io)?);
    let reporter = Arc::new(StderrUsageReporter::new("keywords", reporter_writer));
    let sinks: Vec<Arc<dyn AuditSink>> = vec![file_sink.clone(), reporter.clone()];
    let composite = Arc::new(CompositeAuditSink::new(sinks));

    let client = OpenRouterClient::new(
        &cfg.openrouter,
        &cfg.retries,
        composite.clone() as Arc<dyn AuditSink>,
    )
    .map_err(|err| AtsError::Other(format!("failed to build OpenRouter client: {err}")))?;

    let outcome = keywords::extract(
        &client,
        composite.as_ref(),
        &posting_md,
        &cfg.models.keyword_extraction,
        cfg.retries.schema_validation_max_attempts,
    )
    .await?;

    run_folder.add_token_usage(&outcome.usage_total);
    run_folder
        .set_extra("token_usage_total", outcome.usage_total)
        .map_err(|e| AtsError::Other(format!("failed to record token_usage_total: {e}")))?;

    let pretty = serde_json::to_string_pretty(&outcome.set)
        .map_err(|e| AtsError::Other(format!("failed to serialise KeywordSet: {e}")))?;

    std::fs::write(run_folder.path().join("keywords.json"), &pretty).map_err(AtsError::Io)?;
    std::fs::write(
        run_folder.path().join("keywords.md"),
        outcome.markdown.as_bytes(),
    )
    .map_err(AtsError::Io)?;

    write_output(stdout, pretty.as_bytes())
}

fn read_stdin(stdin: &mut dyn Read) -> Result<String, AtsError> {
    let mut buf = String::new();
    stdin.read_to_string(&mut buf).map_err(AtsError::Io)?;
    Ok(buf)
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

