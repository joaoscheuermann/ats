---
status: done
order: 1
created: 2026-04-22 14:35
title: "Scaffold workspace and CLI skeleton"
---

## Description

First vertical slice. Establishes the five-crate layout (`ats-core`, `ats-scrape`, `ats-llm`, `ats-pdf`, `ats-cli`) under `packages/*`, wires them into the existing Cargo workspace, and stands up the `ats` binary with `clap` parsing every subcommand. No real pipeline logic yet — subcommand handlers return an "unimplemented" diagnostic with the correct exit code. The skeleton delivers the repo-wide seams every later Effort plugs into: config loader, filesystem layout, embedded assets, structured logging, error taxonomy, exit-code mapping, and per-invocation audit-folder creation.

## Objective

Running `ats --help` prints the full subcommand list. Running any subcommand with a valid `config.json` next to the binary returns a clear "not implemented for this Effort" error with the right exit code and creates the correct `runs/<ts>_<command>[_<slug>]/` folder with a populated `run.json`. Missing or malformed `config.json` fails fast with exit code 2 and names the offending key. JSON-lines logs appear on stderr; `--log-format=pretty` switches to human-readable logs.

## Implementation Details

- **Workspace wiring**
  - Add members to the root `Cargo.toml`: `packages/ats-core`, `packages/ats-scrape`, `packages/ats-llm`, `packages/ats-pdf`, `packages/ats-cli`.
  - Use `@monodon/rust` generators (`npx nx g @monodon/rust:library ats-core ...`, `npx nx g @monodon/rust:binary ats-cli ...`, etc.) to produce each `Cargo.toml`, `src/`, and `project.json`. Verify the generated targets match the Verification commands below.
- **`ats-core`**
  - `src/assets.rs` — `pub const` exports wrapping `include_str!` for `resume_template.md`, `yaml_schema.json`, `prompts/scrape_to_markdown.md`, `prompts/keyword_extraction.md`, `prompts/resume_optimization.md`, `schemas/job_posting_extraction.json`, `schemas/ats_keyword_extraction.json`. Drop the verbatim files from the journal `## Research` into `packages/ats-core/assets/`.
  - `src/error.rs` — `AtsError` via `thiserror` with variants `Config`, `Yaml(YamlDiag)`, `Scrape(ScrapeClass)`, `Llm(LlmClass)`, `SchemaInvalid`, `Pdf`, `Io`, `Other`. `fn exit_code(&self) -> i32` implementing the locked scheme `0/1/2/3/4/5/6/7`.
  - `src/config.rs` — `Config` serde struct exactly matching the locked shape (no `pdf.style`), no defaults. On missing/invalid file produce `AtsError::Config` with the field path (use `serde_path_to_error`).
  - `src/fs_layout.rs` — `FsLayout` trait + `BinaryFsLayout` concrete that reads `std::env::current_exe()?.parent()` and exposes `config_path`, `cache_dir`, `runs_dir`, `output_dir`. `ensure_dirs()` creates `cache/`, `runs/`, `output/` on first use.
  - `src/ports.rs` — trait stubs for `PageScraper`, `LlmClient`, `PdfWriter`, `Clock`, `AuditSink` (bodies unimplemented — implementations land in later Efforts).
  - `src/audit.rs` — `RunFolder` helper that, given a command name and optional slug, computes `<ts>_<command>[_<slug>]/`, creates the directory, and writes a `run.json` skeleton on drop or on explicit `finalize(outcome, exit_code)`.
  - `src/logging.rs` — `init(log_format: LogFormat)` configuring `tracing_subscriber::fmt` to stderr; JSON formatter by default, pretty formatter when requested.
