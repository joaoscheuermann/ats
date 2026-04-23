---
status: done
order: 7
created: 2026-04-22 14:35
done_at: 2026-04-22 22:30
title: "Implement ats run orchestrator and output folder"
---

## Description

Final vertical slice — ties everything together. Implements US-6 by orchestrating `scrape → keywords → render(cached) → optimize → pdf` in a single process, materialising the run directory with the locked AC-6.4 naming, copying the final PDF into `<binary_dir>/output/`, and aggregating token usage across all LLM calls. Nothing new below the pipeline layer — all stages already exist from prior Efforts and are composed here.

## Objective

Running `./ats run --yaml resume.yaml https://example.com/job` produces, in a fresh `runs/<ts>_run_<slug>/` folder: `baseline.md`, `posting.json`, `posting.md`, `keywords.json`, `keywords.md`, `optimized.md`, `<ts>_<slug>_resume.md`, `<ts>_<slug>_resume.pdf`, `llm-audit.jsonl`, and `run.json`. A copy of `<ts>_<slug>_resume.pdf` is placed in `<binary_dir>/output/`. Stderr streams per-stage structured logs, per-call token usage, and final aggregated totals. Failures anywhere in the pipeline leave the tool with the correct exit code for the failing class and no partial artefacts (AC-6.2).

## Implementation Details

- **`ats-core::slug::sanitize_title`** — delivered in Effort 02/04 per the spec; this Effort consumes it.
- **`ats-core::pipeline::run`**
  - Signature: `async fn run(inputs: RunInputs, ports: RunPorts) -> Result<RunOutputs, AtsError>` where `RunPorts` bundles `&dyn PageScraper`, `&dyn LlmClient`, `&dyn PdfWriter`, `&dyn AuditSink`, `&dyn Clock`, `&dyn FsLayout`.
  - Stage order (AC-6.1):
    1. `scrape::fetch_and_convert(scraper, llm, url, cfg, audit)` → `JobPosting { title, markdown }`.
    2. `slug = sanitize_title(&job.title)` with fallback `"untitled"`.
    3. Create the final run folder name `<ts>_run_<slug>/` **now** that we know the slug. Up to this point the pipeline held intermediates in memory only.
    4. Write `posting.json` (`{ title, description }`, same shape as `ats scrape` stdout) and `posting.md` to the run folder.
    5. `render::baseline(yaml_bytes, cache)` → `baseline_md`. Write to `baseline.md`. Log `cached=true/false`.
    6. `keywords::extract(llm, &job.markdown, cfg, audit)` → `(KeywordSet, keywords_md)`. Write `keywords.json` and `keywords.md`.
    7. `optimize::optimize(llm, &baseline_md, &keywords_md, cfg, audit)` → `optimized_md`. Write `optimized.md`. Run `density::measure` and emit the configured band log.
    8. Name the final artefacts using `<ts>_<slug>_resume`: write `<ts>_<slug>_resume.md` with the optimized Markdown, then `pdf::render` into `<ts>_<slug>_resume.pdf`.
    9. Copy `<ts>_<slug>_resume.pdf` into `<binary_dir>/output/` (overwrite permitted; atomic write via `output.tmp` → rename).
  - `run.json` at finalise time captures: `started_at`, `finished_at`, `scrape_slug`, `cached_baseline` flag, `density`, `token_usage_total = sum over audit entries`, `outcome`, `exit_code`.
- **Holding intermediates in memory until slug is known**
  - The `RunFolder` helper gains a two-phase mode: (a) start as an `EphemeralRun` that records logs, token accounting, and audit entries in memory; (b) once the slug is known, materialise to disk by creating `<ts>_run_<slug>/`, flushing buffered `llm-audit.jsonl` lines, and switching to disk-backed writes for subsequent audits. This keeps AC-6.2 intact: a failure before materialisation leaves zero artefacts; a failure after materialisation leaves the run folder but it is clearly marked `outcome="failed"` in `run.json`.
- **`output/` copy**
  - Implemented in the pipeline's final step. If the copy itself fails (e.g. permission), the PDF still exists inside the run folder — log a warning `stage=output copy=failed` and return the underlying IO error with exit code 1 (the PDF was already generated successfully, so this is correctly categorised as `Io`, not `Pdf`).
- **CLI wiring**
  - Replace the Effort-01 stub for `Run { yaml, url }`:
    1. Load config.
    2. Construct `ChromiumScraper`, `OpenRouterClient`, `Markdown2PdfWriter`.
    3. Open `EphemeralRun` keyed to command `"run"`.
    4. Call `pipeline::run(...)`.
    5. `run.finalize("ok", 0)` on success; on error, if the run was materialised write `run.json` with `outcome="failed" exit_code=<mapped>` before propagating.
  - Unit tests drive `pipeline::run` with fakes for every port — no real network, browser, LLM, or PDF engine required.
