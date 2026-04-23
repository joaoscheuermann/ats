---
status: done
order: 5
created: 2026-04-22 14:35
title: "Implement ats optimize with density metric and warnings"
---

## Description

Implements US-4. Adds the optimizer stage on top of the LLM client from Effort 03, computes the AC-4.3 density metric over the resulting Markdown, and emits the mandated warning/informational log lines. Wires the `ats optimize --resume <path|-> --keywords <path|->` subcommand so the full `render → keywords → optimize` chain can now run end-to-end via Unix pipes.

## Objective

Running `./ats optimize --resume baseline.md --keywords keywords.json` calls the LLM once with the locked optimization prompt, prints the optimized Markdown resume to stdout, measures keyword density per AC-4.3, and emits exactly one warning log per AC-4.3 band (`<2%`, `3%–5%`, `>5%`). The command never fails due to density — it always proceeds and prints the optimized resume. The audit log records the single LLM call with token usage.

## Implementation Details

- **Input plumbing**
  - `InputSource` enum (`File(PathBuf)` | `Stdin`) introduced in Effort 01 is now consumed. Exactly one of the two sources may be `-` (stdin); if both are `-`, exit 2 with a config-style diagnostic.
  - Read both inputs into memory as `String`s up front; the optimizer stage operates on in-memory slices.
- **`ats-core::stage::optimize`**
  - Signature: `async fn optimize(llm, baseline_md, keywords_md, cfg, audit) -> Result<Markdown>`.
  - Build `LlmRequest`:
    - `stage = "optimize"`.
    - `model`, `temperature`, `seed` from `cfg.models.resume_optimization`.
    - `messages = [system: PROMPT_RESUME_OPTIMIZATION, user: "=== RESUME ===\n<baseline_md>\n\n=== KEYWORDS ===\n<keywords_md>"]`.
    - No `response_format` — the prompt asks for plain Markdown.
  - Call `llm.complete(...)` (benefits from the transient retry policy from Effort 03).
  - Return `response.content` as the optimized Markdown. No anti-hallucination check (AC-4.2/NFC-6).
- **`ats-core::density`**
  - `pub fn measure(final_md: &str, keywords: &KeywordSet) -> DensityReport`.
  - `DensityReport { numerator: u32, denominator: u32, density: f32 /* 0.0..=1.0 */ }`.
  - Collect `primary_term` values from **all five** arrays. Deduplicate identical terms (case-insensitive) before counting to avoid double-counting the same term appearing in two categories.
  - For each term, compile `(?i)\b<regex-escaped term>\b` with the `regex` crate (Unicode word boundaries). Sum the number of matches in `final_md`. Numerator is the sum across all terms.
  - Denominator is the total word count of `final_md`: split on Unicode whitespace, keep tokens with at least one letter or digit.
  - Density = numerator / denominator (float; 0.0 if denominator is 0 — log an error in that degenerate case).
  - Emit logs exactly once per call:
    - density < 0.02 → `tracing::warn!("density.low", density=..., count=..., words=...)`.
    - 0.03 ≤ density ≤ 0.05 → `tracing::info!("density.informational_upper", ...)` (covers AC-4.3 "informational warning").
    - density > 0.05 → `tracing::warn!("density.over_ceiling", ...)`.
    - 0.02 ≤ density < 0.03 → no log (in the target band).
  - Also emit one human-readable stderr line summarising the measurement for NFC-17-style visibility.
  - Never returns an error; never triggers retries.
- **CLI wiring**
  - Replace the Effort-01 stub for `Optimize`:
    1. Load config; build `OpenRouterClient`; open `RunFolder::new("optimize", None)`.
    2. Read the resume Markdown and keyword JSON from the chosen sources.
    3. Parse the keyword JSON into `KeywordSet` via serde (validate against the embedded schema as a belt-and-braces sanity check — same validator instance as Effort 03; on invalid input, exit 6 with a stderr hint "keywords JSON did not match ats_keyword_extraction schema"). Also build the Markdown view of the keywords (same helper as Effort 03) to feed the optimizer.
    4. `stage::optimize(...)` → optimized Markdown.
    5. `density::measure(&optimized, &keyword_set)` → emit warnings.
    6. Write optimized Markdown to stdout and to `runs/<ts>_optimize/optimized.md`.
    7. `RunFolder::finalize("ok", 0)` with the density summary recorded in `run.json`.
- **Fixtures**
  - `packages/ats-core/tests/fixtures/`:
    - `optimize/baseline.md`, `optimize/keywords.json`, `optimize/optimized-low.md` (density ~1%), `optimize/optimized-mid.md` (~2.5%), `optimize/optimized-hi.md` (~6%).
  - Density unit tests use these fixtures directly against the real `measure` function.