- **`ats-cli`**
  - `main.rs` — `#[tokio::main]`, `clap` derive `Cli` with global flags `--log-format <json|pretty>` (default `json`) and `--config <path>`.
  - `Commands` enum: `Render { yaml: PathBuf }`, `Scrape { url: String }`, `Keywords`, `Optimize { resume: InputSource, keywords: InputSource }`, `Pdf { out: PathBuf }`, `Run { yaml: PathBuf, url: String }`. `InputSource` accepts `-` for stdin.
  - Each handler: initialise logging → load config → open a `RunFolder` for the command → return `AtsError::Other("not implemented in Effort 1")`. `RunFolder::finalize` writes `run.json` with the outcome and exit code before process exit.
  - Top-level `main` maps `AtsError::exit_code` to `std::process::exit`.
- **Logging surface** — define a `tracing::info_span!("stage", name=...)` helper so later Efforts only have to call it.
- **Asset placement**
  - `packages/ats-core/assets/resume_template.md` — copy from ticket `## Research`.
  - `packages/ats-core/assets/yaml_schema.json` — author a JSON Schema enforcing AC-1.3 required/optional shape (full schema body, not a stub — later Efforts consume it but Effort 1 verifies it parses).
  - `packages/ats-core/assets/prompts/*.md` — copy the three locked prompts.
  - `packages/ats-core/assets/schemas/job_posting_extraction.json` — copy the locked US-2 `response_format` block.
  - `packages/ats-core/assets/schemas/ats_keyword_extraction.json` — copy the locked US-3 `response_format` block.
- **Config snapshot** — `run.json` includes a redacted config snapshot (`openrouter.api_key` masked) plus `{ started_at, finished_at, command, args_summary, outcome, exit_code }`.
- **Windows builds** — keep dependencies default-feature-safe; no Chromium/PDF code yet, so no per-OS issues expected.

## Verification Criteria

Run and observe:

1. `npx nx run-many -t build` succeeds across all five crates.
2. `npx nx run ats-cli:build --configuration=release` produces the release binary; copy it next to a sample `config.json` for the remaining checks.
3. `./ats --help` prints all six subcommands and the two global flags.
4. `./ats render --yaml missing.yaml` with no `config.json` → exit code `2`, stderr JSON log with a clear "config not found" message.
5. With a valid `config.json`, `./ats render --yaml anything.yaml` → exit code `1`, stderr log `stage=render outcome=unimplemented`, and `runs/<ts>_render/run.json` exists and contains the redacted config snapshot plus `outcome:"unimplemented"`.
6. `./ats --log-format=pretty render --yaml anything.yaml` → same behaviour but human-readable stderr.
7. `cargo test --workspace` green; tests cover `Config` parsing (including error path), `AtsError::exit_code` table, and `RunFolder` directory creation/cleanup.

## Done

- All five crates build via `npx nx run-many -t build`.
- Release binary runs; `./ats --help` shows the full surface.
- Running any subcommand against a valid config creates the correct `runs/<ts>_<command>[_<slug>]/run.json` folder and exits with the "unimplemented" code.
- Missing/malformed `config.json` fails fast with exit code 2 and names the offending field.

## Change Summary

### Files created

**ats-core:**
- `packages/ats-core/Cargo.toml`
- `packages/ats-core/project.json`
- `packages/ats-core/src/lib.rs`
- `packages/ats-core/src/{assets,config,error,fs_layout,ports,audit,logging}.rs`
- `packages/ats-core/assets/resume_template.md`
- `packages/ats-core/assets/yaml_schema.json` (hand-authored JSON Schema enforcing AC-1.3)
- `packages/ats-core/assets/prompts/{scrape_to_markdown,keyword_extraction,resume_optimization}.md`
- `packages/ats-core/assets/schemas/job_posting_extraction.json`
- `packages/ats-core/assets/schemas/ats_keyword_extraction.json`

**Adapter crate stubs:**
- `packages/ats-scrape/{Cargo.toml, project.json, src/lib.rs}`
- `packages/ats-llm/{Cargo.toml, project.json, src/lib.rs}`
- `packages/ats-pdf/{Cargo.toml, project.json, src/lib.rs}`

