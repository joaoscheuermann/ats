//! US-6 orchestration: scrape, materialise a run directory, then render →
//! keywords → optimize → PDF.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use time::OffsetDateTime;

use crate::audit::{format_run_dir_ts, RunFolder};
use crate::config::Config;
use crate::domain::Resume;
use crate::error::AtsError;
use crate::fs_layout::FsLayout;
use crate::render::cache;
use crate::slug::sanitize_title;
use crate::stage::keywords;
use crate::stage::optimize;
use crate::stage::scrape::{self, ScrapeStageConfig};
use crate::ports::TokenUsage;

/// Failure with optional materialised run folder (for `outcome=failed` in `run.json`).
pub enum RunPipelineError {
    /// Scrape (or any step before a run directory exists).
    Early(AtsError),
    /// After `runs/<ts>_run_<slug>/` was created; [`RunFolder::finalize`]
    /// should record `outcome=failed` with [`AtsError::exit_code`].
    Late { run: RunFolder, err: AtsError },
}

/// One full `ats run` (from scrape through final PDF in `output/`).
pub struct RunPipelineResult {
    pub run_dir: PathBuf,
    pub ts_prefix: String,
    pub slug: String,
    pub final_md: PathBuf,
    pub final_pdf: PathBuf,
    pub output_pdf: PathBuf,
    pub cached_baseline: bool,
    pub total_tokens: TokenUsage,
}

/// LLM + audit bundle for the post-scrape pipeline (keywords share the
/// same audit as the client’s HTTP-level logging).
pub struct StagesBundle {
    pub client: Box<dyn crate::LlmClient + Send + Sync>,
    pub keyword_audit: Arc<dyn crate::ports::AuditSink>,
}

/// Input for [`run`].
pub struct RunPipelineInput {
    /// Job posting URL.
    pub url: String,
    pub yaml_bytes: Vec<u8>,
    pub resume: Resume,
}

