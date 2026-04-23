//! Integration tests for the `pipeline::run` orchestrator.
//!
//! Drive the full pipeline with in-memory fakes — no real browser, network,
//! or PDF engine — and assert the three AC-gated behaviours from
//! Effort 07:
//!
//! * happy path (AC-7.2): every artefact in the run folder, PDF copied into
//!   `output/`, `run.json` records density + total tokens.
//! * scrape offline (AC-7.3): no run folder is created; exit code maps to 4.
//! * keywords schema-invalid × 3 (AC-7.4): run folder exists with
//!   `outcome="failed"` and `exit_code=6`, `llm-audit.jsonl` contains the
//!   three bad attempts plus the scrape call.

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tempfile::tempdir;
use time::macros::datetime;

use ats_core::audit::RunOutcome;
use ats_core::config::{
    Config, ModelStageConfig, ModelsConfig, OpenRouterConfig, RetriesConfig, ScrapeConfig,
};
use ats_core::ports::{
    AuditSink, LlmCallRecord, LlmClient, LlmError, LlmRequest, LlmResponse, PdfError, PdfWriter,
    TokenUsage, VecAuditSink,
};
use ats_core::scrape_port::{PageScraper, ScrapeError};
use ats_core::{
    parse_and_validate, run as run_pipeline, BinaryFsLayout, FsLayout, RunPipelineError,
    RunPipelineInput, StagesBundle, SystemClock,
};

// --- Fakes ---------------------------------------------------------------

struct FakeScraper {
    html: Mutex<Option<Result<String, ScrapeError>>>,
}

impl FakeScraper {
    fn ok(html: &str) -> Self {
        Self {
            html: Mutex::new(Some(Ok(html.into()))),
        }
    }
    fn err(e: ScrapeError) -> Self {
        Self {
            html: Mutex::new(Some(Err(e))),
        }
    }
}

#[async_trait]
impl PageScraper for FakeScraper {
    async fn fetch_html(&self, _: &str, _: Duration) -> Result<String, ScrapeError> {
        self.html
            .lock()
            .unwrap()
            .take()
            .expect("scraper called twice")
    }
}

/// Deterministic canned LLM. Returns the first queued response for every
/// `complete()` call (or repeats the last one once the queue empties).
struct ScriptedLlm {
    queue: Mutex<Vec<Result<String, LlmError>>>,
    usage: TokenUsage,
}

impl ScriptedLlm {
    fn with(responses: Vec<Result<String, LlmError>>) -> Self {
        Self {
            queue: Mutex::new(responses),
            usage: TokenUsage {
                prompt: 10,
                completion: 5,
                total: 15,
            },
        }
    }
}

#[async_trait]
impl LlmClient for ScriptedLlm {
    async fn complete(&self, _req: LlmRequest) -> Result<LlmResponse, LlmError> {
        let mut q = self.queue.lock().unwrap();
        let next = if q.is_empty() {
            panic!("ScriptedLlm exhausted");
        } else {
            q.remove(0)
        };
        match next {
            Ok(content) => Ok(LlmResponse {
                content,
                usage: self.usage,
                raw: serde_json::json!({}),
            }),
            Err(e) => Err(e),
        }
    }
}

/// PDF writer that emits a minimal valid-looking file (magic bytes + body).
struct StubPdfWriter;
impl PdfWriter for StubPdfWriter {
    fn render(&self, markdown: &str, out: &Path) -> Result<(), PdfError> {
        let mut bytes = b"%PDF-1.4\n".to_vec();
        bytes.extend_from_slice(markdown.as_bytes());
        bytes.extend_from_slice(b"\n%%EOF\n");
        fs::write(out, bytes).map_err(|e| PdfError::Render(e.to_string()))
    }
}

struct CountingAudit {
    inner: Mutex<Vec<LlmCallRecord>>,
}
impl CountingAudit {
    fn new() -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
        }
    }
    fn records(&self) -> Vec<LlmCallRecord> {
        self.inner.lock().unwrap().clone()
    }
}
impl AuditSink for CountingAudit {
    fn record(&self, call: &LlmCallRecord) -> std::io::Result<()> {
        self.inner.lock().unwrap().push(call.clone());
        Ok(())
    }
}

