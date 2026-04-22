# Dependency Inversion Principle (DIP)

Open this file when the design question is which layer may depend on which other layer — especially across client apps, language bindings, native runtime, and offline tooling.

## Meaning

- Keep high-level policy from depending directly on low-level details.
- Make both sides depend on stable boundaries: plain data, explicit configs, small interfaces.
- Point source dependencies toward the policies that should survive library or vendor swaps.

## Apply in a typical Nx monorepo

- Flow generally goes **apps → bindings/surface → runtime/tooling APIs**, not the reverse into app internals.
- Client code should depend on declared contracts (FFI, RPC, IPC), not on how sessions, indexes, or parsers are implemented inside native code.
- Native runtime owns vendor-specific details behind narrow modules; do not leak them into UI or training notebooks.
- Offline pipelines produce artifacts and configuration consumed at runtime; runtime should not import training source trees.
- Pair with ISP: keep contracts small so callers do not depend on unused surface area.

## Decision Rules

- Pass paths, credentials, providers, and limits at initialization or call boundaries — not via hidden globals.
- Put library-specific setup (execution providers, parsers, index loaders) behind packages that own those concerns.
- Prefer explicit structs, generated bindings, and versioned artifact formats as contracts. Avoid side channels.
- If a feature description names a vendor, push that name downward until the feature can be described in workspace terms.
- When two layers coordinate, define a minimal contract instead of reaching into internals.

## Avoid

- Client code depending on native internals beyond the published boundary.
- Runtime discovering configuration from undeclared process-wide state.
- Training or notebooks as the only definition of runtime policy without a shared artifact or spec.
- Bypassing the official boundary with ad hoc globals when the fix is to extend the contract.

## Examples

- Expose `initialize` / `search` (or similar) through the binding layer so clients depend on the API, not on index internals.
- Accept provider or threshold inputs at init time instead of burying defaults in statics.
- Export models or indexes from tooling, then pass file paths or byte blobs into runtime init.

## Sources

- [Robert C. Martin, "The Clean Architecture"](https://blog.cleancoder.com/uncle-bob/2012/08/13/the-clean-architecture.html)
- [Robert C. Martin, "A Little Architecture"](https://blog.cleancoder.com/uncle-bob/2016/01/04/ALittleArchitecture.html)
- [Robert C. Martin, "Solid Relevance"](https://blog.cleancoder.com/uncle-bob/2020/10/18/Solid-Relevance.html)
