---
status: done
order: 2
created: 2026-04-22 14:35
title: "Implement ats render with YAML validation and cache"
---

## Description

Implements US-1 end-to-end. Builds on the skeleton from Effort 01 by turning `ats render --yaml <file>` into a real command that parses YAML, validates it against the embedded JSON Schema, renders the baseline Markdown per the frozen template with optional sections omitted, and caches the result under `<binary_dir>/cache/`. This is the first slice with a non-trivial correctness surface that does not touch the network.

## Objective

Running `ats render --yaml resume.yaml` prints a baseline Markdown resume to stdout that matches a golden fixture exactly, omits sections/bullets/fields that are absent from the YAML, preserves `"Present"` literally, and writes the cache file on first call. A second invocation with an unchanged YAML hits the cache without re-rendering. An invalid YAML (e.g. missing `cv.personal_information`, or wrong type at `cv.work_experience[1].start_date`) exits with code 3 and a structured stderr diagnostic naming the offending path and, when available, the line/column.

## Implementation Details

- **`ats-core::domain`** — define the `Resume` serde model matching the YAML contract: `cv.personal_information { full_name, email, phone, linkedin_url, location }`, `professional_summary: String`, optional `skills: Vec<SkillCategory>`, `work_experience: Vec<Job>`, `education: Vec<Degree>`, `certifications: Vec<Certification>`. Use `Option<Vec<...>>` or `#[serde(default)]` so missing lists parse as empty.
- **YAML parsing** — `serde_yaml_ng::from_slice::<Resume>(&bytes)`. Wrap errors into `YamlDiag { path, reason, line, column }`. Use the underlying `Location` when available; otherwise emit `line=None, column=None`.
- **JSON Schema** — author `packages/ats-core/assets/yaml_schema.json` enforcing AC-1.3:
  - Top-level `cv` object required.
  - `cv.personal_information` (all five string fields required) and `cv.professional_summary` required.
  - `skills`, `work_experience`, `education`, `certifications` all optional arrays; their items enforce their own required fields.
  - `end_date` accepts either `"[MM/YYYY]"` shape or the literal string `"Present"`.
- **Validation flow** — parse YAML to a serde_json::Value first (via `serde_yaml_ng::from_slice`), run `jsonschema::validator_for(&YAML_SCHEMA)?.iter_errors(...)` for full error enumeration, map JSON-pointer paths (e.g. `/cv/work_experience/1/start_date`) to the dotted form `cv.work_experience[1].start_date` before emitting `AtsError::Yaml`.
- **Renderer (`ats-core::render`)** — pure function `render_baseline(resume: &Resume) -> String`:
  1. Header: `# <full_name>` then `<email> | <phone> | <linkedin_url> | <location>`. All five are required, so no conditionals needed here.
  2. `## Professional Summary` → verbatim paragraph.
  3. `## Skills` — emit the section only if `skills` is non-empty; each category is a bullet `- **<category>:** <items joined by ", ">`. Skip categories with empty `items`.
  4. `## Work Experience` — emit only if `work_experience` non-empty. Per job: `**<job_title>** | **<company_name>**`, then `<location> | <start_date> – <end_date>` (preserve `"Present"` verbatim), then bullets (skip the bullet list entirely when `bullets` is empty/missing).
  5. `## Education` — omit the section if empty; per degree: `**<degree_name>**`, then `<institution>, <location> | <graduation_date>`.
  6. `## Certifications` — omit if empty; per entry `- **<title>**, <issuing_organization>, <year>`.
  7. No empty headings, no placeholders.
- **Cache** — key = `Sha256(yaml_bytes || b"\n---\n" || assets::RESUME_TEMPLATE.as_bytes())` → hex. Path: `<cache_dir>/baseline-<hex>.md`. On hit, read the file and return its contents unchanged. On miss, render, write atomically (write to `<path>.tmp` then rename), then return the rendered string.
- **CLI wiring** — replace the Effort-01 stub with: read YAML bytes → parse → validate → render-or-cache → write to stdout. Log `stage=render cached=true/false` and update `run.json` with the cache outcome.
- **Golden fixtures** — under `packages/ats-core/tests/fixtures/`:
  - `minimal.yaml` + `minimal.md` — only required fields.
  - `full.yaml` + `full.md` — every optional section populated.
  - `with-present.yaml` + `with-present.md` — an `end_date: "Present"` case.
  - `invalid-missing-summary.yaml`, `invalid-bad-date-type.yaml` — error cases with expected `path` strings.

