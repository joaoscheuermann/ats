# Monorepo template (Nx)

<a alt="Nx logo" href="https://nx.dev" target="_blank" rel="noreferrer"><img src="https://raw.githubusercontent.com/nrwl/nx/master/images/nx-logo.png" width="45"></a>

This repository is an **Nx workspace template**: a starting point you can **copy, fork, or use as the basis for new monorepos**. It is not a single product—treat it as boilerplate with opinionated plugin wiring so your team can add apps and libraries quickly across **TypeScript/JavaScript**, **Rust**, and **Python**.

## What this template already includes

| Area | Package / plugin | Role |
|------|------------------|------|
| **JavaScript & TS** | [`@nx/js`](https://nx.dev/nx-api/js) | Core JS/TS support, generators, and task patterns for Nx. |
| **TypeScript** | [`@nx/js/typescript`](https://nx.dev/nx-api/js/documents/typescript-plugin) | Inferred TypeScript builds, typecheck, and `tsconfig` wiring (registered in `nx.json`). |
| **Rust** | [`@monodon/rust`](https://github.com/cammisuli/monodon) | Rust crates in the workspace with Nx-aware targets (registered in `nx.json`). |
| **Python** | [`@nxlv/python`](https://www.npmjs.com/package/@nxlv/python) | Python projects (e.g. uv-based layout), generators, and executors—dependency is **installed** so you can scaffold and run Python targets without adding the plugin yourself. |

TypeScript [project references](https://www.typescriptlang.org/docs/handbook/project-references.html) stay aligned with the Nx graph when you run builds or typecheck; you can also run `npx nx sync` (or `npx nx sync:check` in CI).

## Agent skills (`.agents/skills`)

The template ships **personal agent skills**: Markdown playbooks under [`.agents/skills`](.agents/skills) that tools like Cursor can load to steer AI assistants toward consistent Nx workflows, monorepo hygiene, and a structured journal-based dev process. Each skill lives in its own folder with a `SKILL.md` (name and description in the file frontmatter). Below is what each one is for.

### Nx workspace and monorepo operations

| Skill | What it does |
|-------|----------------|
| **`nx-workspace`** | Read-only exploration of the Nx workspace: list projects, inspect configuration and targets, understand dependencies, and debug failed `nx` commands before running tasks. |
| **`nx-generate`** | Run Nx **generators** the right way (discover plugins, prefer `--no-interactive`, read generator behavior, match repo patterns, verify with lint/test/build). Use when scaffolding apps, libraries, or migrations. |
| **`nx-run-tasks`** | How to **execute** Nx tasks: single-project runs, `run-many`, `affected`, inferred targets, and checking available targets (e.g. via `nx show project`). |
| **`nx-plugins`** | **Discover and install** Nx plugins (`nx list`, `nx add <plugin>`) when you need new framework or stack support. |
| **`nx-import`** | Bring **external repos** into the workspace with `nx import` while preserving history—strategies, directory layout, and version quirks (e.g. non-interactive flags). |
| **`link-workspace-packages`** | Wire **workspace dependencies** across packages using the repo’s package manager (npm, pnpm, yarn, bun)—fix `@org/*` resolution and “cannot find module” without fake `tsconfig` path hacks. |
| **`monitor-ci`** | **Watch Nx Cloud CI** pipelines, interpret runs, and coordinate self-healing / fix loops when you care about branch status and automated remediation (not a replacement for raw `gh`/`glab` for generic Git ops). |

### Design, coding standards, and implementation

| Skill | What it does |
|-------|----------------|
| **`architect`** | **Planning only**: architecture and refactor design for an Nx monorepo with journaling, explicit human gates, trade-offs, and effort decomposition—does not implement product code. |
| **`coding-conventions`** | Shared **principles and rules** (SRP, OCP, DIP, ISP, KISS, DRY, early returns, functional style, DI) plus references for **@nxlv/python** and **@monodon/rust** scaffolding—load with architect or developer. |
| **`developer`** | **Implements** work from a journal Effort (or an approved bug-fix plan): explore the repo, write code and tests, follow `coding-conventions`, and hand off verification—does not mutate Effort status or journal metadata itself. |
| **`tester`** | **Verifies** changes against Effort or bug criteria using Nx targets, builds, tests, and evidence-backed pass/fail reporting; does not own arbitrary product edits (delegates fixes back to developer when needed). |

### Journal workflow (optional structured delivery)

These skills assume file-backed journals under `.journal/` (paths, slugs, efforts, bugs, decisions). They chain together for slice-by-slice delivery after architecture is approved.

| Skill | What it does |
|-------|----------------|
| **`journal-manager`** | **CRUD for journal files**: create/update entries, efforts, bug reports, decision logs; resolve `<entry_dir>` from a slug; the low-level file protocol other skills build on. |
| **`decomposer`** | Splits an approved **`## Architecture`** section in `ticket.md` into **ordered Effort** files—vertical slices with runnable outcomes; does not implement code. |
| **`effort-executor`** | Runs efforts **in sequence**: status lifecycle, delegates each effort to **developer**, captures change summaries and decision logs; does not write application code itself. |

### Debugging workflow

| Skill | What it does |
|-------|----------------|
| **`debug-coordinator`** | **Orchestrates** bug workflows: journal/bug setup, triage, loops with **debugger** → human-approved **developer** fixes → **tester** verification and documentation; does not patch product code directly. |
| **`debugger`** | **Hypothesis-driven investigation**: context, exploration, falsifiable hypotheses, diagnostics, structured findings—returns outcomes like root cause vs. needs-more-info; does not fix code or talk to the user without a coordinator. |

---

If you copy this template, keep or trim `.agents/skills` to match how your team uses AI assistants; the skills are documentation and process, not runtime dependencies.

## Using this template for a new project

1. **Copy the repo** (fork, duplicate, or `git clone` into a new directory) and rename it to match your product or organization.
2. Update **root metadata**: `package.json` name (e.g. `@your-org/source`), license, and workspace paths if you change `packages/*`.
3. **Install dependencies**: `npm install` (this workspace uses npm workspaces under `packages/*`).
4. **Replace placeholders** in any generated examples (import paths like `@my-org/...`, project names).
5. **Add projects** with Nx generators, e.g. `@nx/js:library`, `@monodon/rust` generators, `@nxlv/python` generators—see each plugin’s docs for exact commands.

Explore the workspace graph anytime:

```sh
npx nx graph
```

## Everyday commands

Run a target for a project:

```sh
npx nx <target> <project-name>
```

Example for a publishable JS/TS library (adjust names to your org):

```sh
npx nx g @nx/js:lib packages/pkg1 --publishable --importPath=@your-org/pkg1
npx nx build pkg1
```

Keep TS references in sync manually if needed:

```sh
npx nx sync
```

Version and release (when configured):

```sh
npx nx release
```

Use `--dry-run` with `nx release` to preview.

## Tooling & CI

- **Nx Console** — [Editor integration](https://nx.dev/getting-started/editor-setup) for tasks and generators in VS Code and IntelliJ.
- **CI** — Generate a workflow for non–GitHub Actions providers: `npx nx g ci-workflow`. For GitHub Actions, follow [Nx CI docs](https://nx.dev/ci/intro/ci-with-nx).

This template sets `neverConnectToCloud` in `nx.json`; enable [Nx Cloud](https://nx.dev/ci/intro/why-nx-cloud) in your fork if you want remote caching and distributed CI.

## Learn more

- [Nx docs](https://nx.dev)
- [Nx plugins](https://nx.dev/concepts/nx-plugins)
- [Managing releases](https://nx.dev/features/manage-releases)
- [Community](https://go.nx.dev/community)
