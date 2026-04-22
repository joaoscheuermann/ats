# Don't Repeat Yourself (DRY) and reusability-first practice

Open this file when logic, schemas, artifact rules, or workflow knowledge appears in more than one place — or **before writing new implementation code** to decide whether to centralize or reuse existing code.

## Meaning (DRY)

- Keep each piece of knowledge in one authoritative place.
- Treat DRY as duplication of *knowledge*, not just duplicated lines.
- Centralize the rule or source of truth, then regenerate or reuse downstream representations.

## Apply in a typical Nx monorepo

- Keep schemas, IDLs, or proto definitions as the source of truth for generated code in multiple languages.
- Keep foreign-function or RPC definitions as the source of truth for generated client stubs.
- Put shared native behavior in one package when multiple apps or pipelines need the same semantics.
- Put shared offline Python or Node helpers in `packages/*` when more than one workflow needs the same rule.
- Document one artifact path from build/export → package → consumer; avoid tribal knowledge in README fragments only.

## Decision Rules

- Move shared logic into the layer that already owns that runtime (native vs tooling vs app).
- Regenerate from the canonical spec instead of hand-editing generated outputs.
- Share fixtures and validation rules when training/serving parity matters; do not force sharing when runtimes genuinely differ.
- If two modules encode the same business rule differently, pick one source of truth and route the other through it.

## Reusability-first workflow (operational)

Before generating new code:

1. **Workspace audit** — Search the repo (ripgrep, symbol search, or your agent’s codebase search) for similar logic. Assume duplication may already exist. Typical places: `apps/*/src/**`, `packages/**`, tooling scripts.
2. **Proactive splitting** — When implementing a feature, extract logic that could serve a second caller (even if only one exists today), especially generic transforms (formatting, mapping, sanitization) or types shared across modules.
3. **Cross-app extraction** — When a utility serves multiple apps, place it in `packages/*` using your workspace generator (for example `npx nx g @nx/js:lib packages/<name>` for JS/TS libraries). Customize the generator to match your stack.
4. **Consistency** — Reused code should follow project naming and export conventions.

### Constraints (reusability)

- Do not redefine logic that already exists; extend or generalize the existing helper when possible.
- When adapting an existing utility, prefer backward-compatible extension over a parallel copy.
- Tell the human which existing module you reused and where.
- When creating a new shared package, use the workspace generator rather than hand-rolling undocumented `packages/` trees.

## Avoid

- Copying schema knowledge into hand-maintained parsers in multiple languages.
- Repeating artifact path conventions across scripts without shared config or targets.
- Duplicating transforms in UI and native code when they must stay identical — extract or generate instead.
- Over-merging unrelated concepts that only look similar.

## Examples

- Define a schema field once; regenerate all language bindings.
- Keep FFI signatures in one place and regenerate thin clients.
- Move a reusable embedding or parse helper into `packages/*` instead of copying into each app.

## Sources

- [Pragmatic Programmer Tips](https://www.forums.pragprog.com/tips/)
- [The Pragmatic Programmer, DRY excerpt](https://media.pragprog.com/titles/tpp20/dry.pdf)
