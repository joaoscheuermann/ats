# Functional programming standard (TypeScript/JavaScript-oriented)

This convention enforces a **functional** style where it matters most (core logic, transforms, generators). Adjust or scope down for stacks where imperative code is idiomatic (Rust, Dart, etc.) — the spirit is still **immutability at boundaries**, **pure core logic**, and **thin I/O wrappers**.

## Instructions

When writing or refactoring TypeScript/JavaScript in the monorepo:

1. **Strict immutability**
   - Prefer `const` for bindings.
   - Avoid mutating objects or arrays in core logic; use structural sharing (`{...obj}`, `[...arr]`, `Object.assign({}, ...)`) when appropriate.

2. **Pure functions**
   - Deterministic: same input yields same output.
   - No side effects inside core transforms. Isolate I/O, network, and mutation at boundaries.

3. **Declarative data flow (when applying this rule to JS/TS)**
   - Prefer `.map`, `.filter`, `.reduce`, `.find`, `.every` over imperative loops for collection transforms *when readability wins*.
   - Teams may relax loop bans for performance-critical hot paths — document exceptions.

4. **Composition over deep class hierarchies**
   - Prefer small functions composed together over large inheritance trees where practical.

5. **Expression-oriented assignments**
   - Prefer ternaries and short-circuit expressions for simple value selection.

6. **Types**
   - Use `readonly` on interfaces where immutability is intended.
   - Consider discriminated unions for complex state machines.

## Boundary functions

Network I/O, timers, `main.ts`, file reads, and framework callbacks are impure. Keep them at the **edges** — thin wrappers that call pure transforms. Prefer extracting reusable pure pipelines into `packages/*` libraries.

## Constraints (for teams that adopt the strict profile)

- Avoid `array.push`, `splice`, `delete`, and unnecessary `let` reassignment in new code where FP rules apply.
- Shared mutable singletons are discouraged; pass explicit dependencies (see [dependency-injection.md](dependency-injection.md)).

## Examples

### Bad (imperative)

```typescript
let activeTotal = 0;
for (let i = 0; i < products.length; i++) {
  if (products[i].isActive) {
    activeTotal += products[i].price;
  }
}
```

### Good (functional)

```typescript
const activeTotal = products
  .filter((product) => product.isActive)
  .reduce((sum, product) => sum + product.price, 0);
```
