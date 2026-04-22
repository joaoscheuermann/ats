---
name: coding-conventions
description: Shared design principles, Nx Python/Rust scaffolding references, and coding rules for Nx monorepos. Load when designing architecture (with architect) or before implementing code (with developer). Covers SRP, OCP, DIP, ISP, KISS, DRY plus reusability practice, @nxlv/python, @monodon/rust, early returns, functional style, and explicit dependency injection.
---

# Coding conventions

Use this skill as the **single source** for principles and coding rules referenced by the `architect` and `developer` skills.

## When to load

| Role | What to read |
|------|----------------|
| **Architect / planning** | Design principles ([references/srp.md](references/srp.md) through [references/kiss.md](references/kiss.md), [references/dry.md](references/dry.md)). Open [references/nxlv-python.md](references/nxlv-python.md) or [references/monodon-rust.md](references/monodon-rust.md) when scaffolding new packages or crates. |
| **Developer / implementation** | Read [references/dry.md](references/dry.md) (includes reusability workflow), [references/early-returns.md](references/early-returns.md), [references/functional-programming.md](references/functional-programming.md), [references/dependency-injection.md](references/dependency-injection.md). Use Nx plugin refs when touching Python or Rust project layout. |

## Reference index

### Design principles

1. [SRP — Single Responsibility](references/srp.md) — who owns what layer
2. [OCP — Open/Closed](references/ocp.md) — extend via seams, not rewrites
3. [DIP — Dependency Inversion](references/dip.md) — dependency direction across stack
4. [ISP — Interface Segregation](references/isp.md) — small contracts at boundaries
5. [KISS](references/kiss.md) — prefer simple extensions before new machinery
6. [DRY + reusability](references/dry.md) — one source of truth; search before adding code

### Nx scaffolding

- [@nxlv/python](references/nxlv-python.md) — uv-based Python projects, targets, `project.json` template
- [@monodon/rust](references/monodon-rust.md) — Rust crates in Nx, generators, `project.json` template

### Coding rules

- [Early returns & guard clauses](references/early-returns.md)
- [Functional programming standard (TS/JS-oriented)](references/functional-programming.md)
- [Dependency injection & explicit parameters](references/dependency-injection.md)

## Progressive disclosure

Load only the reference files relevant to the current task. Keep principle files small; avoid pasting entire docs into chat when a link path suffices.
