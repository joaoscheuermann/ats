---
status: planning
created: 2026-04-22 14:25
slug: ats-resume-optimizer-cli
---

## Prompt

# ATS Resume Optimizer CLI

## 1. Goal

A command-line tool that produces an ATS-tuned, factually faithful resume tailored to a specific job posting, given the user's experience as YAML and a job posting URL.

## 2. Primary persona

Job Seeker — single local CLI operator. No GUI, no form filling, no page interaction beyond reading.

## 3. Non-functional constraints (final)

| #      | Constraint                                                                                                                  |
| ------ | --------------------------------------------------------------------------------------------------------------------------- |
| NFC-1  | Native Rust CLI binary                                                                                                      |
| NFC-2  | No GUI, no form filling, no interactive page actions                                                                        |
| NFC-3  | PDF export is always produced                                                                                               |
| NFC-4  | LLM calls use the supplied prompts verbatim                                                                                 |
| NFC-5  | Keyword extraction output must validate against the supplied JSON schema                                                    |
| NFC-6  | Anti-hallucination enforced by prompt only; no programmatic post-check                                                      |
| NFC-7  | Target keyword density 2–3%, hard ceiling 5%. Violations **warn and proceed**                                               |
| NFC-8  | Standard ATS-safe headers, single-column layout                                                                             |
| NFC-9  | Scraping uses **`chromiumoxide`** (pure-Rust headless Chromium)                                                             |
| NFC-10 | PDF rendered via `markdown2pdf` with its default styling. Style options are surfaced in `config.json` for future adjustment |
| NFC-11 | LLM provider: **OpenRouter**, OpenAI-compatible API                                                                         |
| NFC-12 | No telemetry in the tool or its dependencies (that we control)                                                              |
| NFC-13 | LLM `temperature` and `seed` are configurable per-stage in `config.json`                                                    |
| NFC-14 | Platforms: Windows, macOS, Linux                                                                                            |
| NFC-15 | Observability: structured logs with timestamps + per-stage progress, always on                                              |
| NFC-16 | Language-agnostic: the tool accepts any language for inputs and passes them through to the LLM without filtering            |
| NFC-17 | Token usage is reported to the user after each LLM call and aggregated per run; no cost cap                                 |
| NFC-18 | One invocation = one URL (no concurrency)                                                                                   |
| NFC-19 | Every LLM call's `{prompt, model, temperature, seed, response, token usage}` is recorded alongside run artifacts            |
| NFC-20 | Config, cache, and run artifacts live in the **same folder as the binary** for this version (revisit when distributed)      |
| NFC-21 | Scrape-stage LLM calls use the supplied `job_posting_extraction` block as OpenRouter `response_format` (structured JSON output) |

---

## 4. User Stories & Acceptance Criteria

### US-1 — Render baseline resume from YAML

**AC-1.1 — Happy path.** Given a valid YAML, the tool renders a Markdown document strictly following the built-in template (section order and headings as supplied below), with YAML values inserted verbatim (no paraphrasing).

**AC-1.2 — Multiple items.** Work experiences and skill categories appear in YAML order; `"Present"` is preserved literally.

**AC-1.3 — Missing optional sections.** The only **required** YAML fields are `cv.personal_information` (as a whole block) and `cv.professional_summary`. Any missing optional field/bullet/section is omitted from the Markdown — no empty headings, no placeholders.

**AC-1.4 — Invalid YAML.** The YAML is validated against a published schema. On failure, the tool exits non-zero and reports the offending path (e.g. `cv.work_experience[1].start_date`), the reason, and the input file's line/column when available.

**AC-1.5 — Fixed template & caching.** The template is the one supplied below (frozen for this version). The rendered baseline is cached and reused across runs when a content hash of `(yaml_file_bytes, template_bytes)` is unchanged.

**Built-in template (frozen):**

```md
# [Full Name]

[Email Address] | [Phone Number] | [LinkedIn URL] | [City, State]

## Professional Summary

[Provide a 2-3 sentence overview …]

## Skills

- **[Category 1, e.g., Technical Skills]:** [Skill 1, Skill 2, …]
- **[Category 2, e.g., Frameworks & Tools]:** [Skill 1, Skill 2, …]
- **[Category 3, e.g., Methodologies]:** [Skill 1, Skill 2, …]

## Work Experience

**[Standard Industry Job Title]** | **[Company Name]**
[City, State] | [MM/YYYY] – [MM/YYYY or Present]

- [Action Verb] [Task/Project] using [Relevant Keyword/Tool] to achieve [Quantifiable Result/Metric].
- …

## Education

**[Specific Degree Name, …]**
[Institution Name], [City, State] | [Graduation Month/Year or MM/YYYY]

## Certifications

- **[Exact Certification Title, …]**, [Issuing Organization], [Year]
```

---

### US-2 — Scrape job posting from URL

**AC-2.1 — Happy path.** Given a reachable URL, `chromiumoxide` loads the page, waits for network idle, captures the rendered HTML, and hands it to the LLM. The LLM returns structured JSON `{ title, description }` where `description` is the posting content as Markdown. The OpenRouter request uses `response_format` set verbatim to `assets/schemas/job_posting_extraction.json` (NFC-21). The LLM is solely responsible for deciding what is and is not the job posting. Internally, the pipeline maps `description` into `JobPosting.markdown` for US-3.

**AC-2.1a — Locked system prompt for scrape→Markdown.** Verbatim `assets/prompts/scrape_to_markdown.md` (NFC-4):

```md
Extract the primary job posting from the provided HTML.

Extraction Rules:

1. Identify the primary job posting on the page. If multiple exist, extract only the main one.
2. Extract the job title and the full job description (including company, location, employment type, responsibilities, requirements, and benefits).
3. Exclude all non-posting content: navigation, footers, cookie banners, related jobs, ads, application forms, recruiter marketing, and legal boilerplate.
4. Preserve the exact original wording and language. Do not paraphrase, summarize, translate, or invent content.
5. Format the description as clean Markdown:
   - Use `##` headings for logical sections, preferring the source's exact labels.
   - Use bullet points for lists.
   - Preserve original paragraph breaks.
   - Exclude all HTML tags, tables, images, and links.
