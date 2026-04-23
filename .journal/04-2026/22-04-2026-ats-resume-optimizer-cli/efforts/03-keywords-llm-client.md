---
status: done
order: 3
created: 2026-04-22 14:35
title: "Implement ats keywords with OpenRouter client, retry, and audit"
---

## Description

Brings up the entire LLM infrastructure (`ats-llm`) and uses it to ship `ats keywords` end-to-end. This is the first Effort that makes a real network call, so it also lands the transient-retry policy, the audit-log writer, token-usage reporting, and OpenRouter error classification — all reused by later LLM-using Efforts. Keywords is chosen first (before scrape) because it exercises the LLM pipeline without also needing a browser runtime, keeping the failure surface narrow.

## Objective

Running `cat posting.md | ats keywords` performs one OpenAI-compatible chat completion against OpenRouter using the embedded keyword-extraction prompt and `response_format`, validates the returned content against the embedded JSON Schema, retries the call up to 3 times on schema failure and up to 5 times on transient errors, prints the validated keyword JSON to stdout, and writes a fully populated `runs/<ts>_keywords/llm-audit.jsonl` with every attempt. Token usage for each call and the run total are visible on stderr.

## Implementation Details

- **`ats-llm` crate**
  - `OpenRouterClient` struct constructed with `{ api_key, base_url, transient: RetryConfig }`. Holds a `reqwest::Client` with rustls.
  - Implements `LlmClient::complete(req: LlmRequest) -> Result<LlmResponse, LlmError>`.
  - `LlmRequest { stage, model, temperature, seed, messages, response_format: Option<Value> }`. Maps to the OpenAI chat completions body: `{ model, temperature, seed, messages, response_format }`.
  - `LlmResponse { content: String, usage: TokenUsage, raw: Value }`.
  - **Retry policy** — the transient loop wraps the HTTP call: attempts up to `cfg.retries.llm_transient_max_attempts`, sleeping `cfg.retries.llm_transient_backoff_ms[i]` between attempts. Retriable classes: `LlmClass::Transient` (rate-limit 429, 5xx, `reqwest` timeout/connect errors). Non-retriable surfaced immediately: `Auth` (401/403), `ContextExceeded` (detected via OpenRouter error code or status 400 with a body mentioning context length), `Other`.
  - **Audit sink** — `FileAuditSink` implementing `AuditSink`, writes one `serde_json` line per attempt to `<runs_dir>/<current-run>/llm-audit.jsonl`, containing the full `LlmCallRecord { timestamp, stage, model, temperature, seed, prompt, response, usage, attempt, outcome }`. Wraps a `Mutex<File>` so sequential async calls from one run are safely serialised.
  - **Token-usage reporter** — after every successful call, emit `tracing::info!("llm.usage", stage=..., prompt=..., completion=..., total=...)` and a plain `eprintln!`-equivalent human line (e.g. `[keywords] tokens: prompt=834 completion=712 total=1546`). Aggregate per-run totals into `RunFolder`'s `run.json`.
- **`ats-core::stage::keywords`**
  - Input: posting Markdown. Output: `(KeywordSet, String /* markdown view */)`.
  - Word-count the input; if `< 200`, emit a warning `stage=keywords low_signal=true words=<n>` and proceed (AC-3.3).
  - Build `LlmRequest` with the `keyword_extraction` model/temperature/seed from config, `messages = [system: PROMPT_KEYWORD_EXTRACTION, user: <posting_md>]`, and `response_format = KEYWORD_RESPONSE_FORMAT` (the embedded block) — forwarded verbatim.
  - Validation loop (AC-3.2): up to `cfg.retries.schema_validation_max_attempts` total attempts. Parse `response.content` as JSON; validate with `jsonschema` against the inner `schema` of `KEYWORD_RESPONSE_FORMAT`. On failure, record the attempt in audit (`outcome="schema-invalid"`), then retry. On the 3rd invalid → `AtsError::SchemaInvalid` (exit 6); the run's audit log already contains all three bad responses per NFC-19.
  - On success, deserialise `KeywordSet` and render the Markdown view: `## Hard Skills and Tools`, `## Soft Skills and Competencies`, etc., each as bullet lines `- <primary_term> (score <n><, cluster <semantic_cluster> when present><, acronym <acronym> when present>)`.