/// Run the end-to-end pipeline. `llm_scrape` must be wired to
/// `scrape_buffer` on the client side. `build_stages` materialises
/// `llm-audit.jsonl` in the run directory (including the scrape call records)
/// and returns a client + audit for [`keywords::extract`].
pub async fn run<F>(
    input: &RunPipelineInput,
    config: &Config,
    started_at: OffsetDateTime,
    layout: &dyn FsLayout,
    scraper: &dyn crate::scrape_port::PageScraper,
    llm_scrape: &dyn crate::LlmClient,
    scrape_buffer: &crate::ports::VecAuditSink,
    pdf: &dyn crate::PdfWriter,
    build_stages: F,
) -> Result<(RunFolder, RunPipelineResult), RunPipelineError>
where
    F: FnOnce(
        &Path,
        Vec<crate::LlmCallRecord>,
    ) -> Result<StagesBundle, AtsError>,
{
    let stage_cfg = ScrapeStageConfig {
        idle_timeout: Duration::from_millis(config.scrape.network_idle_timeout_ms),
        model: config.models.scrape_to_markdown.clone(),
    };
    let scrape_out = match scrape::fetch_and_convert(
        scraper,
        llm_scrape,
        &input.url,
        &stage_cfg,
    )
    .await
    {
        Ok(o) => o,
        Err(e) => return Err(RunPipelineError::Early(e)),
    };
    let scrape_usage = scrape_out.usage;
    let initial_audit = match scrape_buffer.take() {
        Ok(r) => r,
        Err(e) => return Err(RunPipelineError::Early(AtsError::Io(e))),
    };

    let slug = sanitize_title(&scrape_out.posting.title);
    let mut run = match RunFolder::new_with_started_at(layout, started_at, "run", Some(&slug)) {
        Ok(r) => r,
        Err(e) => return Err(RunPipelineError::Early(AtsError::Io(e))),
    };
    run.add_token_usage(&scrape_usage);

    let posting_json = serde_json::json!({
        "title": scrape_out.posting.title,
        "markdown": scrape_out.posting.markdown,
    });
    if let Err(e) = (|| {
        let pj = serde_json::to_string_pretty(&posting_json)
            .map_err(|e| AtsError::Other(format!("posting json: {e}")))?;
        fs::write(run.path().join("posting.json"), pj).map_err(AtsError::Io)?;
        fs::write(
            run.path().join("posting.md"),
            scrape_out.posting.markdown.as_bytes(),
        )
        .map_err(AtsError::Io)
    })() {
        return Err(RunPipelineError::Late { run, err: e });
    }

    let stages = match build_stages(run.path(), initial_audit) {
        Ok(s) => s,
        Err(e) => return Err(RunPipelineError::Late { run, err: e }),
    };

    let cache = match cache::load_or_render(layout, &input.yaml_bytes, &input.resume) {
        Ok(c) => c,
        Err(e) => return Err(RunPipelineError::Late { run, err: AtsError::Io(e) }),
    };
    if let Err(e) = run
        .set_extra("cached", cache.cached)
        .and_then(|_| run.set_extra("cached_baseline", cache.cached))
        .and_then(|_| run.set_extra("scrape_slug", &slug))
        .and_then(|_| run.set_extra("cache_path", cache.path.display().to_string()))
    {
        return Err(RunPipelineError::Late {
            run,
            err: AtsError::Other(e.to_string()),
        });
    }

    if let Err(e) = fs::write(run.path().join("baseline.md"), cache.content.as_bytes()) {
        return Err(RunPipelineError::Late {
            run,
            err: AtsError::Io(e),
        });
    }

    let kw = match keywords::extract(
        stages.client.as_ref(),
        stages.keyword_audit.as_ref(),
        &scrape_out.posting.markdown,
        &config.models.keyword_extraction,
        config.retries.schema_validation_max_attempts,
    )
    .await
    {
        Ok(k) => k,
        Err(e) => return Err(RunPipelineError::Late { run, err: e }),
    };
    run.add_token_usage(&kw.usage_total);

    if let Err(e) = (|| {
        let pretty_kw = serde_json::to_string_pretty(&kw.set)
            .map_err(|e| AtsError::Other(format!("keywords json: {e}")))?;
        fs::write(run.path().join("keywords.json"), &pretty_kw).map_err(AtsError::Io)?;
        fs::write(run.path().join("keywords.md"), kw.markdown.as_bytes()).map_err(AtsError::Io)
    })() {
        return Err(RunPipelineError::Late { run, err: e });
    }

    let opt = match optimize::run(
        stages.client.as_ref(),
        stages.keyword_audit.as_ref(),
        &cache.content,
        &kw.markdown,
        &config.models.resume_optimization,
    )
    .await
    {
        Ok(o) => o,
        Err(e) => return Err(RunPipelineError::Late { run, err: e }),
    };
    run.add_token_usage(&opt.usage);

    let report = crate::density::measure(&opt.markdown, &kw.set);
    if let Err(e) = run
        .set_extra(
            "keyword_density",
            serde_json::json!({
                "value": report.density,
                "numerator": report.numerator,
                "denominator": report.denominator,
            }),
        )
        .and_then(|_| run.set_extra("density", report.density))
    {
        return Err(RunPipelineError::Late {
            run,
            err: AtsError::Other(e.to_string()),
        });
    }

    if let Err(e) = fs::write(run.path().join("optimized.md"), opt.markdown.as_bytes()) {
        return Err(RunPipelineError::Late {
            run,
            err: AtsError::Io(e),
        });
    }

    let ts_prefix = match format_run_dir_ts(started_at) {
        Ok(s) => s,
        Err(e) => {
            return Err(RunPipelineError::Late {
                run,
                err: AtsError::Io(e),
            });
        }
    };
    let base = format!("{ts_prefix}_{slug}_resume");
    let final_md = run.path().join(format!("{base}.md"));
    let final_pdf = run.path().join(format!("{base}.pdf"));
    if let Err(e) = fs::write(&final_md, opt.markdown.as_bytes()) {
        return Err(RunPipelineError::Late {
            run,
            err: AtsError::Io(e),
        });
    }
    if let Err(e) = pdf.render(&opt.markdown, &final_pdf) {
        return Err(RunPipelineError::Late { run, err: e.into() });
    }
    if !final_pdf.exists() {
        return Err(RunPipelineError::Late {
            run,
            err: AtsError::Pdf("pdf output missing after render".into()),
        });
    }

    let output_name = match final_pdf.file_name() {
        Some(n) => n.to_owned(),
        None => {
            return Err(RunPipelineError::Late {
                run,
                err: AtsError::Other("pdf path has no file name".into()),
            });
        }
    };
    let out_dir = layout.output_dir();
    if let Err(e) = fs::create_dir_all(&out_dir) {
        return Err(RunPipelineError::Late {
            run,
            err: AtsError::Io(e),
        });
    }
    let output_pdf = out_dir.join(&output_name);
    let tmp = out_dir.join(format!(".tmp_{}", output_name.to_string_lossy()));
    if let Err(e) = fs::copy(&final_pdf, &tmp) {
        tracing::warn!(error = %e, stage = "output", "copy=failed");
        return Err(RunPipelineError::Late {
            run,
            err: AtsError::Io(e),
        });
    }
    if let Err(e) = fs::rename(&tmp, &output_pdf) {
        let _ = fs::remove_file(&tmp);
        tracing::warn!(error = %e, stage = "output", "copy=failed");
        return Err(RunPipelineError::Late {
            run,
            err: AtsError::Io(e),
        });
    }

    let total_tokens = run.aggregated_token_usage();
    let res = RunPipelineResult {
        run_dir: run.path().to_path_buf(),
        ts_prefix,
        slug,
        final_md,
        final_pdf,
        output_pdf,
        cached_baseline: cache.cached,
        total_tokens,
    };
    Ok((run, res))
}
