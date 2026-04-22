# Open-Closed Principle (OCP)

Open this file when the question is how to add a new behavior, artifact type, provider, or workflow without rewriting existing flows.

## Meaning

- Keep a design open for extension and closed for modification.
- Add new behavior by plugging into a stable seam instead of repeatedly editing the same high-risk modules.
- Treat existing Nx targets, package boundaries, and artifact pipelines as extension points before inventing a new framework.

## Apply in a typical Nx monorepo

- Extend through new Nx targets, additive scripts, new artifacts, or new modules that plug into an existing flow.
- Add explicit configuration inputs (paths, feature flags, providers) instead of hardcoding them across layers.
- Use existing codegen and schema seams as the supported way to grow cross-language behavior (e.g. IDL → generated readers).
- Keep OCP separate from DIP: OCP answers *where* to extend; DIP answers *which direction* dependencies point.

## Decision Rules

- Prefer adding a new target on an existing project over one catch-all script for every mode.
- Prefer additive configuration or a new focused module over editing many switch statements across stacks.
- When a new artifact type is needed, define its source of truth and generation path instead of one-off loaders in several places.
- Extend an existing interface only when all current clients need the new shape. Otherwise add a new type or operation.
- If introducing one variant touches many unrelated files, strengthen the seam first.

## Avoid

- Hardcoding provider choices, asset names, or thresholds in global statics.
- Editing the same flow in app, bindings, runtime, and tooling every time a variant appears.
- Adding a generic orchestration layer before proving current seams cannot absorb the change.

## Examples

- Add `npx nx run <project>:<target>` for a workflow instead of a single mega-script.
- Add a module behind existing initialization inputs instead of branching UI for each backend.
- Add a schema field and regenerate code instead of hand-editing generated stubs in multiple languages.

## Sources

- [Robert C. Martin, "The Open Closed Principle"](https://blog.cleancoder.com/uncle-bob/2014/05/12/TheOpenClosedPrinciple.html)
- [Robert C. Martin, "An Open and Closed Case"](https://blog.cleancoder.com/uncle-bob/2013/03/08/AnOpenAndClosedCase.html)