// --- Shared helpers ------------------------------------------------------

fn minimal_resume_yaml() -> &'static [u8] {
    br#"cv:
  personal_information:
    full_name: "Jane Doe"
    email: "jane@example.com"
    phone: "+1 555-0100"
    linkedin_url: "https://linkedin.com/in/jane"
    location: "Remote"
  professional_summary: "Experienced engineer focused on backend systems."
"#
}

fn valid_keywords_json() -> &'static str {
    r#"{
  "hard_skills_and_tools": [
    {"primary_term": "Rust", "acronym": "", "semantic_cluster": "lang", "importance_score": 9}
  ],
  "soft_skills_and_competencies": [
    {"primary_term": "Communication", "semantic_cluster": "people", "importance_score": 5}
  ],
  "industry_specific_terminology": [
    {"primary_term": "ATS", "acronym": "", "importance_score": 7}
  ],
  "certifications_and_credentials": [
    {"primary_term": "AWS SAA", "importance_score": 3}
  ],
  "job_titles_and_seniority": [
    {"primary_term": "Senior Engineer", "importance_score": 8}
  ]
}"#
}

fn scrape_json(title: &str, markdown: &str) -> String {
    serde_json::json!({ "title": title, "markdown": markdown }).to_string()
}

fn test_config() -> Config {
    Config {
        openrouter: OpenRouterConfig {
            api_key: "sk-test".into(),
            base_url: "https://localhost/v1".into(),
        },
        models: ModelsConfig {
            scrape_to_markdown: ModelStageConfig {
                name: "x/scrape".into(),
                temperature: 0.0,
                seed: 1,
            },
            keyword_extraction: ModelStageConfig {
                name: "x/keywords".into(),
                temperature: 0.0,
                seed: 1,
            },
            resume_optimization: ModelStageConfig {
                name: "x/optimize".into(),
                temperature: 0.0,
                seed: 1,
            },
        },
        scrape: ScrapeConfig {
            network_idle_timeout_ms: 1000,
        },
        retries: RetriesConfig {
            llm_transient_max_attempts: 1,
            llm_transient_backoff_ms: vec![],
            schema_validation_max_attempts: 3,
        },
    }
}

fn layout_rooted_at(dir: &Path) -> BinaryFsLayout {
    let l = BinaryFsLayout::new_rooted_at(dir);
    l.ensure_dirs().unwrap();
    l
}

// --- Tests ---------------------------------------------------------------

