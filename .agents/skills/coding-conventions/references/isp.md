# Interface Segregation Principle (ISP)

Open this file when an interface, request DTO, RPC payload, or FFI struct is growing and you need to decide whether to split it.

## Meaning

- Keep clients from depending on methods or fields they do not use.
- Prefer several small role-specific interfaces over one broad contract everyone must carry.
- Apply ISP aggressively at language boundaries where wide contracts create churn.

## Apply in a typical Nx monorepo

- Keep boundary operations narrow: init, infer, query, load resource, teardown — usually separate entry points.
- Keep configuration objects focused on one concern instead of every optional knob for every workflow.
- Split unrelated concerns (e.g. export-time vs runtime-init) even when they mention the same product feature.
- Treat generated bindings and serialized formats as contracts that should stay easy to regenerate.

## Decision Rules

- Add a new operation or DTO when only some callers need the new fields.
- Separate one-time setup inputs from per-request inputs.
- Keep payloads flat when possible: primitives, byte buffers, small structs — avoid nested “everything” bags.
- If explaining one type requires unrelated workflows, split the type.

## Avoid

- One mega-request carrying buffers, debug flags, paths, and options for unrelated operations.
- Config objects threaded everywhere where most fields are unused per call site.
- Extending a shared struct for one new caller when it forces churn on all others.

## Examples

- Keep database/index `initialize` inputs separate from `search` so startup-only options do not pollute hot paths.
- Add a dedicated API for one pipeline stage instead of a monolithic “do everything” call when callers need only one step.
- Split offline export settings from runtime init settings even when both reference the same model family.

## Sources

- [Robert C. Martin, "The Clean Architecture"](https://blog.cleancoder.com/uncle-bob/2012/08/13/the-clean-architecture.html)
- [Robert C. Martin, "Solid Relevance"](https://blog.cleancoder.com/uncle-bob/2020/10/18/Solid-Relevance.html)