## Verification Criteria

Run and observe:

1. `npx nx run ats-core:test` green; density cases include `<2%`, `~2.5%`, `~4%`, `>5%`, denominator=0, and deduplication of terms appearing in multiple categories.
2. With a real config:
   - `./ats optimize --resume fixtures/baseline.md --keywords fixtures/keywords.json > optimized.md` — stdout is Markdown; `runs/<ts>_optimize/optimized.md` identical; `run.json` contains `density=<value>`; `llm-audit.jsonl` has one record.
   - `./ats render --yaml fixtures/resume.yaml | ./ats optimize --resume - --keywords fixtures/keywords.json` — piping works.
3. Using `fixtures/optimize/optimized-hi.md` routed as the "LLM" output (via a fake `LlmClient` in tests): verify `density.over_ceiling` warning is emitted; command still exits 0.
4. Using `fixtures/optimize/optimized-low.md` as LLM output in tests: `density.low` warning emitted; exit 0.
5. Invalid keywords JSON (wrong shape) → exit 6 with a diagnostic referencing the `ats_keyword_extraction` schema.
6. Supplying `--resume -` together with `--keywords -` → exit 2 with a diagnostic explaining only one input may use stdin.

## Done

- `ats optimize` produces an optimized Markdown resume and always exits 0 regardless of density, while logging the correct warning/informational line for each AC-4.3 band.
- Density measurement is deterministic and covered by unit tests at each band.
- The full pipeline `ats render → ats optimize` composes cleanly via stdio.

## Change Summary

### Files created

- `packages/ats-core/src/density.rs` — `DensityReport`, `measure` (case-insensitive dedup of primary terms across categories, `(?i)\b…\b` with `find_iter`, word count via whitespace split + alphanumeric check, AC-4.3 band logging once, `eprintln!` human summary).
- `packages/ats-core/src/stage/optimize.rs` — `OptimizeOutcome`, `run` (single `LlmRequest`, `stage: "optimize"`, system `PROMPT_RESUME_OPTIMIZATION`, user `=== RESUME ===` / `=== KEYWORDS ===` blocks, no `response_format`).
- `packages/ats-cli/src/commands/optimize.rs` — handler: `read_input` for both sources, JSON parse + `parse_keywords_from_value` (shared with keyword stage), `to_markdown`, `OpenRouterClient`, `density::measure`, `optimized.md`, stdout, `run.json` extras `keyword_density` + `token_usage_total`.
- `packages/ats-cli/tests/optimize_smoke.rs` — wiremock happy path, invalid keywords → exit 6, both stdin → exit 2.
- `packages/ats-core/tests/fixtures/optimize/*` — fixtures for density and optimize unit tests.

### Files modified

- `Cargo.toml` (workspace) — `regex = "1"` in `[workspace.dependencies]`.
- `packages/ats-core/Cargo.toml` — `regex` dependency.
- `packages/ats-core/src/lib.rs` — `pub mod density;` + stage optimize export.
- `packages/ats-core/src/stage/mod.rs` — `pub mod optimize;`.
- `packages/ats-core/src/stage/keywords.rs` — `pub fn parse_keywords_from_value(&Value) -> Result<KeywordSet, String>`; `validate_and_parse` delegates to it (DRY for CLI optimize path).
- `packages/ats-cli/src/commands/mod.rs` — `pub mod optimize;`.
- `packages/ats-cli/src/input_source.rs` — `read_input` helper.
- `packages/ats-cli/src/main.rs` — dispatch `Optimize` to `handle_optimize`, reject both inputs as stdin with `AtsError::Config` (exit 2).

### Files deleted
None.

### Key decisions

1. **Schema-invalid keywords** at CLI boundary map to `AtsError::SchemaInvalid` (exit 6) with messages referencing `ats_keyword_extraction`.
2. **Both resume and keywords as `-`** → `AtsError::Config` (exit 2) — "only one may use stdin" style message.
3. **`keyword_density` in `run.json`** — object `{ "value", "numerator", "denominator" }` (density is raw ratio 0.0–1.0, not a percentage string).
4. **Shared `parse_keywords_from_value`** in `stage::keywords` — single source of JSON Schema validation for both `keywords` and `optimize` subcommands.

### Verification

- `cargo test --workspace` — **183** tests across crates (e.g. 101 ats-core lib + integration, 3 optimize_smoke, etc.), all pass.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `npx nx run-many -t test build --parallel=1` — 5/5 projects green (per subagent run).

### Status

`completed`.
