---
status: done
order: 6
created: 2026-04-22 14:35
done_at: 2026-04-22 22:30
title: "Implement ats pdf via markdown2pdf"
---

## Description

Implements US-5. Adds the `ats-pdf` adapter wrapping the `markdown2pdf` crate with its default styling, wires the `ats pdf --out <path>` subcommand, and adds the per-invocation audit folder. This is the smallest Effort — it exists on its own so PDF rendering is verified in isolation before the `run` orchestrator in Effort 07 glues it to the rest of the pipeline.

## Objective

Running `./ats pdf --out /tmp/out.pdf < optimized.md` reads Markdown from stdin and writes a valid PDF to the given path (magic bytes `%PDF-`, file size > 0, opens in a PDF viewer). The subcommand never writes to stdout. `runs/<ts>_pdf/run.json` records the outcome and byte count of the output file.

## Implementation Details

- **`ats-pdf` crate**
  - `Markdown2PdfWriter` (unit struct or zero-field struct).
  - Implements `PdfWriter::render(&self, markdown: &str, out: &Path) -> Result<(), PdfError>`:
    - Call the `markdown2pdf` crate's public rendering entry-point (exact function name verified at implementation time against the published crate docs) with default styling.
    - The crate is synchronous; invoke from inside `tokio::task::spawn_blocking` at the caller side (CLI layer) so the async runtime is not blocked. The `PdfWriter` trait itself stays synchronous — `spawn_blocking` lives in the CLI adapter call site.
    - Map any error from `markdown2pdf` into `PdfError::Render(String)`.
    - Write atomically: write to `<out>.tmp` then rename to `<out>`; remove the temp file on error.
  - Optional: surface a `Markdown2PdfWriter::new()` constructor for symmetry, even though it holds no state.
- **CLI wiring**
  - Replace the Effort-01 stub for `Pdf { out }`:
    1. Load config; open `RunFolder::new("pdf", None)` (no LLM calls, so no audit JSONL — `run.json` only).
    2. Read all of stdin into a `String`.
    3. `tokio::task::spawn_blocking(move || Markdown2PdfWriter::new().render(&md, &out_path)).await??`.
    4. Stat the output file; record `bytes_written`, `output_path` in `run.json`.
    5. `RunFolder::finalize("ok", 0)`.
  - `--out` is required (clap `required = true`). Path is resolved relative to CWD.
  - Stdout is untouched per AC-6.3.
- **Fonts & Chromium-free** — `markdown2pdf` is self-contained per the user's confirmation; no external renderer dependency. If a future run surfaces a system-font issue on Windows, document it as a follow-up bug, not a blocker here.

## Verification Criteria

Run and observe:

1. `npx nx run ats-pdf:build` green.
2. Unit test in `ats-pdf`: render a tiny Markdown fixture, assert the output file starts with the bytes `%PDF-` and is > 200 bytes.
3. With the release binary and a real config:
   - `echo '# Hello\n\nWorld' | ./ats pdf --out /tmp/hello.pdf` — exit 0; `file /tmp/hello.pdf` (or `Get-Item` on Windows) confirms a PDF; opening in the OS PDF viewer shows the expected content.
   - `./ats render --yaml fixtures/resume.yaml | ./ats pdf --out /tmp/baseline.pdf` — produces a multi-section PDF rendering of the baseline resume.
   - `runs/<ts>_pdf/run.json` contains `outcome="ok"` and `bytes_written` > 0; no `llm-audit.jsonl` is written.
4. Error case: `./ats pdf --out /root/unwritable.pdf < fixtures/optimized.md` (or equivalent unwritable path on Windows) → exit 7, stderr diagnostic `class=pdf reason=...`, run.json reflects the failure.

## Done

- `ats pdf` produces valid PDFs from Markdown on stdin and exits 7 with a clear diagnostic when the renderer or filesystem fails.
- Unit and smoke tests confirm the output is a real PDF file.
- The `runs/<ts>_pdf/run.json` folder is populated with byte count and outcome.

## Change Summary

- **`ats-core::ports`** — added `PdfError::Render(String)` and `PdfWriter` port; `From<PdfError> for AtsError` maps to `AtsError::Pdf` → exit 7.
- **`ats-pdf`** — `Markdown2PdfWriter` unit struct calls `markdown2pdf::parse_into_file(.., ConfigSource::Default, None)` with `*.tmp` → rename atomic write and a `%PDF-` magic check. Deliberately does **not** create missing parent directories so AC-6.4 ("unwritable path → exit 7, class=pdf") is observable.
- **`ats-cli::commands::pdf`** — reads all of stdin, calls the writer inside `tokio::task::spawn_blocking`, then records `bytes_written` and `output_path` in `run.json`. Does not open a `FileAuditSink` (no LLM calls in this command).
- **Tests** — `ats-pdf::tests::tiny_markdown_yields_valid_pdf` (unit, > 200 bytes, `%PDF-`); `ats-cli::tests::pdf_smoke::pdf_happy_path_writes_magic_bytes_and_exits_zero` and `pdf_unwritable_parent_exits_seven_and_class_is_pdf` (release-shape smoke via `assert_cmd`).