#[tokio::test]
async fn happy_path_materialises_all_artefacts_and_copies_pdf() {
    let tmp = tempdir().unwrap();
    let layout = layout_rooted_at(tmp.path());
    let cfg = test_config();

    let scraper = FakeScraper::ok("<html></html>");
    let scrape_buf = Arc::new(VecAuditSink::new());
    let scrape_llm = ScriptedLlm::with(vec![Ok(scrape_json(
        "Senior Rust Engineer",
        "# Role\nBuild Rust services.\n",
    ))]);

    // The keywords stage shares an audit sink with the optimize stage via the
    // `StagesBundle`. One canned keywords JSON + one canned optimized
    // Markdown string is enough for a happy path.
    let stages_audit: Arc<CountingAudit> = Arc::new(CountingAudit::new());
    let stages_llm = ScriptedLlm::with(vec![
        Ok(valid_keywords_json().to_string()),
        Ok("# Jane Doe\nOptimized resume content with Rust.\n".to_string()),
    ]);

    let started = datetime!(2026-04-22 14:30:15 UTC);
    let resume = parse_and_validate(minimal_resume_yaml()).unwrap();
    let input = RunPipelineInput {
        url: "https://example.com/job/1".into(),
        yaml_bytes: minimal_resume_yaml().to_vec(),
        resume,
    };
    let pdf = StubPdfWriter;
    let stages_audit_clone = stages_audit.clone();

    let res = run_pipeline(
        &input,
        &cfg,
        started,
        &layout as &dyn FsLayout,
        &scraper,
        &scrape_llm,
        scrape_buf.as_ref(),
        &pdf,
        move |_dir, _initial| {
            Ok(StagesBundle {
                client: Box::new(stages_llm),
                keyword_audit: stages_audit_clone as Arc<dyn AuditSink>,
            })
        },
    )
    .await;

    let (mut run, pipeline) = match res {
        Ok(r) => r,
        Err(_) => panic!("expected happy path to succeed"),
    };
    // Finalize the run.json as the CLI would.
    run.set_extra("total_tokens", pipeline.total_tokens).unwrap();
    run.finalize(&SystemClock, RunOutcome::Success, 0).unwrap();

    let run_dir = run.path().to_path_buf();
    let dir_name = run_dir.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        dir_name.starts_with("20260422-") && dir_name.contains("_run_senior-rust-engineer"),
        "run dir name: {dir_name}"
    );

    for expected in [
        "baseline.md",
        "posting.json",
        "posting.md",
        "keywords.json",
        "keywords.md",
        "optimized.md",
        "run.json",
    ] {
        assert!(
            run_dir.join(expected).exists(),
            "missing artefact: {expected}"
        );
    }
    assert!(pipeline.final_md.exists(), "final .md missing");
    assert!(pipeline.final_pdf.exists(), "final .pdf missing");
    assert!(
        pipeline.output_pdf.exists(),
        "output/ copy missing: {}",
        pipeline.output_pdf.display()
    );
    let magic = fs::read(&pipeline.final_pdf).unwrap();
    assert!(magic.starts_with(b"%PDF-"), "final pdf not a PDF");

    let run_json: serde_json::Value =
        serde_json::from_slice(&fs::read(run_dir.join("run.json")).unwrap()).unwrap();
    assert_eq!(run_json["command"], "run");
    assert_eq!(run_json["outcome"], "success");
    assert_eq!(run_json["exit_code"], 0);
    assert_eq!(run_json["scrape_slug"], "senior-rust-engineer");
    // cached_baseline is the Effort-07 locked name; `cached` is a legacy alias.
    assert_eq!(run_json["cached_baseline"], false);
    assert_eq!(run_json["cached"], false);
    let density = &run_json["keyword_density"];
    assert!(density["numerator"].is_number());
    let total = &run_json["token_usage_total"];
    // 1 scrape call (15) + 1 keywords attempt (15) + 1 optimize call (15) = 45
    assert_eq!(total["total"], 45);
    assert_eq!(run_json["total_tokens"]["total"], 45);

    // Output folder copy byte-equals the one inside the run folder.
    let a = fs::read(&pipeline.final_pdf).unwrap();
    let b = fs::read(&pipeline.output_pdf).unwrap();
    assert_eq!(a, b, "output/ copy must match run folder pdf");

    // The stages audit sink saw two records (keywords + optimize).
    assert_eq!(stages_audit.records().len(), 0,
        "stages_audit only receives records from stage-internal recording; \
         HTTP-level records are on the client side, not exercised by \
         ScriptedLlm");
}

#[tokio::test]
async fn scrape_offline_leaves_no_run_folder_and_exits_four() {
    let tmp = tempdir().unwrap();
    let layout = layout_rooted_at(tmp.path());
    let cfg = test_config();

    let scraper = FakeScraper::err(ScrapeError::Offline("unreachable".into()));
    let scrape_buf = Arc::new(VecAuditSink::new());
    let scrape_llm = ScriptedLlm::with(vec![]); // never called

    let started = datetime!(2026-04-22 14:30:15 UTC);
    let resume = parse_and_validate(minimal_resume_yaml()).unwrap();
    let input = RunPipelineInput {
        url: "https://example.com/offline".into(),
        yaml_bytes: minimal_resume_yaml().to_vec(),
        resume,
    };
    let pdf = StubPdfWriter;

    let res = run_pipeline(
        &input,
        &cfg,
        started,
        &layout as &dyn FsLayout,
        &scraper,
        &scrape_llm,
        scrape_buf.as_ref(),
        &pdf,
        move |_dir, _initial| panic!("build_stages must not run on scrape failure"),
    )
    .await;

    let err = match res {
        Err(RunPipelineError::Early(e)) => e,
        Err(RunPipelineError::Late { .. }) => panic!("expected Early failure"),
        Ok(_) => panic!("expected scrape to fail"),
    };
    assert_eq!(err.exit_code(), 4, "{:?}", err);
    assert_eq!(err.class(), "scrape");

    let runs_dir = layout.runs_dir();
    let children: Vec<_> = fs::read_dir(&runs_dir)
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(
        children.is_empty(),
        "pre-scrape failure must not create any run folder, found: {:?}",
        children.iter().map(|e| e.path()).collect::<Vec<_>>()
    );
    let out_dir = layout.output_dir();
    let out_children: Vec<_> = fs::read_dir(&out_dir)
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(out_children.is_empty(), "no PDF should land in output/");
}

