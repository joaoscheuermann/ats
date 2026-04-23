//! `ats optimize` (US-4) — one LLM call, keyword density, run-folder artifacts.

use std::io::{self, Read, Write};
use std::sync::Arc;

use serde_json::Value;

use ats_core::audit::RunFolder;
use ats_core::config::Config;
use ats_core::density;
use ats_core::stage::keywords::{parse_keywords_from_value, to_markdown};
use ats_core::stage::optimize;
use ats_core::{AtsError, AuditSink};
use ats_llm::{CompositeAuditSink, FileAuditSink, OpenRouterClient};

use crate::input_source::InputSource;
use crate::input_source::read_input;
use crate::observer::StderrUsageReporter;

/// Entry point dispatched from `main.rs`.
pub async fn handle(
    cfg: &Config,
    run_folder: &mut RunFolder,
    resume: &InputSource,
    keywords: &InputSource,
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
) -> Result<(), AtsError> {
    handle_with_reporter(
        cfg,
        run_folder,
        resume,
        keywords,
        stdin,
        stdout,
        Box::new(io::stderr()) as Box<dyn Write + Send>,
    )
    .await
}

async fn handle_with_reporter(
    cfg: &Config,
    run_folder: &mut RunFolder,
    resume: &InputSource,
    keywords: &InputSource,
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    reporter_writer: Box<dyn Write + Send>,
) -> Result<(), AtsError> {
    let baseline_md = read_input(resume, stdin).map_err(AtsError::Io)?;
    let keywords_raw = read_input(keywords, stdin).map_err(AtsError::Io)?;

    let value: Value = serde_json::from_str(&keywords_raw).map_err(|e| {
        AtsError::SchemaInvalid(format!(
            "keywords input is not valid JSON ({e}); expected a document matching the ats_keyword_extraction schema"
        ))
    })?;

    let set = parse_keywords_from_value(&value).map_err(|e| {
        AtsError::SchemaInvalid(format!(
            "keywords JSON did not match ats_keyword_extraction schema: {e}"
        ))
    })?;

    let keywords_md = to_markdown(&set);

    let audit_path = run_folder.path().join("llm-audit.jsonl");
    let file_sink = Arc::new(FileAuditSink::create(&audit_path).map_err(AtsError::Io)?);
    let reporter = Arc::new(StderrUsageReporter::new("optimize", reporter_writer));
    let sinks: Vec<Arc<dyn AuditSink>> = vec![file_sink.clone(), reporter.clone()];
    let composite = Arc::new(CompositeAuditSink::new(sinks));

    let client = OpenRouterClient::new(
        &cfg.openrouter,
        &cfg.retries,
        composite.clone() as Arc<dyn AuditSink>,
    )
    .map_err(|err| AtsError::Other(format!("failed to build OpenRouter client: {err}")))?;

    let outcome = optimize::run(
        &client,
        composite.as_ref(),
        &baseline_md,
        &keywords_md,
        &cfg.models.resume_optimization,
    )
    .await?;

    run_folder.add_token_usage(&outcome.usage);
    run_folder
        .set_extra("token_usage_total", outcome.usage)
        .map_err(|e| AtsError::Other(format!("failed to record token_usage_total: {e}")))?;

    let report = density::measure(&outcome.markdown, &set);
    run_folder
        .set_extra(
            "keyword_density",
            serde_json::json!({
                "value": report.density,
                "numerator": report.numerator,
                "denominator": report.denominator,
            }),
        )
        .map_err(|e| AtsError::Other(format!("failed to record keyword_density: {e}")))?;

    std::fs::write(
        run_folder.path().join("optimized.md"),
        outcome.markdown.as_bytes(),
    )
    .map_err(AtsError::Io)?;

    write_output(stdout, outcome.markdown.as_bytes())
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
