# `ats` — ATS Resume Optimizer CLI

An **offline-friendly, reproducible** command-line tool that turns a single source-of-truth resume (YAML) into an **ATS-tuned, keyword-aligned PDF** for a specific job posting.

Given **one YAML** and **one job posting URL**, `ats run` will:

1. Scrape the posting with a headless Chromium and distil it to clean Markdown via an LLM.
2. Extract the posting's ATS keywords (hard skills, soft skills, industry terms, certifications, job titles) against a strict JSON Schema.
3. Render a reusable, content-addressed **baseline** resume Markdown from your YAML (cached by SHA-256).
4. Run one LLM pass that weaves the posting's keywords into the baseline while preserving facts.
5. Measure keyword density, render the optimized resume to a **PDF**, and drop a copy in `output/`.

Every run writes a **dated, per-invocation folder** under `runs/` with every intermediate artefact, an append-only `llm-audit.jsonl` (every LLM attempt, prompt, response, token usage), and a `run.json` summary — so you can reproduce, audit, or debug any pipeline you've ever executed.

> Status: this crate lives in an Nx + Cargo monorepo (template kept upstream). The shipping product is the five-crate workspace under `packages/ats-*`.

---

## Table of contents

- [Features](#features)
- [Architecture](#architecture)
- [Install / Build](#install--build)
- [Configure](#configure-configjson)
- [The resume YAML contract](#the-resume-yaml-contract)
- [Subcommands](#subcommands)
  - [`ats render`](#ats-render-us-1)
  - [`ats scrape`](#ats-scrape-us-2)
  - [`ats keywords`](#ats-keywords-us-3)
  - [`ats optimize`](#ats-optimize-us-4)
  - [`ats pdf`](#ats-pdf-us-5)
  - [`ats run`](#ats-run-us-6-the-happy-path)
- [File system layout](#file-system-layout)
- [Exit codes](#exit-codes)
- [Logging](#logging)
- [Development](#development)
- [Troubleshooting](#troubleshooting)

---

## Features

- **One pipeline, one command** — `ats run --yaml resume.yaml <URL>` produces a tailored PDF in one call.
- **Deterministic baseline** — SHA-256 content hashing means your baseline Markdown only re-renders when the YAML (or the embedded template) actually changes; subsequent runs reuse `cache/baseline-<hex>.md`.
- **Full auditability** — every LLM call is written verbatim to `runs/<ts>_*/llm-audit.jsonl` (prompt, response, token usage, attempt, outcome). `run.json` records the redacted config, args, density, token totals, outcome, and exit code.
- **Composable subcommands** — each stage is a self-contained command that reads from stdin and writes to stdout, so you can pipe `ats render | ats pdf --out …` or inspect any step in isolation.
- **Clean error taxonomy** — distinct exit codes (0/1/2/3/4/5/6/7) per failure class, with structured stderr diagnostics.
- **No external services beyond OpenRouter and Chrome** — OpenRouter for LLM calls, a system-installed Chrome/Chromium for scraping, nothing else.

## Architecture

Five Rust crates under `packages/`:

| Crate | Role |
| --- | --- |
| `ats-core` | Domain model, port traits (`LlmClient`, `PageScraper`, `PdfWriter`, `AuditSink`, `Clock`), embedded assets (resume template, JSON Schemas, prompts), pipeline orchestrator, error taxonomy. Concretion-free — no `reqwest`, no Chromium, no PDF engine here. |
| `ats-llm` | `OpenRouterClient` (OpenAI-compatible), transient-retry loop, `FileAuditSink` (JSONL), `CompositeAuditSink` (fan-out). |
| `ats-scrape` | `ChromiumScraper` via `chromiumoxide`, reachability probe, and main-frame status classification (401/403/404/451/timeouts/offline). |
| `ats-pdf` | `Markdown2PdfWriter` wrapping the `markdown2pdf` crate (atomic `*.tmp` → rename, `%PDF-` magic guard). |
| `ats-cli` | The `ats` binary: clap parsing, config loading, logging init, run-folder lifecycle, and subcommand handlers. |

Stages communicate through `ats-core`'s port traits (Dependency Inversion Principle). Production adapters live in the other crates; tests inject fakes without touching the network, a browser, or the PDF engine.

```
┌─────────┐    ┌──────────┐    ┌──────────┐    ┌──────────┐    ┌──────────┐    ┌─────┐
│ YAML    │──▶ │ render   │───▶│ baseline │    │ keywords │───▶│ optimize │───▶│ PDF │
│ posting │──▶ │ scrape   │───▶│ markdown │───▶│          │    │          │    │     │
└─────────┘    └──────────┘    └──────────┘    └──────────┘    └──────────┘    └─────┘
                                                              density ▲            │
                                                                                   ▼
                                                                      output/<ts>_<slug>_resume.pdf
```

## Install / Build

Requirements:

- **Rust** 1.85+ (stable; `Cargo.toml` pins `rust-version`).
- **Node 18+** + npm (only needed for Nx commands; the raw `cargo` flow works without it).
- A **system-installed Chrome/Chromium** for `ats scrape` / `ats run` (any recent Google Chrome, Edge, Brave, or Chromium works — `chromiumoxide` auto-discovers it).

### Build from source

```sh
# Release build (recommended — binary is ~17 MB)
cargo build --release -p ats-cli --target-dir dist/target/ats-cli

# The binary is at:
#   dist/target/ats-cli/release/ats.exe      (Windows)
#   dist/target/ats-cli/release/ats          (Linux/macOS)
```

Or with Nx:

```sh
npx nx run ats-cli:build --configuration=release
```

### Publish a deploy folder

Every `ats` invocation resolves `config.json`, `cache/`, `runs/`, and `output/` **relative to the directory containing the binary**. The simplest deploy is to put `ats` (or `ats.exe`) next to a `config.json`:

```
release/
├─ ats.exe            # or `ats` on Linux/macOS
├─ config.json
├─ resume.yaml        # your input
├─ cache/             # created on first render
├─ runs/              # created on first invocation
└─ output/            # populated by `ats run`
```

Copy the binary:

```powershell
Copy-Item dist\target\ats-cli\release\ats.exe release\ats.exe -Force
```

## Configure (`config.json`)

A single JSON file next to the binary. **All fields are required**; there are no defaults.

```json
{
  "openrouter": {
    "api_key": "sk-or-v1-…",
    "base_url": "https://openrouter.ai/api/v1"
  },
  "models": {
    "scrape_to_markdown":  { "name": "minimax/minimax-m2.7",    "temperature": 0.0, "seed": 666 },
    "keyword_extraction":  { "name": "minimax/minimax-m2.7",    "temperature": 0.0, "seed": 666 },
    "resume_optimization": { "name": "google/gemini-3.1-pro-preview", "temperature": 0.2, "seed": 666 }
  },
  "scrape":  { "network_idle_timeout_ms": 30000 },
  "retries": {
    "llm_transient_max_attempts": 5,
    "llm_transient_backoff_ms": [1000, 2000, 4000, 8000, 16000],
    "schema_validation_max_attempts": 3
  }
}
```

Key points:

- The **three stages** can use different models. `scrape_to_markdown` and `keyword_extraction` benefit from a structured-output-capable model (response-format JSON Schema is enforced); `resume_optimization` is plain Markdown-out.
- `llm_transient_backoff_ms` drives the exponential retry on 429/5xx/network errors. Length must match `llm_transient_max_attempts - 1` budget.
- `schema_validation_max_attempts` caps retries when the keyword-extraction model returns a payload that fails the embedded JSON Schema.
- The **API key is masked** in `run.json` — only `***` is persisted alongside audit artefacts.
- `--config <path>` on the CLI lets you point at a different file (handy for multi-profile setups).

## The resume YAML contract

Validated by `packages/ats-core/assets/yaml_schema.json`. Only `cv.personal_information` (as a full block) and `cv.professional_summary` are required; everything else is optional and simply omitted from the rendered Markdown when absent.

Minimal example:

```yaml
# yaml-language-server: $schema=../packages/ats-core/assets/yaml_schema.json
cv:
  personal_information:
    full_name: "Jane Doe"
    email: "jane@example.com"
    phone: "+1 555-0100"
    linkedin_url: "https://linkedin.com/in/jane"
    location: "Remote"
  professional_summary: "Senior engineer specialising in Rust, distributed systems, and ATS tooling."
```

Optional sections (see the schema for the exhaustive shape):

- `skills[]` → `{ category, items[] }`
- `work_experience[]` → `{ job_title, company_name, location, start_date, end_date, bullets[] }` — dates as `MM/YYYY` or `"Present"`
- `education[]` → `{ degree_name, institution, location, graduation_date }`
- `certifications[]` → `{ title, issuing_organization, year }`

A real, multi-section example is at [`release/resume.yaml`](release/resume.yaml).

## Subcommands

All subcommands share two global flags:

```
ats [--log-format json|pretty] [--config <path>] <subcommand> [args…]
```

Global flags must appear **before** the subcommand (`ats --log-format pretty run …`, not `ats run --log-format pretty …` — clap treats it as subcommand-scoped if you try).

### `ats render` (US-1)

Render the baseline resume Markdown from a YAML file; streams to stdout.

```sh
ats render --yaml resume.yaml
```

Behaviour:

- Validates the YAML against the embedded JSON Schema (exit 3 with `cv.work_experience[1].start_date: …` style path on failure).
- Hashes `normalize_eol(yaml_bytes) || "\n---\n" || normalize_eol(template_bytes)` with SHA-256; CRLF/LF differences do **not** change the key. Output is written to `cache/baseline-<hex>.md` on miss, reused on hit.
- `runs/<ts>_render/run.json` records `{ cached: true|false, cache_path: … }`.
- Broken pipe (e.g. `ats render | head`) is treated as success.

### `ats scrape` (US-2)

Scrape a job posting URL; emits `{ title, markdown }` JSON to stdout.

```sh
ats scrape "https://example.com/jobs/123"
```

Behaviour:

- Probes reachability before launching Chromium (exit 4 with `scrape/offline` if DNS/TCP fails).
- Launches `chromiumoxide`, navigates, waits up to `scrape.network_idle_timeout_ms`, captures HTML, and hands it to the `scrape_to_markdown` LLM with a locked response-format schema.
- `runs/<ts>_scrape_<slug>/` is renamed to include a slug derived from the scraped title.
- Main-frame 401/403 → exit 4 `scrape/auth-required`; 404 → `not-found`; 451 → `geo-blocked`; other 4xx/5xx → `http-<status>`.
- Test seam: `ATS_SCRAPE_STUB_HTML_FILE=<path>` swaps Chromium for a file-backed stub scraper (used by the integration suite).

### `ats keywords` (US-3)

Read posting Markdown on stdin; emit the validated ATS keyword JSON to stdout.

```sh
cat posting.md | ats keywords > keywords.json
```

Behaviour:

- Calls `keyword_extraction` with the locked prompt and response-format schema.
- Schema validation is retried up to `retries.schema_validation_max_attempts`; three strikes → exit 6 with every bad attempt recorded in `llm-audit.jsonl`.
- Produces both `keywords.json` and a human-readable `keywords.md` inside the run folder; stdout is the JSON form.
- HTTP-level failures (429/5xx) retry with `retries.llm_transient_backoff_ms`; auth/context-exceeded fail fast (exit 5).

### `ats optimize` (US-4)

Rewrite a baseline resume against a keyword set with one LLM call; emit the optimized Markdown to stdout.

```sh
ats optimize --resume baseline.md --keywords keywords.json > optimized.md
# Or pipe either one (but not both) from stdin:
cat baseline.md | ats optimize --resume - --keywords keywords.json > optimized.md
```

Behaviour:

- Validates the keywords JSON against the same schema used by `ats keywords` (bad shape → exit 6).
- Calls `resume_optimization` with both inputs as context.
- Measures keyword density on the optimized Markdown (whole-word, case-insensitive matches / total words) and logs an AC-4.3 band (`silent`, `informational_lower`, `informational_upper`, `warn`).
- `run.json` records `keyword_density: { value, numerator, denominator }` and `token_usage_total`.

### `ats pdf` (US-5)

Render a PDF from Markdown on stdin.

```sh
cat optimized.md | ats pdf --out resume.pdf
```

Behaviour:

- Runs the synchronous `markdown2pdf` renderer inside `tokio::task::spawn_blocking` so the async runtime isn't held.
- Atomic write: produces `<out>.tmp`, then renames to `<out>`. Missing parent directories are surfaced as a real error (exit 7), not silently created.
- Verifies the output starts with `%PDF-` before declaring success.
- `run.json` records `bytes_written` and `output_path`.
- Stdout is never touched.

### `ats run` (US-6, the happy path)

Compose everything in one process:

```sh
ats --log-format pretty run --yaml release/resume.yaml "https://job-boards.greenhouse.io/airtable/jobs/8400388002"
```

Pipeline (`scrape → render(cached) → keywords → optimize → pdf → copy-to-output`):

1. Scrape the posting and distil to Markdown. If this step fails, **no run folder is created** — the tool exits 4 with a clean stderr diagnostic (AC-6.2).
2. Sanitize the scraped title into a URL-safe slug and create `runs/<ts>_run_<slug>/`.
3. Write `posting.json` and `posting.md`.
4. Render or reuse the baseline (`cache/baseline-<hex>.md`) and write `baseline.md`.
5. Call `keyword_extraction`; write `keywords.json` + `keywords.md`.
6. Call `resume_optimization`; write `optimized.md`. Measure density and emit the band log.
7. Copy optimized Markdown to `<ts>_<slug>_resume.md` and render `<ts>_<slug>_resume.pdf`.
8. Atomically copy the final PDF into `<binary_dir>/output/<ts>_<slug>_resume.pdf`.

Any failure after step 2 leaves the run folder on disk with `run.json.outcome = "failed"` and the correct exit code for the failing class, plus every LLM audit line up to the failure.

Sample stderr tail:

```
INFO ats::stage::scrape: scrape.llm.ok  elapsed_ms=36668 total_tokens=54361
INFO ats::llm: llm.attempt.ok stage="keywords" elapsed_ms=29341 total_tokens=3693
[stages] tokens: prompt=1447 completion=2246 total=3693 attempt=1 outcome=ok
INFO ats::llm: llm.attempt.ok stage="optimize" elapsed_ms=38968 total_tokens=8197
[stages] tokens: prompt=2747 completion=5450 total=8197 attempt=1 outcome=ok
INFO ats::optimize: density.informational_upper density=0.0499 numerator=46 denominator=921
Keyword density: 5.0% (46 matches / 921 words)
INFO ats::commands::run: run.finished stage="run" total_tokens=66251
```

## File system layout

Everything is rooted at the **binary's directory**, so a single folder is fully self-contained:

```
<binary_dir>/
├─ ats (or ats.exe)
├─ config.json
├─ cache/
│  └─ baseline-<sha256>.md               # one per unique YAML+template
├─ runs/
│  ├─ 20260422-213822_render/            # one folder per invocation
│  │  └─ run.json
│  ├─ 20260422-213822_scrape_<slug>/
│  │  ├─ posting.json / posting.md
│  │  ├─ llm-audit.jsonl
│  │  └─ run.json
│  └─ 20260422-213822_run_<slug>/
│     ├─ baseline.md
│     ├─ posting.json / posting.md
│     ├─ keywords.json / keywords.md
│     ├─ optimized.md
│     ├─ llm-audit.jsonl
│     ├─ 20260422-213822_<slug>_resume.md
│     ├─ 20260422-213822_<slug>_resume.pdf
│     └─ run.json
└─ output/
   └─ 20260422-213822_<slug>_resume.pdf   # byte-identical copy of the run's PDF
```

### `run.json` fields

| Field | Meaning |
| --- | --- |
| `started_at` / `finished_at` | ISO-8601 local with offset |
| `command` | `render` / `scrape` / `keywords` / `optimize` / `pdf` / `run` |
| `slug` | Scraper-derived slug (scrape / run only) |
| `args_summary` | Parsed CLI args (secrets redacted) |
| `config_snapshot` | Full config JSON with `openrouter.api_key` masked |
| `token_usage_total` | Aggregated `{ prompt, completion, total }` across every LLM call |
| `outcome` | `success` / `schema-invalid` / `scrape/offline` / `llm/auth` / `pdf` / … |
| `exit_code` | Process exit code |
| `cached` / `cached_baseline` | Whether render reused a cache file (same value, two keys for forwards/backwards compat) |
| `keyword_density` | `{ value, numerator, denominator }` (optimize / run) |
| `density` | Scalar density, handy for dashboards |
| `bytes_written` / `output_path` | `pdf` command |
| `final_pdf` | Absolute path of the copy inside `output/` (`run` only) |

### `llm-audit.jsonl`

One JSON object per line, one line per LLM **attempt** (success or failure), including the full prompt, response, `usage`, `attempt`, and `outcome` tag. In `ats run`, scrape-phase records are buffered in memory and flushed at the top of the file as soon as the run folder is materialised.

## Exit codes

Locked taxonomy in `packages/ats-core/src/error.rs`:

| Code | Class | Typical cause |
| --- | --- | --- |
| `0` | `ok` | Success |
| `1` | `io` / `other` | Filesystem error, unexpected internal failure |
| `2` | `config` | Missing or malformed `config.json` |
| `3` | `yaml` | YAML schema / parser failure (path prefix in stderr) |
| `4` | `scrape` | Scraper could not deliver usable HTML (offline / auth / 404 / timeout …) |
| `5` | `llm` | LLM call failed (transient after retries, auth, context-exceeded, other) |
| `6` | `schema-invalid` | Keyword extraction failed JSON Schema validation N times |
| `7` | `pdf` | PDF render / write failure |

## Logging

Structured stderr logs. Two formats:

- `--log-format json` (default) — one JSON object per line, good for pipelines and SIEMs.
- `--log-format pretty` — ANSI-coloured human output, good for live runs.

`RUST_LOG` is honoured (`tracing-subscriber::EnvFilter`). Common tweaks:

```sh
# Quieter default
RUST_LOG=info,chromiumoxide=warn ats run --yaml resume.yaml <URL>
# Trace the LLM retry loop
RUST_LOG=ats_llm=debug,ats_core::stage=debug ats keywords < posting.md
```

## Development

```sh
# Clone + build + test
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# Nx equivalents
npx nx run-many -t build test lint
```

Test structure (189 tests across 14 suites as of this writing):

- **Unit tests** live alongside each module (`ats-core`, `ats-llm`, `ats-scrape`, `ats-pdf`).
- **Integration tests** in `packages/*/tests/*.rs` drive either the real binary (`assert_cmd`) or the pipeline with in-memory fakes:
  - `ats-cli/tests/cli_smoke.rs` — global plumbing.
  - `ats-cli/tests/{render,keywords,optimize,scrape,pdf}_smoke.rs` — each subcommand end-to-end.
  - `ats-core/tests/pipeline_run.rs` — happy / scrape-offline / keywords-×3-invalid for `pipeline::run`.
  - `ats-scrape/tests/chromium_e2e.rs` — gated on `ATS_E2E_CHROME=1`; runs the real browser against a local `wiremock` server.

Test seams:

- `ATS_BINARY_DIR` — override the "binary directory" when running tests (so `runs/`, `cache/`, and `output/` land somewhere hermetic). **Unset it in interactive shells** before running `cargo test`; a leaked value leaks into child processes and breaks the smoke tests.
- `ATS_SCRAPE_STUB_HTML_FILE` — swap Chromium for a file-backed stub scraper.

## Troubleshooting

**Silent exit with code 1.**
Usually a **stale binary**. Rebuild and re-publish:

```powershell
cargo build --release -p ats-cli --target-dir dist/target/ats-cli
Copy-Item dist\target\ats-cli\release\ats.exe release\ats.exe -Force
```

**`config.json not found at …` (exit 2).**
Either publish `config.json` next to the binary or pass `--config <path>`.

**`yaml file not found: …` (exit 3).**
Path is resolved relative to the **current working directory**, not the binary directory. From the repo root you usually want `--yaml release/resume.yaml`.

**`scrape error: offline` (exit 4) on a URL you can clearly reach in a browser.**
The built-in reachability probe does a DNS + TCP check before launching Chrome. Corporate proxies, VPN splits, or IPv6-only hosts can trip it up — run `curl -I <url>` from the same shell to compare.

**`scrape error: browser-missing` (exit 4).**
`chromiumoxide` couldn't find a Chrome/Chromium binary. Install one (any modern Chrome/Edge/Brave/Chromium) or set `CHROME` / `PUPPETEER_EXECUTABLE_PATH` to an absolute path.

**`schema validation failed` (exit 6).**
The keyword-extraction model returned JSON that didn't match the embedded schema three times in a row. Try a model known to honour OpenAI-style `response_format`/`json_schema`, or bump `retries.schema_validation_max_attempts`.

**`llm error: context-exceeded` (exit 5).**
The posting + system prompt is larger than the model's context window. Swap `keyword_extraction` to a larger-context model in `config.json`.

**WebSocket / `untagged enum Message` spam from chromiumoxide.**
Noise from the underlying CDP library; filter it out:

```sh
RUST_LOG=info,chromiumoxide=warn,chromiumoxide::handler=error ats run …
```

---

### License

MIT — see `Cargo.toml`.