## Verification Criteria

Run and observe:

1. `npx nx run ats-core:test` green; includes unit tests for the path mapping (JSON pointer → dotted form) and the cache hash stability, plus golden-file comparisons for `minimal`, `full`, and `with-present`.
2. Build the release binary, drop `config.json` next to it, then:
   - `./ats render --yaml fixtures/full.yaml > /tmp/baseline.md` — stdout equals `fixtures/full.md` byte-for-byte; `runs/<ts>_render/run.json` records `outcome="ok" cached=false`; `cache/baseline-<hex>.md` is present.
   - Repeat the same command — new `runs/<ts>_render/` folder exists, stdout still matches, and the latest `run.json` records `cached=true`; the `cache/` file is unchanged.
   - `./ats render --yaml fixtures/minimal.yaml` — stdout equals `fixtures/minimal.md`; the Skills, Work Experience, Education, Certifications sections are absent entirely.
   - `./ats render --yaml fixtures/invalid-missing-summary.yaml` — exit code 3, stderr JSON diagnostic naming `cv.professional_summary` as the offending path.
   - `./ats render --yaml fixtures/invalid-bad-date-type.yaml` — exit code 3, diagnostic naming `cv.work_experience[1].start_date` with reason and line/column when the YAML parser supplied them.

## Done

- `ats render --yaml <file>` produces golden-matching baseline Markdown for valid inputs and exit code 3 with a path-qualified diagnostic for invalid inputs.
- Cache hits are observable in `run.json` and avoid re-rendering.
- `npx nx run ats-core:test` and `npx nx run ats-cli:build --configuration=release` are green.

## Change Summary

### Files created

- `packages/ats-core/src/render/mod.rs` — re-export module for the render submodules.
- `packages/ats-core/src/render/markdown.rs` — `render_baseline(&Resume) -> String`; matches `assets/resume_template.md` byte patterns (`*` bullets, U+2013 EN-DASH between dates), preserves `"Present"` verbatim, strips trailing whitespace, ends with a single `\n`.
- `packages/ats-core/src/render/validate.rs` — three-pass pipeline (YAML → `serde_json::Value` with line/column on syntax errors; `jsonschema` validation with kind-aware path enrichment for `Required` / `AdditionalProperties`; `serde_path_to_error` deserialisation into `ResumeYaml`). Schema validator cached in `OnceLock<JSONSchema>`.
- `packages/ats-core/src/render/cache.rs` — SHA-256 keyed content cache (`hash_key`, `load_or_render`, `CacheResult`); atomic writes via `*.tmp` → rename.
- `packages/ats-core/src/domain/mod.rs`, `packages/ats-core/src/domain/resume.rs` — `Resume` / `PersonalInformation` / `SkillCategory` / `Job` / `Degree` / `Certification` serde models.
- `packages/ats-cli/src/commands/mod.rs`
- `packages/ats-cli/src/commands/render.rs` — `handle<W: Write>` with NotFound → `AtsError::Yaml`, cache miss/hit recorded in `run.json` extras, streams Markdown to injected writer, `tracing::info!(target: "ats::render", ...)` on completion, broken-pipe tolerant.
- `packages/ats-core/tests/render_golden.rs` — golden-file comparison tests.
- `packages/ats-core/tests/fixtures/{minimal,full,with-present,invalid-missing-summary,invalid-bad-date-type}.{yaml,md}` — CRLF-tolerant fixtures.
- `packages/ats-cli/tests/render_smoke.rs` — end-to-end CLI test with `assert_cmd`.

### Files modified