- **Token aggregation**
  - `AuditSink` gains an in-memory accumulator (simple `AtomicU64` per field guarded behind the sink) returning `TotalUsage { prompt, completion, total }` at run end. Both `RunFolder` and `EphemeralRun` read this accumulator when writing `run.json`.
- **Failure semantics** (AC-6.2)
  - If scrape fails → no run folder is created; exit 4.
  - If keywords, render, optimize, or pdf fails → the run folder exists (we materialised after scrape) but `run.json` records `outcome="failed"`; no partial state is reused later; the cache for US-1 is the only cross-run persistence.

## Verification Criteria

Run and observe:

1. `npx nx run-many -t test build lint` green across all five crates.
2. Unit test `pipeline::run` end-to-end with fakes:
   - Fake scraper returns a fixed `JobPosting`; fake LLM returns canned responses for the three stages; fake PDF writer writes a stub file.
   - Assert the run folder is named `<ts>_run_<slug>/`, contains every listed artefact, `run.json` has correct `density` and aggregated token usage, and a PDF appears in `output/`.
3. Integration test: fake scraper fails with `ScrapeClass::Offline` → no run folder created, exit 4.
4. Integration test: fake LLM returns schema-invalid content three times at the keywords stage → run folder created (post-scrape), `outcome="failed"`, exit 6, `llm-audit.jsonl` contains all three bad responses.
5. End-to-end smoke against real services:
   - `./ats run --yaml fixtures/resume.yaml 'https://<real-posting>'` — exit 0.
   - `ls runs/<ts>_run_<slug>/` shows all ten expected files.
   - `ls output/` contains the same `<ts>_<slug>_resume.pdf`; `diff` against the one in the run folder confirms byte equality.
   - Opening the PDF in the OS viewer shows a readable, single-column resume with no creative headers and `MM/YYYY` dates.
   - Re-running with the same YAML sees `cached_baseline: true` in `run.json`.
6. Stderr shows per-stage spans, per-LLM-call token usage, the density band log, and a final `run.finished total_tokens=...` line.

## Done

- `ats run` produces the complete run folder and a copy of the final PDF in `output/` for real inputs, end-to-end, on Windows/macOS/Linux.
- The pipeline's failure semantics match AC-6.2: pre-scrape failure leaves nothing on disk; post-scrape failures leave a clearly marked failed run folder.
- Aggregated token usage is reported and persisted; per-stage logs and per-call audit lines are present.
- All prior Efforts' subcommands still work via stdio, and the combined flow via pipes matches what `ats run` produces in-process.

## Change Summary

- **`ats-core::pipeline`** — new module exposing `run(...) -> Result<(RunFolder, RunPipelineResult), RunPipelineError>` with `RunPipelineError::{Early, Late { run, err }}` modelling AC-6.2 directly. Orchestrates scrape → (materialise `runs/<ts>_run_<slug>/`) → posting.{json,md} → baseline (cached) → keywords → optimize → density → `<ts>_<slug>_resume.{md,pdf}` → atomic copy into `<binary_dir>/output/`.
- **`ats-core::audit`** — `RunFolder::new_with_started_at` lets the orchestrator pin the timestamp at pipeline start (so the run dir name matches what stderr logs at scrape time); `aggregated_token_usage()` exposes the in-memory total; `format_run_dir_ts` is shared with the final artefact naming helper.
- **`ats-core::ports`** — `VecAuditSink` buffers the scrape-phase LLM audit record in memory until the run folder exists.
- **`ats-llm::file_audit`** — `FileAuditSink::create_with_records(path, initial)` truncates/creates the JSONL file and writes the buffered scrape audit records before keyword extraction starts.
- **`ats-cli::commands::run`** — new `handle` that wires `ChromiumScraper` (or `STUB_HTML_ENV` for hermetic integration tests) + two `OpenRouterClient`s (one for scrape audited via `VecAuditSink` + stderr reporter, one for stages audited via `FileAuditSink` + stderr reporter). `main.rs` now dispatches `Commands::Run` before the generic `RunFolder::new` so scrape-early failures create nothing under `runs/`.
- **`run.json` extras** — `scrape_slug`, `cached`, `cached_baseline`, `cache_path`, `keyword_density` (`{value, numerator, denominator}`), `density`, `total_tokens`, `final_pdf`; the built-in `token_usage_total` aggregates across every LLM call.
- **Tests** — `packages/ats-core/tests/pipeline_run.rs` drives three end-to-end scenarios with fakes (happy path / scrape offline / keywords × 3 schema-invalid) and asserts artefact layout, run-folder naming, density / token numbers, output-folder copy byte equality, and the `outcome="failed"` / `exit_code=6` contract. `pipeline_run::scrape_offline_leaves_no_run_folder_and_exits_four` also confirms `runs/` is empty after an `Early` failure.