6. If the page does not contain a job posting, extract the page's main content on a best-effort basis. Do not refuse the request.
```

**AC-2.2 — JS-rendered page.** The browser waits for **network idle** with a timeout of **30s**, configurable via `config.json` (`scrape.network_idle_timeout_ms`). On timeout, exit non-zero with a distinct `scrape/timeout` error.

**AC-2.3 — Auth / 404 / network errors.** On auth redirects, non-2xx responses, DNS failure, connection refused, or navigation timeout, exit non-zero with a diagnostic class of `auth-required | not-found | geo-blocked | network-timeout | offline`. No partial artifacts written.

**AC-2.4 — Non-job pages.** No validation. The tool does not judge page content; whatever the LLM returns is the output.

**AC-2.5 — Offline detection.** Network connectivity failures are detected and reported as the `offline` class, distinct from timeouts.

**AC-2.6 — Language-agnostic.** The LLM preserves the source language; the tool makes no language assumptions.

---

### US-3 — Extract & rank ATS keywords

**AC-3.1 — Happy path.** Given job-posting Markdown, the tool calls the LLM with the supplied extraction system prompt. The response must validate against the `ats_keyword_extraction` JSON schema. The tool emits the validated JSON and a Markdown-formatted view suitable for US-4.

**AC-3.2 — Schema-invalid response.** Up to **3 attempts**. On the 4th failure, exit non-zero; the final invalid response is preserved in the run's audit log; no partial artifact emitted.

**AC-3.3 — Low-signal posting.** If the posting Markdown is shorter than **200 words**, the tool logs a warning and proceeds.

**AC-3.4 — Language.** No language gating. The tool accepts any language and forwards it to the LLM unchanged.

**AC-3.5 — Context window.** The tool fetches model metadata from OpenRouter's `/models` endpoint (context window per model) at run start and caches it for the run. If `prompt_tokens + expected_completion` exceeds the configured model's window, the tool fails fast with a diagnostic naming the model, its context size, and the measured size of the input.

---

### US-4 — Optimize resume against keywords

**AC-4.1 — Happy path.** Given baseline resume Markdown and keywords Markdown, the tool calls the LLM with the supplied optimization system prompt and returns a single Markdown resume that preserves the template's section order and headers, uses ATS-safe headers only, and formats dates as `MM/YYYY` (or `MM/YYYY – Present`).

**AC-4.2 — Anti-hallucination.** Enforcement is via the supplied prompt only. No programmatic diff-check.

**AC-4.3 — Density measurement (locked).** After optimization, the tool computes keyword density as:

> `density = (sum of whole-word, case-insensitive occurrences of **every** `primary_term` from the keyword JSON across all five categories) ÷ (total word count of the optimized resume)`

If `density < 2%` or `density > 5%`, the tool logs a warning with the measured value and proceeds. Between 3% and 5% also emits an informational warning per your soft-upper directive. No retries, no failure.

**AC-4.4 — Voice preservation.** The optimized Professional Summary retains the candidate's original factual claims (years, domain, role family); only phrasing, keyword choice, and emphasis change.

---

### US-5 — PDF export

**AC-5.1 — Happy path.** The final Markdown is rendered via `markdown2pdf` using its default styling. A `.pdf` file is always written.

**AC-5.2 — Length.** No length constraint; multi-page is permitted.

---

### US-6 — End-to-end pipeline

**AC-6.1 — Pipeline order.** `scrape → keywords → render(cached) → optimize → pdf`. All intermediate artifacts for a run are written to a per-run subdirectory of the tool's data directory (same folder as the binary, per NFC-20).

**AC-6.2 — No resume-on-failure.** Failed runs do not retain partial state for reuse. Only the US-1 baseline cache crosses runs.

**AC-6.3 — Composability & stdio (locked).**
Subcommands: `render`, `scrape`, `keywords`, `optimize`, `pdf`, `run`.
I/O contract:

| Subcommand | Input                                                                  | Output                                                                     |
| ---------- | ---------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| `render`   | `--yaml <path>`                                                        | Baseline resume Markdown → **stdout**                                      |
| `scrape`   | `<URL>` positional                                                     | Scrape JSON (`{title, description}`) → **stdout**                           |
| `keywords` | Posting Markdown on **stdin**                                          | Keyword JSON → **stdout**                                                  |
| `optimize` | `--resume <path or -\>` + `--keywords <path or -\>` (one may be stdin) | Optimized resume Markdown → **stdout**                                     |
| `pdf`      | Markdown on **stdin**                                                  | Writes PDF to `--out <path>` (PDF is binary; never stdout)                 |
| `run`      | `--yaml <path>` `<URL>`                                                | Writes all artifacts as files in the run directory under the tool data dir |

Only `run` uses file outputs; individual subcommands use stdio to be pipeline-friendly.

**AC-6.4 — Run output naming (locked).**

- Markdown: `<timestamp>_<job-title>_resume.md`
- PDF: `<timestamp>_<job-title>_resume.pdf`
- `timestamp` format: `YYYYMMDD-HHMMSS` (local time, sortable).
- `job-title` is taken from the scrape stage's `title` field.
- **Sanitization rules (locked):** lowercase; replace runs of non-alphanumeric characters with `-`; strip leading/trailing `-`; truncate to 60 characters; if the resulting slug is empty, fall back to `untitled`.
- The full run directory holds the Markdown, PDF, all intermediate artifacts, and the LLM audit log.

---

## 5. Cross-cutting — final

| Topic                                            | Decision                                                                                                      |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------- |
| LLM credentials                                  | Read from `config.json` in the binary's folder. Missing/invalid credentials → fail fast with a clear message. |
| LLM transient errors (rate limits, 5xx, network) | **Up to 5 attempts**, delays `1s → 2s → 4s → 8s → 16s` (exponential, capped at 16s).                          |
| LLM schema-invalid responses (US-3 only)         | 3 attempts (per AC-3.2). Independent from transient-error retries.                                            |
| LLM cost ceiling                                 | None. Report token usage per call and per run.                                                                |
| Context window exceeded                          | Fail fast; windows discovered via OpenRouter `/models`.                                                       |
| Output collisions                                | Prevented by timestamped naming.                                                                              |
| Offline                                          | Distinct error class.                                                                                         |
| Concurrency                                      | One invocation, one URL.                                                                                      |
| Privacy / telemetry                              | None.                                                                                                         |
| Reproducibility                                  | Full call record saved per run.                                                                               |

---

## 6. Config file contract (locked)

`config.json`, sitting next to the binary. **No defaults — every field is required.**

```json
{
  "openrouter": {
    "api_key": "...",
    "base_url": "https://openrouter.ai/api/v1"
  },
  "models": {
    "scrape_to_markdown": { "name": "...", "temperature": 0.0, "seed": 42 },
    "keyword_extraction": { "name": "...", "temperature": 0.0, "seed": 42 },
    "resume_optimization": { "name": "...", "temperature": 0.2, "seed": 42 }
  },
  "scrape": {
    "network_idle_timeout_ms": 30000
  },
  "retries": {
    "llm_transient_max_attempts": 5,
    "llm_transient_backoff_ms": [1000, 2000, 4000, 8000, 16000],
    "schema_validation_max_attempts": 3
  },
  "pdf": {
    "style": {}
  }
}
```

Missing or malformed `config.json` → fail fast naming the offending key or parse error. `pdf.style` is an object reserved for future `markdown2pdf` styling overrides; defaults apply when empty.

---

## 7. Out of scope

- GUI / TUI
- Filling out or submitting job applications
- Interactive page actions (clicking, scrolling, auth flows)
- Multi-URL / concurrent runs
- Multiple or user-supplied resume templates
- Cross-run caching of anything other than the US-1 baseline
- Programmatic hallucination verification (prompt-only)
- Spend caps or cost enforcement
- Distribution / packaging concerns (revisit later)

## Research

### Locked inputs (ship inside the binary via `include_str!`)

All items below are **frozen for this version**. They live under `packages/ats-core/assets/` and are exposed as `pub const` strings from `ats-core::assets`. Changing any of them recompiles the binary and, because their bytes feed the baseline cache hash, naturally invalidates the baseline cache.

#### Resume template — `assets/resume_template.md`

```md
# [Full Name]
[Email Address] | [Phone Number] | [LinkedIn URL] | [City, State]