- **CLI wiring** — replace the Effort-01 stub for `Keywords`:
  1. Read all of stdin into a `String`.
  2. Open `RunFolder::new("keywords", None)`.
  3. Build an `OpenRouterClient` from the loaded config.
  4. `stage::keywords::extract(&client, &posting_md, &cfg, &audit)`.
  5. Serialise the `KeywordSet` with `serde_json::to_string_pretty` and write to stdout.
  6. `RunFolder::finalize("ok", 0)` so `run.json` records token totals and final state.
- **Fixtures + mocks**
  - `packages/ats-llm/tests/fixtures/`:
    - `valid-keyword-response.json` — one well-formed response.
    - `invalid-shape.json` — missing `importance_score` on a hard-skill entry.
    - `rate-limit-then-success.json` — used by the transient-retry test.
  - `packages/ats-llm/tests/` integration tests use `wiremock` to stand up a local OpenAI-compatible server, exercise 429→success, 500→success, 401 (no retry), context-length 400 (no retry, distinct class), and the 3-strikes schema loop.

## Verification Criteria

Run and observe:

1. `npx nx run ats-llm:test` green, covering each retry/error path plus audit-log contents.
2. `npx nx run ats-core:test` green — includes schema-validation loop tests using a fake `LlmClient` that returns scripted bad-then-good responses.
3. With a real OpenRouter API key in `config.json`:
   - `cat fixtures/posting-engineer.md | ./ats keywords > keywords.json` — stdout is valid JSON, `jq 'keys' keywords.json` shows all five top-level categories, exit code 0.
   - `runs/<ts>_keywords/llm-audit.jsonl` has exactly one line with `outcome="ok"` and non-zero token counts.
   - `runs/<ts>_keywords/run.json` includes an aggregated `token_usage_total`.
   - Stderr shows one human-readable `[keywords] tokens:` line plus structured logs.
4. Force schema failure using a fake/mock: `ats keywords` (wired in a test harness to a mock that returns malformed JSON thrice) → exit code 6 and three audit lines with `outcome="schema-invalid"`.
5. Low-signal warning observable when piping a < 200-word posting.

## Done

- `ats keywords` returns a schema-valid JSON object for real postings and exits non-zero with exit code 6 after 3 schema failures.
- The full transient/schema retry behaviour and per-attempt audit logging is verified by integration tests against a mock OpenAI-compatible server.
- Token usage is printed to stderr per call and aggregated in `run.json`.

## Change Summary

### Files created (all genuinely new in Effort 03; `ats-core` additions below were initially misattributed as "pre-existing" in the developer's return — corrected here)

**`ats-core` (ports, domain, stage):**
- `packages/ats-core/src/stage/mod.rs`
- `packages/ats-core/src/stage/keywords.rs` — `extract(...)` with schema-validation loop (3 attempts), low-signal warning, `KeywordExtractionOutcome`, fake-`LlmClient` unit tests.
- `packages/ats-core/src/domain/keywords.rs` — `KeywordSet` + `HardSkill`, `SoftSkill`, `IndustryTerm`, `CertificationKeyword` (renamed to avoid clash with `domain::resume::Certification`), `JobTitle`; `all_primary_terms()` helper for Effort 05's density metric; `to_markdown(...)` view.
- `packages/ats-core/src/ports.rs` — extended with `LlmClient` trait, `LlmRequest`, `LlmResponse`, `LlmError`, `ChatMessage`, `ChatRole`, `LlmClass`. `TokenUsage`, `LlmCallRecord`, `AuditSink` already existed from Effort 01.

**`ats-llm`:**
- `packages/ats-llm/src/openrouter.rs` — `OpenRouterClient` implementing `LlmClient`; attempt loop with configurable backoff; HTTP status → `LlmError` classifier; `is_context_exceeded_error`; `classify_status`; per-attempt audit writes. 19 unit tests.
- `packages/ats-llm/src/file_audit.rs` — `FileAuditSink` (`Mutex<BufWriter<File>>`, one JSON line per record). 2 tests.
- `packages/ats-llm/src/composite_audit.rs` — `CompositeAuditSink` tee for file-sink + stderr-reporter. 1 test.
- `packages/ats-llm/src/lib.rs` — re-exports (replaced Effort-01 placeholder).

**`ats-cli`:**
- `packages/ats-cli/src/commands/keywords.rs` — `handle(...)` reads stdin, builds composite sink (file + `StderrUsageReporter`), calls `stage::keywords::extract`, writes `keywords.json` / `keywords.md`, folds token usage into `run.json`. 1 unit test.
- `packages/ats-cli/tests/keywords_smoke.rs` — 5 `assert_cmd` + `wiremock` integration tests (happy path, 3×schema-invalid, 401 auth, 400 context-exceeded, 429→429→200 transient recovery).