**CLI binary:**
- `packages/ats-cli/{Cargo.toml, project.json}`
- `packages/ats-cli/src/{main.rs, input_source.rs}`
- `packages/ats-cli/tests/cli_smoke.rs`

### Files modified

- `Cargo.toml` — registered all 5 workspace members; pinned edition 2021 / MSRV 1.75; added `[workspace.dependencies]` for serde, serde_json, serde_path_to_error, thiserror, tracing, tracing-subscriber, tokio, clap, time, async-trait, jsonschema.

### Files deleted
None.

### Key decisions / trade-offs

1. **Hand-rolled scaffolding instead of `@monodon/rust` generators.** The generator duplicates `packages/` under `--directory` and forces snake-case crate names, producing `packages/packages/ats_core/ats_core/` instead of the architecture's `packages/ats-core` with a dash-named crate. Rolled back, authored `Cargo.toml`/`project.json`/`src/lib.rs` by hand using the `coding-conventions/references/monodon-rust.md` template. Nx still lists all 5 projects and the `@monodon/rust` executors (`build`, `test`, `lint`, `run`) work normally.
2. **Shared versions in `[workspace.dependencies]`** so adapters don't drift (DRY).
3. **Port-trait placement deviation (scoped).** Only `Clock`, `AuditSink`, and the `TokenUsage`/`LlmCallRecord` data types are defined in `ats-core::ports` today. `PageScraper`, `LlmClient`, `PdfWriter` will land with their adapter crates in Efforts 03/04/06 where their error types naturally belong. Reduces dead code now; Efforts 03/04/06 can pull them up without touching Effort-01 modules.
4. **`io::Error::other`** preferred over `io::Error::new(ErrorKind::Other, _)` to satisfy clippy's `io_other_error` lint under `-D warnings`.
5. **`ExitCode` from `main` instead of `process::exit`** so `RunFolder::finalize` and tracing drains flush cleanly through normal unwinding. Same observable exit-code behaviour.
6. **YAML schema strictness balanced per AC-1.3.** Strict `additionalProperties: false` at root + `cv` + `personal_information`; relaxed inside array items so optional fields can be omitted without validation failure.
7. **`logging::init` is idempotent** (`Once` + `try_init`) so `assert_cmd`-style integration tests that spawn the binary many times in one process don't panic.
8. **`InputSource` in `ats-cli`** (CLI presentation concern).
9. **`tempfile`, `assert_cmd`, `predicates`** added as test-only dev-deps where needed.

### Verification evidence

- `npx nx show projects` → `ats-scrape, ats-core, ats-cli, ats-llm, ats-pdf`.
- `npx nx run-many -t build` → **OK** for all 5.
- `npx nx run ats-cli:build --configuration=release` → `ats.exe` at `dist/target/ats-cli/release/ats.exe` (~1.39 MB).
- `cargo test --workspace` → **34 tests pass** (25 in ats-core, 9 in ats-cli, 0 in stubs).
- `npx nx run-many -t test` → **OK** for all 5.
- `cargo clippy --workspace --all-targets -- -D warnings` → **clean**.
- Release-binary smoke (copied into `%TEMP%\ats-smoke\ats.exe`):
  - `ats --help` lists all 6 subcommands + both global flags (exit 0).
  - Missing `config.json` → exit **2** with JSON stderr naming the config path.
  - Valid config + any subcommand → exit **1**, stderr `stage=<cmd> class=other error="not implemented in Effort 1"`, `runs/<ts>_<command>/run.json` present with `outcome:"unimplemented"`, `exit_code:1`, redacted `config_snapshot` (`api_key:"***"`).
  - `--log-format=pretty` switches to colorized human formatter.
  - All six subcommands exercised; each writes its own `runs/<ts>_<command>/run.json`.
  - Stdout is empty on every unimplemented path.

### Status

`completed` — all Objective, Done, and Verification Criteria pass.