## Professional Summary
[Provide a 2-3 sentence overview highlighting your years of experience, core competencies, and major career achievements using relevant keywords from the job description.]


## Skills
* **[Category 1, e.g., Technical Skills]:** [Skill 1, Skill 2, Skill 3, Skill 4]
* **[Category 2, e.g., Frameworks & Tools]:** [Skill 1, Skill 2, Skill 3, Skill 4]
* **[Category 3, e.g., Methodologies]:** [Skill 1, Skill 2, Skill 3, Skill 4]


## Work Experience


**[Standard Industry Job Title]** | **[Company Name]**
[City, State] | [MM/YYYY] – [MM/YYYY or Present]
* [Action Verb] [Task/Project] using [Relevant Keyword/Tool] to achieve [Quantifiable Result/Metric].
* [Action Verb] [Task/Project] using [Relevant Keyword/Tool] to achieve [Quantifiable Result/Metric].
* [Action Verb] [Task/Project] using [Relevant Keyword/Tool] to achieve [Quantifiable Result/Metric].


**[Standard Industry Job Title]** | **[Company Name]**
[City, State] | [MM/YYYY] – [MM/YYYY]
* [Action Verb] [Task/Project] using [Relevant Keyword/Tool] to achieve [Quantifiable Result/Metric].
* [Action Verb] [Task/Project] using [Relevant Keyword/Tool] to achieve [Quantifiable Result/Metric].
* [Action Verb] [Task/Project] using [Relevant Keyword/Tool] to achieve [Quantifiable Result/Metric].


## Education


**[Specific Degree Name, e.g., Bachelor of Science in Computer Science]**
[Institution Name], [City, State] | [Graduation Month/Year or MM/YYYY]


## Certifications
* **[Exact Certification Title, e.g., Project Management Professional (PMP)]**, [Issuing Organization], [Year]
```

#### YAML contract — shape the embedded JSON Schema must enforce

```yaml
cv:
  personal_information:
    full_name: "[Full Name]"
    email: "[Email Address]"
    phone: "[Phone Number]"
    linkedin_url: "[LinkedIn URL]"
    location: "[City, State]"

  professional_summary: >
    [2-3 sentence overview...]

  skills:
    - category: "[Category 1]"
      items: ["[Skill 1]", "[Skill 2]", "[Skill 3]", "[Skill 4]"]

  work_experience:
    - job_title: "[Standard Industry Job Title]"
      company_name: "[Company Name]"
      location: "[City, State]"
      start_date: "[MM/YYYY]"
      end_date: "[MM/YYYY or Present]"
      bullets:
        - "[Action Verb] [Task/Project] using [Relevant Keyword/Tool] to achieve [Quantifiable Result/Metric]."

  education:
    - degree_name: "[Specific Degree Name]"
      institution: "[Institution Name]"
      location: "[City, State]"
      graduation_date: "[Graduation Month/Year or MM/YYYY]"

  certifications:
    - title: "[Exact Certification Title]"
      issuing_organization: "[Issuing Organization]"
      year: "[Year]"
```

**Required (per AC-1.3):** `cv.personal_information` (full block) and `cv.professional_summary`. Everything else is optional; missing optional fields/bullets/sections are omitted from the rendered Markdown (no empty headings, no placeholders).

#### Scrape→Markdown system prompt — `assets/prompts/scrape_to_markdown.md`

(Verbatim from spec AC-2.1a.)

```md
Extract the primary job posting from the provided HTML.

Extraction Rules:

1. Identify the primary job posting on the page. If multiple exist, extract only the main one.
2. Extract the job title and the full job description (including company, location, employment type, responsibilities, requirements, and benefits).
3. Exclude all non-posting content: navigation, footers, cookie banners, related jobs, ads, application forms, recruiter marketing, and legal boilerplate.
4. Preserve the exact original wording and language. Do not paraphrase, summarize, translate, or invent content.
5. Format the description as clean Markdown:
   - Use `##` headings for logical sections, preferring the source's exact labels.
   - Use bullet points for lists.
   - Preserve original paragraph breaks.
   - Exclude all HTML tags, tables, images, and links.
6. If the page does not contain a job posting, extract the page's main content on a best-effort basis. Do not refuse the request.
```

#### Job posting extraction JSON schema — `assets/schemas/job_posting_extraction.json`

The full OpenAI-compatible `response_format` block, forwarded **verbatim** to OpenRouter on every US-2 scrape LLM call:

```json
{
  "type": "json_schema",
  "json_schema": {
    "name": "job_posting",
    "strict": true,
    "schema": {
      "type": "object",
      "properties": {
        "title": {
          "type": "string",
          "description": "The exact job title as it appears on the page. Use an empty string if absent."
        },
        "description": {
          "type": "string",
          "description": "The extracted job posting content formatted as clean Markdown."
        }
      },
      "required": ["title", "description"],
      "additionalProperties": false
    }
  }
}
```

#### Keyword extraction system prompt — `assets/prompts/keyword_extraction.md`

```md
Extract, categorize, and rank Applicant Tracking System (ATS) keywords from the provided job description based on the following algorithmic rules.


**Task 1: Extraction and Categorization**
Parse the input text and extract all relevant keywords into these specific categories:
*   **Hard Skills and Tools:** Programming languages, software platforms, technical workflows, and methodologies.
*   **Soft Skills and Competencies:** Leadership, cross-functional collaboration, problem-solving, and communication abilities.
*   **Industry-Specific Terminology:** Sector-specific jargon, performance metrics, and regulatory frameworks (e.g., HIPAA, KPIs, return on investment).
*   **Certifications and Credentials:** Required degrees, professional licenses, and formal certifications.
*   **Job Titles and Seniority:** Exact role titles and leadership scope indicators (e.g., strategic planning, change management).
Whenever a term includes an acronym, extract both the fully spelled-out form and its abbreviation (e.g., "Search Engine Optimization (SEO)" or "Customer Relationship Management (CRM)").


**Task 2: Semantic Grouping**
Because modern ATS platforms utilize Natural Language Processing (NLP) to evaluate semantic equivalents, group contextually related terms together within your extracted lists. For example, cluster variations like "project management," "managing projects," and "program coordination" as a single semantic entity.


**Task 3: Algorithmic Ranking**
Rank the extracted keyword clusters in descending order of importance based on standard ATS scoring parameters:
*   **Keyword Frequency:** Assign the highest weight to terms that appear multiple times throughout the job description.
*   **Document Location:** Prioritize terms explicitly located under mandatory headers such as "Requirements," "Qualifications," or "Preferred Experience".
*   **Skill Weighting:** Rank technical requirements, hard skills, and tools significantly higher than soft skills, as technical competencies typically account for 40% to 60% of the total ATS relevance score.


**System Constraints & Output Format:**
*   Only extract terms explicitly stated or directly semantically implied within the provided text. Do not hallucinate or inject external industry buzzwords.
*   Mirror the exact language used by the employer whenever possible, as some systems enforce literal matching.
*   Format the final output strictly as a structured JSON object containing the categorized and ranked keyword clusters.
```

#### Keyword extraction JSON schema — `assets/schemas/ats_keyword_extraction.json`

The full OpenAI-compatible `response_format` block, forwarded **verbatim** to OpenRouter on every US-3 call:

```json
{
  "type": "json_schema",
  "json_schema": {
    "name": "ats_keyword_extraction",
    "description": "Structured extraction, categorization, and ranking of ATS keywords from a job description.",
    "strict": true,
    "schema": {
      "type": "object",
      "properties": {
        "hard_skills_and_tools": {
          "type": "array",
          "description": "Technical skills, programming languages, methodologies, and software platforms.",
          "items": {
            "type": "object",
            "properties": {
              "primary_term": { "type": "string", "description": "The exact wording used in the job description." },
              "acronym": { "type": "string", "description": "The abbreviation or fully spelled out version, if applicable." },
              "semantic_cluster": { "type": "string", "description": "The overarching concept or taxonomy group this term belongs to." },
              "importance_score": { "type": "integer", "description": "A 1-10 weight score based on frequency and placement." }
            },
            "required": ["primary_term", "acronym", "semantic_cluster", "importance_score"],
            "additionalProperties": false
          }
        },
        "soft_skills_and_competencies": {
          "type": "array",
          "description": "Behavioral traits, leadership indicators, and working styles.",
          "items": {
            "type": "object",
            "properties": {
              "primary_term": { "type": "string" },
              "semantic_cluster": { "type": "string" },
              "importance_score": { "type": "integer" }
            },
            "required": ["primary_term", "semantic_cluster", "importance_score"],
            "additionalProperties": false
          }
        },
        "industry_specific_terminology": {
          "type": "array",
          "description": "Specialized jargon, performance metrics, and regulatory frameworks.",
          "items": {
            "type": "object",
            "properties": {
              "primary_term": { "type": "string" },
              "acronym": { "type": "string" },
              "importance_score": { "type": "integer" }
            },
            "required": ["primary_term", "acronym", "importance_score"],
            "additionalProperties": false
          }
        },
        "certifications_and_credentials": {
          "type": "array",
          "description": "Required licenses, degrees, or professional certifications.",
          "items": {
            "type": "object",
            "properties": {
              "primary_term": { "type": "string" },
              "importance_score": { "type": "integer" }
            },
            "required": ["primary_term", "importance_score"],
            "additionalProperties": false
          }
        },
        "job_titles_and_seniority": {
          "type": "array",
          "description": "Exact role titles and leadership scope indicators.",
          "items": {
            "type": "object",
            "properties": {
              "primary_term": { "type": "string" },
              "importance_score": { "type": "integer" }
            },
            "required": ["primary_term", "importance_score"],
            "additionalProperties": false
          }
        }
      },
      "required": [
        "hard_skills_and_tools",
        "soft_skills_and_competencies",
        "industry_specific_terminology",
        "certifications_and_credentials",
        "job_titles_and_seniority"
      ],
      "additionalProperties": false
    }
  }
}
```

**Density denominator (AC-4.3)** walks every `primary_term` across **all five arrays**.

#### Resume optimization system prompt — `assets/prompts/resume_optimization.md`

```md
**System Prompt: ATS & AI Resume Optimization Expert**