- `packages/ats-core/src/lib.rs` — exposed `pub mod domain;` and `pub mod render;`; re-exported the public surface.
- `packages/ats-cli/src/main.rs` — delegated `Commands::Render` to `commands::render::handle(...)`; other subcommands still stubbed; added `ATS_BINARY_DIR` env var test seam for hermetic integration tests (production path unchanged when unset).
- `packages/ats-core/src/audit.rs` — added an `extras` mechanism that merges into `run.json` top-level without breaking existing keys.
- `packages/ats-cli/tests/cli_smoke.rs` — rewrote two tests so they survive the Render handler being real (use `scrape` for the "pretty" and "unimplemented" cases).

### Files deleted

- None.

### Key decisions / trade-offs

1. **`JSONSchema::options().compile(&v)` instead of `validator_for`** — the pinned `jsonschema 0.18.3` predates the helper; identical behaviour; matches the pattern already in use from Effort 01.
2. **Path enrichment for `Required` and `AdditionalProperties` errors.** JSON Schema points `instance_path` at the parent of a missing property, so a missing `professional_summary` would otherwise report `cv`. Matching on `ValidationErrorKind::Required { property }` and appending `.<property>` keeps the diagnostic aligned with AC-1.4 (`cv.professional_summary`). Unexpected keys are enumerated in the reason string.
3. **`ATS_BINARY_DIR` env-var test seam** rather than a `--binary-dir` flag — non-invasive, only touched at `main`, production unchanged, lets integration tests drive the binary against a tempdir without ever writing outside it.
4. **CRLF-tolerant goldens.** This checkout has `core.autocrlf=true`; rather than add a repo-level `.gitattributes` (which would force-normalise `resume_template.md` and invalidate any existing cache hashes), the tests `.replace("\r\n", "\n")` before comparison. The renderer always emits LF. **Caveat noted below.**
5. **Sync handler signature `handle<W: Write>(..., writer)`** — nothing in this Effort is async; injecting `W: Write` gives trivial `&mut Vec<u8>` unit tests and keeps DI explicit. The `tokio::main` dispatcher calls it synchronously.
6. **Broken-pipe tolerance** on stdout: `ats render | head` is treated as success. Audit extras (`cached`, `cache_path`) are populated before the write so they survive.
7. **Per-field `trim_end`** on rendered strings plus dropping empty bullets defends against YAML `|`/`>` folded blocks that leave trailing whitespace.

### Verification evidence

- `cargo test --workspace` → **76 tests pass** (render: 9 markdown + 10 validate + 4 cache; CLI render handler: 4; golden integration: 5; render_smoke: 4; plus Effort-01 suite unchanged).
- `cargo clippy --workspace --all-targets -- -D warnings` → clean.
- `npx nx run-many -t build test lint --parallel=1` → green for all 5 projects. (`--parallel=3` occasionally hits transient Windows cargo fingerprint-file locks; serial is reliable.)
- `npx nx run ats-cli:build --configuration=release` → green; release binary at `dist/target/ats-cli/release/ats.exe`, **~4.84 MB** (up from the Effort-01 skeleton — reflects jsonschema + serde_yaml_ng + sha2 being compiled in).
- **Release-binary smoke** (in `%TEMP%\ats-smoke-e02\`):
  1. Setup — binary + `config.json` + 5 fixtures staged; pre-check: empty `runs/`, empty `cache/`.
  2. `ats render --yaml full.yaml` → exit 0; stdout == fixture after CRLF normalisation; cache file `baseline-a08ef0cf…500b.md` (1373 bytes) written.
  3. Rerun → exit 0; new `runs/<ts>_render/`; `run.json` has `"cached": true` with the same `cache_path`; `cache/` still contains exactly one file.
  4. `ats render --yaml minimal.yaml` → exit 0; 179-char stdout; header + summary only; `has_skills/has_work/has_edu/has_cert` all False.
  5. `ats render --yaml invalid-missing-summary.yaml` → exit **3**; stderr JSON `error:"yaml error: cv.professional_summary: \`professional_summary\` is a required property"`; `run.json` has `outcome:"yaml"`, `exit_code:3`.
  6. `ats render --yaml invalid-bad-date-type.yaml` → exit **3**; stderr JSON `error:"yaml error: cv.work_experience[1].start_date: 12345 is not of type \"string\""`; `run.json` has `outcome:"yaml"`, `exit_code:3`.

### Status

`completed` — Objective, Done, and all six Verification Criteria satisfied.
