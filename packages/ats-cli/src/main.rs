//! `ats` — CLI entrypoint.
//!
//! Wires the clap surface, run folders, logging, config, and
//! `AtsError` → exit-code mapping. The `run` subcommand is handled before
//! the shared `RunFolder` allocation so a scrape failure does not create a
//! directory on disk.

use std::path::PathBuf;
use std::process::ExitCode;

use ats_core::audit::{RunFolder, RunOutcome};
use ats_core::{
    AtsError, BinaryFsLayout, Config, FsLayout, LogFormat, SystemClock,
};
use clap::{Parser, Subcommand};
use serde_json::{json, Value};

mod commands;
mod input_source;
mod observer;

use commands::keywords::handle as handle_keywords;
use commands::optimize::handle as handle_optimize;
use commands::pdf::handle as handle_pdf;
use commands::render::{handle as handle_render, RenderArgs};
use commands::run::handle as handle_run;
use commands::scrape::handle as handle_scrape;
use input_source::InputSource;

#[derive(Debug, Parser)]
#[command(
    name = "ats",
    version,
    about = "ATS-tuned resume optimizer: render, scrape, extract keywords, optimize, PDF.",
    long_about = None,
)]
struct Cli {
    /// Format for structured logs written to stderr.
    #[arg(long, global = true, value_enum, default_value_t = LogFormatArg::Json)]
    log_format: LogFormatArg,

    /// Override the `config.json` location. Defaults to `<binary_dir>/config.json`.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum LogFormatArg {
    Json,
    Pretty,
}

impl From<LogFormatArg> for LogFormat {
    fn from(value: LogFormatArg) -> Self {
        match value {
            LogFormatArg::Json => LogFormat::Json,
            LogFormatArg::Pretty => LogFormat::Pretty,
        }
    }
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Render the baseline resume Markdown from a YAML file (stdout).
    Render {
        /// Path to the resume YAML.
        #[arg(long)]
        yaml: PathBuf,
    },
    /// Scrape a job posting URL; emits `{title, markdown}` JSON to stdout.
    Scrape {
        /// Job posting URL.
        url: String,
    },
    /// Extract ATS keywords from posting Markdown on stdin; emits keyword JSON to stdout.
    Keywords,
    /// Optimize a baseline resume against keywords; emits optimized Markdown to stdout.
    Optimize {
        /// Path to the baseline resume Markdown, or `-` for stdin.
        #[arg(long)]
        resume: InputSource,
        /// Path to the keyword JSON, or `-` for stdin.
        #[arg(long)]
        keywords: InputSource,
    },
    /// Render a PDF from optimized Markdown on stdin; writes to `--out`.
    Pdf {
        /// Destination path for the PDF.
        #[arg(long)]
        out: PathBuf,
    },
    /// Full pipeline: render → scrape → keywords → optimize → pdf.
    Run {
        /// Path to the resume YAML.
        #[arg(long)]
        yaml: PathBuf,
        /// Job posting URL.
        url: String,
    },
}

