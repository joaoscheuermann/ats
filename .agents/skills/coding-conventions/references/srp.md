# Single Responsibility Principle (SRP)

Open this file when the main design question is "which layer should own this change?" or "why should this module exist at all?"

## Meaning

- Treat a responsibility as a set of changes driven by the same actor or reason.
- Group code that changes together for the same reason. Separate code that changes for different reasons.
- Use SRP to keep major ownership lines stable instead of mixing UI, native runtime, offline tooling, and training in one place.

## Apply in a typical Nx monorepo

- **Client apps** (`apps/*`): UX, navigation, lifecycle, platform integration.
- **FFI / bindings packages** (if present): language boundary, generated bindings, marshaling — not business policy for the core runtime.
- **Native or performance-critical runtime** (`packages/*` native libs): latency-sensitive paths, inference, transforms — keep orchestration thin at the boundary.
- **Offline tooling** (`packages/*` builders, `data/*`, batch jobs): artifact generation, ETL, validation — not shipped inside thin client loops.
- **Model / training** (`models/*` or dedicated training apps): architecture, train loops, export — feed product runtime via artifacts, not direct imports from training into app hot paths.

## Decision Rules

- If a change is driven by app UX or lifecycle, keep it in `apps/*` and avoid pushing it into native core or training code unless the boundary contract requires it.
- If a change affects runtime performance, critical transforms, or on-device behavior, keep the core logic in the designated runtime package(s).
- If a change exists only to produce or validate artifacts offline, keep it in tooling packages or scripts rather than app startup paths.
- If a change is about training quality, export, or offline metrics, keep it in training/tooling areas and integrate through versioned artifacts and contracts.
- If one proposed module would need sign-off from different teams or “reasons to change” often conflict, split it before implementation.

## Avoid

- Letting UI layers own rules that must stay bit-identical with native runtime (duplicate sources of truth).
- Growing a thin binding layer into a second runtime with policy that belongs in apps or core.
- Mixing dataset generation, batch rendering, and UX in one feature module without clear ownership.

## Examples

- Add an offline build step in a dedicated package instead of hiding it in app `main()`.
- Add a native pipeline stage and expose only minimal inputs/outputs through the existing boundary.
- Keep a “tuning” screen in the app but apply thresholds in the runtime layer that owns execution.

## Sources

- [Robert C. Martin, "The Single Responsibility Principle"](https://blog.cleancoder.com/uncle-bob/2014/05/08/SingleReponsibilityPrinciple.html)
