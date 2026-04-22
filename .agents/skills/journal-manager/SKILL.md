---
name: journal-manager
description: Reads, writes, and updates local developer journal files under .journal/. Use for journal entries, effort files, bug reports, decision logs, and resolving entry directories by slug.
---

# Journal manager

Manage journal files under `.journal/` using your environment’s file and search tools (read, write, glob, patch/replace).

## Journal structure

Entries live under a month bucket and a dated folder: **`DD-MM-YYYY-<slug>`** (kebab-case slug).

```
.journal/
  <MM-YYYY>/
    <DD-MM-YYYY>-<slug>/
      ticket.md       # frontmatter + ## Prompt, ## Research, ## Architecture
      decisions.md    # append-only decision log
      efforts/
        01-<slug>.md
      bugs/
        bug-001-<slug>.md
```

## Resolve journal entry directory

Many operations take a **`slug`** (`slug:` in `ticket.md` frontmatter). The path is **not** `.journal/<slug>/`.

**Resolve `slug` → `<entry_dir>`:**

1. Glob `.journal/*/*/ticket.md` (two segments under `.journal/` after the month folder).
2. For each file, read YAML frontmatter and find `slug: <value>`.
3. Keep entries where `<value>` equals the given slug (trimmed, exact).
4. **None** → slug missing. **One** → parent folder is `<entry_dir>`. **Multiple** → use the ticket whose `created:` is latest (`YYYY-MM-DD HH:MM`); tie-break by latest date in the folder name.

`<entry_dir>` is always `.journal/<MM-YYYY>/<DD-MM-YYYY>-<slug>/`.

Effort/bug paths under `<entry_dir>` need no extra resolution.

## Operations

### Create a Journal Entry

**Input:** `slug`, `prompt` (verbatim markdown), optional `status` (default `planning`; use `debugging` for debug journals).

The prompt MUST appear under `## Prompt` **exactly** as given.

1. Current time → `created_line` `YYYY-MM-DD HH:MM`, month bucket `MM-YYYY`, folder prefix `DD-MM-YYYY`.
2. Create `.journal/<MM-YYYY>/<DD-MM-YYYY>-<slug>/` (if it exists, stop — conflict).
3. Write `ticket.md`:

```markdown
---
status: <status>
created: <created_line>
slug: <slug>
---

## Prompt

<verbatim prompt>

## Research

(empty)

## Architecture

(empty)
```

4. Write `decisions.md`: `# Decision Log: <slug>`
5. Ensure `efforts/` and `bugs/` exist (e.g. `.gitkeep`).

### Read a Journal Entry

**Input:** `slug` or `<entry_dir>`.

Resolve if needed; read `<entry_dir>/ticket.md` in full.

### Update a Section

**Input:** `slug`, `section` (heading name e.g. `Architecture`), `content`.

Read `ticket.md`, replace body between `## <section>` and the next `## ` (or EOF). Append section if missing. Preserve frontmatter.

### Update Status

**Input:** `slug`, new `status`.

Patch `status:` in `ticket.md` frontmatter.

### Create an Effort

**Input:** `slug`, `order`, `title`, `description` (full five-section body).

Resolve `<entry_dir>`. File slug from title (kebab-case, max ~5 words). Write `efforts/<NN>-<file-slug>.md`:

```markdown
---
status: pending
order: <order>
created: <YYYY-MM-DD HH:MM>
title: "<title>"
---

<description>
```

### Read an Effort

**Input:** effort file path. Return full file.

### List Efforts

**Input:** `slug`.

Glob `<entry_dir>/efforts/*.md` (exclude `.gitkeep`). Read frontmatter: `status`, `order`, `title`. Return sorted by `order` with paths.

### Update Effort Status

**Input:** effort path, `status` (`pending` | `in_progress` | `done`).

Patch `status:` in frontmatter.

### Append Change Summary

**Input:** effort path, `summary` markdown.

Append (or add) `## Change Summary` with the summary content.

### Log Decision

**Input:** `slug`, `agent`, `context`, `decision`, `rationale`, optional `alternatives`.

Append to `decisions.md`:

```markdown
### <YYYY-MM-DD HH:MM> - <agent>

**Context**: <context>
**Decision**: <decision>
**Rationale**: <rationale>
**Alternatives considered**: <alternatives or "None">
```

### Create a Bug Report

**Input:** `slug`, `title`, `description`.

Count `bugs/bug-*.md`, next id, file slug from title. Write `bugs/bug-<NNN>-<file-slug>.md`:

```markdown
---
status: open
created: <YYYY-MM-DD HH:MM>
title: "<title>"
---

<description>
```

### Check for Duplicate Bugs

**Input:** `slug`, `feature_area`, `symptom`.

Search `bugs/*.md` for keywords; return candidate paths (caller decides duplicate).

### Close a Bug

**Input:** bug file path.

Set `status: closed` in frontmatter.

## Section editing protocol

Sections use `## ` headings. Read latest content before edit; do not reorder or drop sections; preserve `---` frontmatter.

## Timestamps

`YYYY-MM-DD HH:MM` local time when creating entries.

## Constraints

- Only mutate under `.journal/`.
- Do not delete journal files; status + append only.
- Do not change `created` or effort `order` after creation.