**Role Definition:**
You are an elite Applicant Tracking System (ATS) Specialist and Resume Writer. Your objective is to process a user's pre-structured resume and optimize it using a provided ranked list of ATS keywords, hard skills, tools, soft skills, and job titles. Your goal is to maximize the resume's match rate for both traditional keyword-matching parsers and modern AI-driven semantic ranking algorithms without compromising human readability.


**Inputs:**
1. A ranked list of target keywords (categorized into hard skills, soft skills, tools/tech, and job titles).
2. A structured user resume.


**Optimization Directives:**


**1. Keyword Density and Strategic Placement**
*   Maintain a strict keyword density of 2% to 3% relative to the total word count. Never exceed a 5% density, as keyword stuffing triggers ATS spam filters, damages readability, and results in heavy penalization.
*   Distribute the most heavily weighted keywords strategically across multiple sections rather than clumping them into a single list.
*   Front-load 3 to 5 of the most critical keywords into the opening sentences of the Professional Summary.
*   Ensure the "Skills" section is cleanly formatted (using commas or standard bullet points) and limited to a focused list of 8 to 15 highly relevant hard skills and tools.
*   Feature primary keywords 2 to 3 times throughout the document: once in the summary, once in the skills section, and once tied to a specific work achievement.


**2. The STAR-K Formula for Experience Bullets**
*   Rewrite the work experience bullet points using the STAR-K formula: Situation, Task, Action, Result + Keywords.
*   Follow the specific structure of "Action Verb + Task/Project + Keyword + Quantifiable Result".
*   Never list keywords without context in the experience section. Transform generic duties into measurable impacts. For example, change "Managed projects and teams" to "Led cross-functional collaboration across engineering and design teams using Jira and Asana, reducing project delays by 12%".


**3. Terminology, Acronyms, and Semantic Layering**
*   Mirror the exact terminology of the highest-ranked keywords, as many ATS platforms still rely on exact string matching for hard skills and job titles.
*   For all tools, methodologies, and credentials, spell out the full term followed by its abbreviation in parentheses upon the first mention (e.g., "Search Engine Optimization (SEO)", "Customer Relationship Management (CRM)"). This guarantees the ATS parser will trigger a match regardless of whether the recruiter searches the acronym or the full phrase.
*   Apply the R.E.A.L. method (Read, Extract, Apply, Layer) by layering in semantic variations and synonyms (e.g., using "data analytics" alongside "data analysis") to appeal to AI-driven semantic search engines that evaluate conceptual similarity.


**4. Strict Formatting Constraints**
*   Maintain a clean, single-column, reverse-chronological layout, which is the most universally readable format for ATS parsers.
*   Use strictly standard, universally recognized section headers such as "Work Experience", "Education", and "Technical Skills". Do not use creative headers like "My Journey" or "Career Highlights," as these cause parsing failures and prevent the system from categorizing the keywords inside them.
*   Ensure all employment dates follow a consistent, ATS-friendly format (MM/YYYY to MM/YYYY, e.g., 03/2023 – Present).
*   Remove any tables, text boxes, multi-column structures, icons, or graphics. These elements scramble the reading order of the ATS and cause critical data to be dropped.
*   Use simple standard bullet points (• or -) to avoid character encoding errors.


**5. Ethical Enhancement and Authenticity**
*   Do not hallucinate, invent, or inject unverified skills, metrics, or experiences that are not present or strongly implied in the user's original resume.
*   Enhance the existing expressions both semantically and lexically to improve alignment, but ensure all newly introduced terms are contextually factual to the candidate's original input.
*   Retain the candidate's genuine professional voice and structural integrity while sharpening clarity and emphasizing business impact.