impl Commands {
    fn name(&self) -> &'static str {
        match self {
            Commands::Render { .. } => "render",
            Commands::Scrape { .. } => "scrape",
            Commands::Keywords => "keywords",
            Commands::Optimize { .. } => "optimize",
            Commands::Pdf { .. } => "pdf",
            Commands::Run { .. } => "run",
        }
    }

    fn args_summary(&self) -> Value {
        match self {
            Commands::Render { yaml } => json!({ "yaml": yaml.display().to_string() }),
            Commands::Scrape { url } => json!({ "url": url }),
            Commands::Keywords => json!({}),
            Commands::Optimize { resume, keywords } => json!({
                "resume": resume.describe(),
                "keywords": keywords.describe(),
            }),
            Commands::Pdf { out } => json!({ "out": out.display().to_string() }),
            Commands::Run { yaml, url } => json!({
                "yaml": yaml.display().to_string(),
                "url": url,
            }),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    ats_core::init_logging(cli.log_format.into());

    let exit = run(cli).await;
    ExitCode::from(exit as u8)
}

async fn run(cli: Cli) -> i32 {
    // Config must load before we touch the runs/ directory: a missing
    // config.json is an exit-2 failure and it would be confusing to litter
    // `runs/` with a folder that reports "the config I couldn't load".
    //
    // Tests (and `cargo run` via a wrapper) can point the binary at an
    // arbitrary root by setting `ATS_BINARY_DIR`; this keeps integration
    // tests hermetic without sprinkling test-only code into the binary.
    let layout = match resolve_layout(cli.config.clone()) {
        Ok(layout) => layout,
        Err(err) => {
            tracing::error!(%err, "unable to resolve binary directory");
            return AtsError::from(err).exit_code();
        }
    };

    let config = match Config::load_from(&layout.config_path()) {
        Ok(cfg) => cfg,
        Err(err) => {
            let code = err.exit_code();
            tracing::error!(error = %err, exit_code = code, "config load failed");
            return code;
        }
    };

    let clock = SystemClock;
    let command_name = cli.command.name();
    let args_summary = cli.command.args_summary();
    let config_json = config.redacted_snapshot();

    if let Commands::Run { yaml, ref url } = &cli.command {
        return handle_run(
            &config,
            &layout,
            args_summary.clone(),
            config_json.clone(),
            yaml.as_path(),
            url,
        )
        .await;
    }

    let mut run_folder = match RunFolder::new(&layout, &clock, command_name, None) {
        Ok(folder) => folder,
        Err(err) => {
            tracing::error!(%err, "unable to create run folder");
            return AtsError::from(err).exit_code();
        }
    };
    run_folder.set_args_summary(args_summary.clone());
    run_folder.set_config_snapshot(config_json.clone());

    tracing::info!(
        stage = command_name,
        args = %args_summary,
        run_dir = %run_folder.path().display(),
        "run.started"
    );

    let result = dispatch(&cli.command, &config, &layout, &mut run_folder).await;

    let mut tag_buf = String::new();
    let (exit_code, outcome) = match &result {
        Ok(()) => (0, RunOutcome::Success),
        Err(err) => {
            let code = err.exit_code();
            let outcome = classify_outcome(err, &mut tag_buf);
            tracing::error!(
                stage = command_name,
                class = err.class(),
                exit_code = code,
                error = %err,
                "run failed"
            );
            (code, outcome)
        }
    };

    if let Err(finalize_err) = run_folder.finalize(&clock, outcome, exit_code) {
        tracing::error!(%finalize_err, "failed to write run.json");
    } else {
        tracing::info!(
            stage = command_name,
            outcome = outcome_tag(&outcome),
            exit_code,
            run_dir = %run_folder.path().display(),
            "run.finished"
        );
    }

    exit_code
}

async fn dispatch(
    command: &Commands,
    config: &Config,
    layout: &dyn FsLayout,
    run_folder: &mut RunFolder,
) -> Result<(), AtsError> {
    match command {
        Commands::Render { yaml } => {
            let stdout = std::io::stdout().lock();
            handle_render(
                &RenderArgs {
                    yaml_path: yaml.as_path(),
                },
                layout,
                run_folder,
                stdout,
            )
        }
        Commands::Keywords => {
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            let mut stdin_lock = stdin.lock();
            let mut stdout_lock = stdout.lock();
            handle_keywords(config, run_folder, &mut stdin_lock, &mut stdout_lock).await
        }
        Commands::Scrape { url } => {
            let stdout = std::io::stdout();
            let mut stdout_lock = stdout.lock();
            handle_scrape(config, run_folder, url, &mut stdout_lock).await
        }
        Commands::Optimize { resume, keywords } => {
            if resume.is_stdin() && keywords.is_stdin() {
                return Err(AtsError::Config(
                    "only one of --resume and --keywords may be '-' (stdin); both cannot read from standard input"
                        .into(),
                ));
            }
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            let mut stdin_lock = stdin.lock();
            let mut stdout_lock = stdout.lock();
            handle_optimize(
                config,
                run_folder,
                resume,
                keywords,
                &mut stdin_lock,
                &mut stdout_lock,
            )
            .await
        }
        Commands::Pdf { out } => handle_pdf(config, run_folder, out).await,
        Commands::Run { .. } => {
            // Handled before `RunFolder` creation in `run()`.
            Err(AtsError::Other("internal: run is handled in main::run()".into()))
        }
    }
}

/// Resolve the binary-directory rooted [`BinaryFsLayout`]. Honours the
/// `ATS_BINARY_DIR` environment variable as a test seam, then falls back to
/// the directory containing the current executable.
fn resolve_layout(config_override: Option<PathBuf>) -> std::io::Result<BinaryFsLayout> {
    if let Some(dir) = std::env::var_os("ATS_BINARY_DIR") {
        return Ok(BinaryFsLayout::new_rooted_at(PathBuf::from(dir))
            .with_config_override(config_override));
    }
    Ok(BinaryFsLayout::new_from_current_exe()?.with_config_override(config_override))
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

fn llm_class_tag(class: &ats_core::LlmClass) -> &'static str {
    use ats_core::LlmClass;
    match class {
        LlmClass::Transient(_) => "transient",
        LlmClass::Auth(_) => "auth",
        LlmClass::ContextExceeded(_) => "context-exceeded",
        LlmClass::Other(_) => "other",
    }
}

fn outcome_tag<'a>(outcome: &RunOutcome<'a>) -> &'a str {
    match outcome {
        RunOutcome::Success => "success",
        RunOutcome::Unimplemented => "unimplemented",
        RunOutcome::Failure(tag) => tag,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn help_lists_every_subcommand() {
        let mut cmd = Cli::command();
        let help = cmd.render_long_help().to_string();
        for expected in [
            "render", "scrape", "keywords", "optimize", "pdf", "run",
        ] {
            assert!(
                help.contains(expected),
                "--help should mention `{expected}`: {help}"
            );
        }
    }

    #[test]
    fn global_flags_parse() {
        let cli = Cli::try_parse_from([
            "ats",
            "--log-format",
            "pretty",
            "--config",
            "C:/tmp/config.json",
            "render",
            "--yaml",
            "x.yaml",
        ])
        .unwrap();
        assert!(matches!(cli.log_format, LogFormatArg::Pretty));
        assert_eq!(cli.config.as_deref().unwrap().to_string_lossy(), "C:/tmp/config.json");
        assert_eq!(cli.command.name(), "render");
    }

    #[test]
    fn optimize_accepts_stdin_dash() {
        let cli = Cli::try_parse_from([
            "ats",
            "optimize",
            "--resume",
            "-",
            "--keywords",
            "kw.json",
        ])
        .unwrap();
        if let Commands::Optimize { resume, keywords } = &cli.command {
            assert!(resume.is_stdin());
            assert!(!keywords.is_stdin());
        } else {
            panic!("expected optimize command");
        }
    }
}