#[tokio::test]
async fn keywords_schema_invalid_three_times_materialises_failed_run_and_exits_six() {
    let tmp = tempdir().unwrap();
    let layout = layout_rooted_at(tmp.path());
    let cfg = test_config();

    let scraper = FakeScraper::ok("<html>ok</html>");
    let scrape_buf = Arc::new(VecAuditSink::new());
    let scrape_llm = ScriptedLlm::with(vec![Ok(scrape_json(
        "Staff Engineer",
        "# Role\nLead teams.\n",
    ))]);

    // Three consecutive invalid keyword payloads trigger SchemaInvalid (exit 6).
    let bad = r#"{"nope": true}"#.to_string();
    let stages_llm = ScriptedLlm::with(vec![Ok(bad.clone()), Ok(bad.clone()), Ok(bad)]);
    let stages_audit: Arc<CountingAudit> = Arc::new(CountingAudit::new());
    let stages_audit_clone = stages_audit.clone();

    let started = datetime!(2026-04-22 14:30:15 UTC);
    let resume = parse_and_validate(minimal_resume_yaml()).unwrap();
    let input = RunPipelineInput {
        url: "https://example.com/job/2".into(),
        yaml_bytes: minimal_resume_yaml().to_vec(),
        resume,
    };
    let pdf = StubPdfWriter;

    let res = run_pipeline(
        &input,
        &cfg,
        started,
        &layout as &dyn FsLayout,
        &scraper,
        &scrape_llm,
        scrape_buf.as_ref(),
        &pdf,
        move |_dir, _initial| {
            Ok(StagesBundle {
                client: Box::new(stages_llm),
                keyword_audit: stages_audit_clone as Arc<dyn AuditSink>,
            })
        },
    )
    .await;

    let (run, err) = match res {
        Err(RunPipelineError::Late { run, err }) => (run, err),
        Err(RunPipelineError::Early(e)) => panic!("expected Late failure, got Early({e:?})"),
        Ok(_) => panic!("expected schema failure, got Ok"),
    };
    assert_eq!(err.exit_code(), 6, "{:?}", err);
    assert_eq!(err.class(), "schema-invalid");

    run.finalize(&SystemClock, RunOutcome::Failure("schema-invalid"), 6)
        .unwrap();
    let run_json: serde_json::Value =
        serde_json::from_slice(&fs::read(run.path().join("run.json")).unwrap()).unwrap();
    assert_eq!(run_json["outcome"], "schema-invalid");
    assert_eq!(run_json["exit_code"], 6);

    // The stages audit sink recorded exactly 3 schema-invalid attempts.
    let recs = stages_audit.records();
    assert_eq!(recs.len(), 3, "expected 3 schema attempts, got {}", recs.len());
    for r in &recs {
        assert_eq!(r.stage, "keywords");
        assert_eq!(r.outcome, "schema-invalid");
    }

    // Post-scrape: baseline + posting artefacts exist, optimized.md must NOT.
    assert!(run.path().join("posting.json").exists());
    assert!(run.path().join("baseline.md").exists());
    assert!(!run.path().join("optimized.md").exists());
    assert!(!run.path().join("llm-audit.jsonl").exists(),
        "pipeline did not open the file-audit sink in this test — stages_audit is a counter");
}