### Files modified

- `packages/ats-llm/Cargo.toml` — added `reqwest` (rustls-tls), `tokio`, `async-trait`, `serde_json`, `thiserror`, `tracing`, `time`; dev-deps `wiremock`, `tempfile`.
- `packages/ats-cli/Cargo.toml` — dev-deps `wiremock`, `tokio` (macros+rt-multi-thread), `serde_json`, `tempfile`; switched `tempfile`/`tokio` to workspace versions.
- `packages/ats-cli/src/commands/mod.rs` — declared `pub mod keywords;`.
- `packages/ats-cli/src/main.rs` — dispatch `Commands::Keywords` to `commands::keywords::handle`; switched tokio flavor from `multi_thread` to `current_thread` to drop the `Send` bound across stdin/stdout `.await`s. Scrape / Optimize / Pdf / Run still on Effort-01 stubs.

### Files deleted
None.

### Key decisions / trade-offs

1. **Reporter-writer injected via `Box<dyn Write + Send>`** — production passes `io::stderr()`, tests pass `Vec<u8>`. Keeps the stderr token reporter trivially unit-testable without capturing `stderr` via FFI.
2. **`#[tokio::main(flavor = "current_thread")]`** — the binary runs exactly one async dispatch task and never fans out work; dropping `Send` bounds simplifies crossing `.await` with stdin/stdout locks. Integration tests spin up their own `#[tokio::test(flavor = "multi_thread")]` runtimes; no user-visible effect.
3. **Two-layer audit outcomes.** HTTP client audits `ok` / `transient` / `auth` / `context-exceeded` / `other`; the stage layer independently appends `schema-invalid` entries when HTTP succeeded but content was malformed. Gives a complete picture in the 3-strikes case: 3×`ok` from HTTP + 3×`schema-invalid` from stage.
4. **Retry backoff semantics.** `llm_transient_backoff_ms[i]` indexed by `attempt-1`; falls back to the last configured value if the Vec is shorter than `max_attempts`; never sleeps after the final attempt. Tests use `[0,0,0,0,0]` for no real sleep.
5. **`is_context_exceeded_error` detects four patterns** (OpenAI `context_length_exceeded`, "context length", "maximum context", "context window"). Exhaustively unit-tested.
6. **`prompt_tokens` in context-exceeded errors** extracted from either `error.prompt_tokens` or `usage.prompt_tokens` — both OpenAI-style formats found in the wild.
7. **Inline stub payloads in `keywords_smoke.rs`** rather than a `fixtures/*.json` directory — keeps each scenario's expected payload adjacent to its assertions. Minor deviation from the Effort's Implementation Details.

### Verification evidence

| Command | Result |
|---|---|
| `cargo test -p ats-llm` | 25/25 pass |
| `cargo test -p ats-core` | 73/73 pass |
| `cargo test -p ats-cli` | 23/23 pass |
| `cargo test --workspace` | **121/121** (up from 79) |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `npx nx run-many -t build test lint --parallel=1` | 5/5 green |
| `npx nx run ats-cli:build --configuration=release` | green; `dist/target/ats-cli/release/ats.exe` = **~7.98 MB** |

**Release-shape smoke scenarios**, verified by `assert_cmd::Command::cargo_bin("ats")` driving a `wiremock::MockServer` pointed at by `config.json.openrouter.base_url` in a tempdir via `ATS_BINARY_DIR`:

| Scenario | Exit | Observable |
|---|---|---|
| Valid completion (full keyword JSON) | 0 | stdout JSON with all 5 categories; `keywords.json`, `keywords.md`, `llm-audit.jsonl`, `run.json` present; `run.json.token_usage_total.total == 1546`; stderr has `[keywords] tokens:` line |
| 3×schema-invalid | 6 | 3 audit records `outcome:"schema-invalid"`; stderr mentions `keyword extraction` |
| 401 auth | 5 | 1 audit record `outcome:"auth"` |
| 400 context-exceeded | 5 | 1 audit record `outcome:"context-exceeded"` |
| 429→429→200 | 0 | audit outcomes `["transient","transient","ok"]` |

### Bookkeeping notes (carried forward)

- `packages/ats-core/assets/schemas/job_posting_extraction.json` sits in the assets tree but is not referenced anywhere. It appears to be speculative scaffolding left from Effort 01. Flag to Effort 04 for cleanup or first use (scrape stage will need a scrape-output schema of its own; this file may or may not be the right shape).

### Status

`completed` — Objective, Done, and all Verification Criteria satisfied.