**Output Generation:**
Process the inputs using these directives and generate the fully optimized resume in plain markdown text. Ensure the final output is perfectly structured for machine parsing while remaining highly persuasive for the human recruiter who will read it next.
```

### Approved spec amendments

- **AC-3.5″ (replaces AC-3.5):** No local tokenization, no `/models` call. If OpenRouter returns a context-length-exceeded error for any LLM call, the tool classifies it as `llm/context-exceeded`, logs and records the provider's message, and exits non-zero (exit code `5`). Token usage is read from each response's `usage` field and reported.
- **§6 Config amendment:** `pdf.style` removed for this version (revisit when styling overrides land).

## Architecture

### 1. Category & workspace fit

**Pure Rust CLI app.** One process, one URL per invocation (NFC-18), no FFI, no ML, no cross-stack runtime. Ships as a single binary `ats` on Windows, macOS, and Linux (NFC-14). Lives under `packages/*` via `@monodon/rust`, registered as members in the existing (currently empty) Cargo workspace. No new top-level area required.

Excluded categories: bindings/FFI, offline data builder, ML/training, cross-stack. No domain-specific architecture skills loaded from this repo (none match).

### 2. Crate layout

```
packages/
  ats-core/    # domain types, port traits, pipeline + stages, config, embedded assets
  ats-scrape/  # chromiumoxide PageScraper adapter
  ats-llm/     # OpenRouter LlmClient adapter + retry + audit sink
  ats-pdf/     # markdown2pdf PdfWriter adapter
  ats-cli/     # clap binary "ats" — stdio subcommands and "run" orchestrator
```

Dependency graph:

```
ats-cli ──► ats-core
        ├─► ats-scrape ──► ats-core
        ├─► ats-llm    ──► ats-core
        └─► ats-pdf    ──► ats-core
```

`ats-core` has no runtime-heavy deps (no chromium, no reqwest, no markdown2pdf). Adapter crates are the only places those concretions appear. Swappable behind traits for tests and future providers.

### 3. Domain model (`ats-core::domain`)

```
Resume            # parsed YAML tree; serde model matching the locked YAML contract
JobPosting        # { title: String, markdown: String }
KeywordSet        # the validated keyword JSON, deserialized
OptimizedResume   # newtype over String (Markdown)
RunPaths          # resolved per-invocation directory + file paths
TokenUsage        # { prompt, completion, total } per call; aggregated per run
LlmCallRecord     # { timestamp, stage, model, temperature, seed, prompt, response, usage, attempt, outcome }
```

Error taxonomy (via `thiserror`) — one `AtsError` with these variants, each mapped to an exit code:

| Variant | Exit | Notes |
|---|---|---|
| `Config` | 2 | missing/malformed `config.json`; field path in message |
| `Yaml(YamlDiag)` | 3 | path (e.g. `cv.work_experience[1].start_date`), reason, line/col where available |
| `Scrape(ScrapeClass)` | 4 | `auth-required`, `not-found`, `geo-blocked`, `network-timeout`, `offline`, `timeout`, `http(status)` |
| `Llm(LlmClass)` | 5 | `transient` (after retry exhaustion), `auth`, `context-exceeded`, `other` |
| `SchemaInvalid` | 6 | US-3 only, after 3 attempts |
| `Pdf` | 7 | markdown2pdf failure |
| `Io` / `Other` | 1 | fallback |

### 4. Port traits (`ats-core::ports`)

```rust
#[async_trait]
pub trait PageScraper {
    async fn fetch_html(&self, url: &str, idle_timeout: Duration) -> Result<String, ScrapeError>;
}

#[async_trait]
pub trait LlmClient {
    async fn complete(&self, req: LlmRequest) -> Result<LlmResponse, LlmError>;
}

pub trait PdfWriter {
    fn render(&self, markdown: &str, out: &Path) -> Result<(), PdfError>;
}

pub trait Clock     { fn now_local(&self) -> OffsetDateTime; }
pub trait FsLayout  { fn binary_dir(&self) -> &Path; fn cache_dir(&self) -> &Path; fn runs_dir(&self) -> &Path; fn output_dir(&self) -> &Path; fn config_path(&self) -> &Path; }
pub trait AuditSink { fn record(&self, call: &LlmCallRecord) -> Result<(), io::Error>; }
```

Each consumer gets only the methods it needs (ISP). Pipeline depends on traits, not concretions (DIP). Adapters receive config at construction (explicit DI; no globals).

### 5. Stages & pipeline (`ats-core::stage`, `ats-core::pipeline`)

Pure orchestration over ports:

- `render::baseline(yaml_bytes, cache) -> Markdown` — US-1, SHA-256 cache
- `scrape::fetch_and_convert(scraper, llm, url, cfg, audit) -> JobPosting` — US-2
- `keywords::extract(llm, posting_md, cfg, audit) -> (KeywordSet, Markdown)` — US-3 with 3-attempt schema loop
- `optimize::optimize(llm, baseline_md, keywords_md, cfg, audit) -> Markdown` — US-4
- `density::measure(final_md, keywords) -> f32` — AC-4.3
- `slug::sanitize_title(title) -> String` — AC-6.4

`pipeline::run(inputs, ports) -> RunOutputs` enforces AC-6.1: `scrape → keywords → render(cached) → optimize → pdf`. Intermediates held in memory until the scraped title is known, then the run directory is materialized with the correct `<ts>_run_<slug>/` name; no temp-rename dance (AC-6.2 — failed runs leave no partial state to reuse).

### 6. US-1 baseline render (`ats-core::render`)

- Parse YAML with `serde_yaml_ng` → `Resume`.
- Validate with `jsonschema::validator_for(...)` against `YAML_SCHEMA` (embedded). Errors mapped to `YamlDiag { path, reason, line, column }` (AC-1.4).
- Hand-rolled Markdown renderer walking the frozen template section-by-section; any optional subtree (bullet list, section array, or single field) that is absent is **omitted** entirely (AC-1.3 — no empty headings, no placeholders). `"Present"` is written verbatim (AC-1.2).
- Cache: `<binary_dir>/cache/baseline-<sha256hex>.md`. Key = SHA-256 of `yaml_bytes ++ b"\n---\n" ++ RESUME_TEMPLATE_BYTES` (AC-1.5). On hit: serve cached bytes unchanged.

### 7. US-2 scrape (`ats-scrape`)

- Reachability probe before launching Chromium: `tokio::net::lookup_host(host)` then a short TCP connect. DNS/connect failure → `ScrapeClass::Offline`.
- `ChromiumScraper::fetch_html` launches headless Chromium via `chromiumoxide::Browser::launch`, `page.goto(url)`, waits for network idle with `scrape.network_idle_timeout_ms`, reads `page.content()`.
- Error classification (AC-2.3, AC-2.5):
  - Navigation timeout → `scrape/timeout`
  - Main-frame HTTP status: 401/403 → `auth-required`, 404 → `not-found`, 451 → `geo-blocked`, other non-2xx → `http(status)`
  - Connection refused / DNS during navigation → `offline`
- Chromium binary: discovered via chromiumoxide's auto-detection (Chrome / Chromium / Edge). **Runtime prerequisite** — documented in README; clear diagnostic on miss.
- `stage::scrape` then calls `LlmClient::complete` with `PROMPT_SCRAPE_TO_MARKDOWN`, `JOB_POSTING_RESPONSE_FORMAT` on the request, and the HTML as user message; parses `{title, description}` and maps to `JobPosting { title, markdown: description }`.

### 8. US-3 keywords (`ats-core::stage::keywords`, `ats-llm`)

- Request body includes the locked `ats_keyword_extraction` schema verbatim in `response_format` (OpenAI-compatible).
- Response content is JSON; validate against the same schema with `jsonschema`. 3-attempt loop; each attempt is a fresh LLM call (same prompt; seed/temperature from config). All attempts recorded in audit log. After 3rd invalid → `SchemaInvalid` (exit 6).
- Low-signal warning when posting Markdown word count < 200 (AC-3.3).
- Markdown view for US-4: categories as bold headers, each term as "`<primary_term>` (score `<n>`, cluster `<semantic_cluster>` when present)". This is the string concatenated into the optimizer prompt.
- OpenRouter transient retry: 5 attempts, backoff `1s → 2s → 4s → 8s → 16s` (cross-cutting §5). Independent of the schema-validation loop.
- Context-length error from OpenRouter → `llm/context-exceeded`, exit 5 (AC-3.5″).

### 9. US-4 optimize & density (`ats-core::stage::optimize`, `ats-core::density`)

- Single LLM call with `PROMPT_RESUME_OPTIMIZATION`. User message concatenates `=== RESUME ===\n<baseline_md>\n=== KEYWORDS ===\n<keywords_md>`. Anti-hallucination is prompt-only per NFC-6; no diff-check (AC-4.2).
- Density (AC-4.3):
  - Collect **every** `primary_term` across all 5 arrays of the keyword JSON.
  - For each, build a whole-word, case-insensitive regex with Unicode word boundaries (`(?i)\b<escaped>\b`).
  - Sum occurrences over the optimized Markdown.
  - Denominator: total word count (Unicode-aware whitespace-split, non-empty tokens).
  - Emit: warning if `< 2%` or `> 5%`; informational when `3% ≤ density ≤ 5%`; no retries, no failure.

### 10. US-5 PDF (`ats-pdf`)

- `Markdown2PdfWriter::render(md, out)` wraps `markdown2pdf`'s public API with default styling (NFC-10). Called from `tokio::task::spawn_blocking` inside async pipeline because the crate is synchronous.
- No `pdf.style` surfacing in this version (amended).
- Always produces a `.pdf` file when called (NFC-3).

### 11. Filesystem layout (NFC-20, AC-6.4, amended for `output/`)

All paths resolved from `std::env::current_exe()?.parent()` at startup.

```
<binary_dir>/
  ats(.exe)
  config.json
  cache/
    baseline-<sha256>.md
  output/
    <ts>_<slug>_resume.pdf              # copied from the run folder at end of `ats run`
  runs/
    <ts>_run_<slug>/
      baseline.md
      posting.json
      posting.md
      keywords.json
      keywords.md
      optimized.md
      <ts>_<slug>_resume.md
      <ts>_<slug>_resume.pdf
      llm-audit.jsonl
      run.json
    <ts>_scrape_<slug>/
      posting.json
      run.json
      llm-audit.jsonl
    <ts>_keywords/
      keywords.json
      run.json
      llm-audit.jsonl
    <ts>_optimize/
      optimized.md
      run.json
      llm-audit.jsonl
    <ts>_render/
      baseline.md
      run.json
    <ts>_pdf/
      run.json
```

Rules:

- `<ts>` = `YYYYMMDD-HHMMSS` (local time, sortable) — AC-6.4.
- `<slug>` = sanitized scrape title per AC-6.4; fallback `untitled`.
- Every subcommand invocation creates its own folder (NFC-19).
- `run.json` always present; `llm-audit.jsonl` present only when the subcommand made at least one LLM call.
- `api_key` is redacted in the `run.json` config snapshot.
- Only `run` populates `output/` — it's the only subcommand with a locked "final" artifact; `pdf` writes where `--out` points.

### 12. Logging & audit trail (NFC-15/17/19)

- `tracing` + `tracing-subscriber` → **stderr**. Default formatter: **JSON lines**. Flag `--log-format=pretty` → colorized human formatter. Stdout is reserved for subcommand machine output (AC-6.3).
- Each stage wrapped in `#[instrument]` spans (`stage="scrape"|"keywords"|...`).
- Per-call token usage emitted as `tracing::info!("llm.call", ...)` plus a human-readable stderr line (NFC-17). Per-run totals logged on `run.finished`.
- Audit sink writes one JSON line per LLM call to `llm-audit.jsonl` in the invocation's folder. Fields per NFC-19: `timestamp`, `stage`, `model`, `temperature`, `seed`, `prompt`, `response`, `usage`, `attempt`, `outcome`.

### 13. CLI surface (`ats-cli`)

Binary name: `ats`. `clap` v4 derive.

Global flags:

- `--log-format <json|pretty>` (default `json`)
- `--config <path>` (default `<binary_dir>/config.json`)

Subcommands (stdio contract per AC-6.3):

| Subcommand | Inputs | Stdout | Side-effects |
|---|---|---|---|
| `ats render --yaml <path>` | YAML file | baseline MD | `runs/<ts>_render/` |
| `ats scrape <URL>` | URL | `{title, description}` JSON | `runs/<ts>_scrape_<slug>/` (audit) |
| `ats keywords` | posting MD on stdin | keyword JSON | `runs/<ts>_keywords/` (audit) |
| `ats optimize --resume <path\|-> --keywords <path\|->` | resume + keywords (one may be `-`) | optimized MD | `runs/<ts>_optimize/` (audit) |
| `ats pdf --out <path>` | MD on stdin | *(none — stdout untouched)* | writes PDF to `--out`; `runs/<ts>_pdf/` |
| `ats run --yaml <path> <URL>` | YAML + URL | *(none)* | `runs/<ts>_run_<slug>/` + copy into `output/` |

### 14. Embedded assets (`ats-core::assets`)

All `include_str!` from `packages/ats-core/assets/`. See `## Research` for verbatim content.

```
resume_template.md
yaml_schema.json
prompts/scrape_to_markdown.md
prompts/keyword_extraction.md
prompts/resume_optimization.md
schemas/job_posting_extraction.json
schemas/ats_keyword_extraction.json
```

Exposed as `pub const RESUME_TEMPLATE`, `YAML_SCHEMA`, `PROMPT_SCRAPE_TO_MARKDOWN`, `PROMPT_KEYWORD_EXTRACTION`, `PROMPT_RESUME_OPTIMIZATION`, `JOB_POSTING_RESPONSE_FORMAT`, `KEYWORD_RESPONSE_FORMAT`.

### 15. Config (`ats-core::config`)

No defaults; missing field → `AtsError::Config(path)` (exit 2). Shape:

```json
{
  "openrouter":  { "api_key": "...", "base_url": "https://openrouter.ai/api/v1" },
  "models": {
    "scrape_to_markdown":  { "name": "...", "temperature": 0.0, "seed": 42 },
    "keyword_extraction":  { "name": "...", "temperature": 0.0, "seed": 42 },
    "resume_optimization": { "name": "...", "temperature": 0.2, "seed": 42 }
  },
  "scrape":  { "network_idle_timeout_ms": 30000 },
  "retries": {
    "llm_transient_max_attempts": 5,
    "llm_transient_backoff_ms": [1000, 2000, 4000, 8000, 16000],
    "schema_validation_max_attempts": 3
  }
}
```

No env-var overrides.

### 16. Third-party dependency picks

| Area | Pick | Alternatives considered |
|---|---|---|
| Async runtime | `tokio` | `async-std` — rejected; chromiumoxide and reqwest are tokio-native. |
| HTTP | `reqwest` (rustls) | `ureq` (sync) — rejected; forces blocking off-thread calls. |
| Browser | `chromiumoxide` | (locked by NFC-9). |
| CLI | `clap` v4 derive | `argh`/`pico-args` — rejected; less standard. |
| YAML | `serde_yaml_ng` | `serde_yml`, `saphyr` — acceptable fallbacks. |
| JSON | `serde_json` | — |
| JSON Schema | `jsonschema` | `boon` — acceptable fallback. |
| Hashing | `sha2` | `blake3` — no perf need here. |
| Regex (density) | `regex` (Unicode word boundaries) | — |
| PDF | `markdown2pdf` | (locked by NFC-10). |
| Logging | `tracing` + `tracing-subscriber` | `slog`, bare `log` — rejected. |
| Error | `thiserror` (libs) + `anyhow` (`main`) | — |
| Dates | `time` | `chrono` — rejected; heavier deps. |

### 17. Impacted paths

**New:**

```
packages/ats-core/     (Cargo.toml, src/, assets/, tests/, project.json)
packages/ats-scrape/   (Cargo.toml, src/, tests/, project.json)
packages/ats-llm/      (Cargo.toml, src/, tests/, project.json)
packages/ats-pdf/      (Cargo.toml, src/, tests/, project.json)
packages/ats-cli/      (Cargo.toml, src/, tests/, project.json)
```

**Updated:**

```
Cargo.toml   (workspace.members list)
README.md    (add "Building ats" and "Running ats" sections once implemented)
```

### 18. Implementation plan (ordered vertical slices)

1. **Scaffold workspace & CLI skeleton** — five crates via `@monodon/rust`, wired into Cargo workspace; clap with all six subcommands stubbed; config loader; logging init (JSON/pretty); `FsLayout` discovery; embedded assets; error taxonomy with exit codes.
2. **US-1 `ats render`** — YAML + JSON Schema + hand-rolled renderer + SHA-256 cache.
3. **US-3 `ats keywords` + OpenRouter LlmClient + audit/retry** — LlmClient with transient retry + audit sink; schema-validated 3-attempt loop; low-signal warning.
4. **US-2 `ats scrape`** — chromiumoxide adapter + reachability probe + error classification + scrape-to-Markdown LLM call.
5. **US-4 `ats optimize`** — optimizer LLM call + density metric + warnings.
6. **US-5 `ats pdf`** — markdown2pdf adapter behind `PdfWriter`.
7. **US-6 `ats run`** — full pipeline orchestrator + run directory materialization + copy final PDF to `output/` + aggregated token usage.

Each step ships with unit + integration tests using port fakes. Real browser/LLM calls gated behind env vars.

### 19. Verification plan (copy-pasteable)

```
npx nx run ats-core:build
npx nx run ats-core:test
npx nx run ats-llm:build
npx nx run ats-llm:test
npx nx run ats-scrape:build
npx nx run ats-pdf:build
npx nx run ats-cli:build --configuration=release
npx nx run-many -t lint test build
```

Smoke tests (after each effort lands its slice):

```
ats --help
ats render --yaml fixtures/resume.yaml
ats scrape https://example.com/job
echo '...posting md...' | ats keywords
ats optimize --resume fixtures/baseline.md --keywords fixtures/keywords.json
ats pdf --out /tmp/out.pdf < fixtures/optimized.md
ats run --yaml fixtures/resume.yaml https://example.com/job
```

### 20. SOLID / YAGNI / KISS / DRY notes

- **SRP** — one crate per reason-to-change (scrape engine, LLM provider, PDF engine, CLI glue, domain).
- **OCP** — port traits; new providers swap in without touching the pipeline. `PdfWriter::render(&str, &Path)` leaves room to add style later without churning callers.
- **DIP** — `ats-core` is concretion-free; adapters depend inward.
- **ISP** — narrow ports; no god-trait.
- **KISS** — frozen template rendered by hand (no engine); in-memory intermediates until run-dir is known (no temp-rename); no `pdf.style`, no `/models` call, no local tokenizer.
- **DRY** — embedded assets are single source of truth for template/prompts/schemas; density regex, slug sanitizer, and error mapping shared in `ats-core`.
- **YAGNI** — skipped: programmatic hallucination check (NFC-6), env-var overrides, multi-URL, user templates, PDF styling, pre-flight token counting, `/models` call.

### 21. Risks & open items

- **Chromium runtime prerequisite** — not bundled; clear diagnostic on miss; documented in README.
- **`markdown2pdf`** — confirmed maintained; no fallback allocated.
- **`serde_yaml_ng`** and **`jsonschema`** — both have maintained alternatives (`serde_yml`, `boon`) if dep weight or issues appear.

### 22. Docs to update

- `README.md` — add build + run quickstart after Effort 7.
- `packages/ats-cli/README.md` (new) — CLI reference + `config.json` shape + subcommand stdio examples.
