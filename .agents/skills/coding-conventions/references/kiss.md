# Keep It Simple, Stupid (KISS)

Open this file when a proposal starts adding layers, orchestrators, abstractions, or generalization that may be heavier than the current need.

## Meaning

- Prefer the simplest design that works, reveals intent, and avoids accidental complexity.
- Start from direct flows before generic frameworks or indirection.
- Use simplicity as a forcing function: fewer moving parts, clearer ownership.

## Apply in a typical Nx monorepo

- Extend the existing workspace layout before adding a cross-cutting “platform” package.
- Extend current app/package boundaries before inventing a new coordination tier.
- Reuse existing codegen and artifact paths instead of parallel pipelines for the same outcome.
- Prefer explicit commands, small modules, and documented artifact locations over hidden registries.

## Decision Rules

- Prefer one focused module inside an existing package over a new cross-stack layer.
- Prefer one new Nx target over a custom dispatcher that interprets many modes.
- Prefer explicit wiring and direct calls over service locators until several real use cases prove otherwise.
- Defer generalization until a second concrete consumer validates the shape.
- If the design needs a long preamble before code exists, it may be too abstract for now.

## Avoid

- “Manager/coordinator” layers that mostly forward calls.
- Plugin systems for a single workflow.
- New artifact formats when an existing schema or pipeline fits.
- Hypothetical reuse at the cost of present clarity.

## Examples

- Add a target under an existing project instead of a wrapper CLI that only forwards to the same script.
- Add one module for a pipeline stage instead of a generic stage graph when order is fixed.
- Add a thin integration module in the app instead of a second abstraction over the FFI surface for one screen.

## Sources

- [Martin Fowler, "BeckDesignRules"](https://martinfowler.com/bliki/BeckDesignRules.html)
- [Martin Fowler, "PrinciplesOfXP"](https://martinfowler.com/bliki/PrinciplesOfXP.html)
- [Martin Fowler, "Yagni"](https://www.martinfowler.com/bliki/Yagni.html)
