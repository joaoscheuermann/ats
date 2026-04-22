# Dependency injection and explicit parameters

Functions should receive their Dependencies as **explicit arguments** rather than importing, constructing, or hiding them in module state — when this pattern fits your language and framework.

## Instructions

1. **Explicit dependencies**: Declare external resources a function needs as parameters. Avoid reaching into module-level mutable singletons or opaque globals for “hidden” inputs.

2. **No hidden state**: Closure-captured mutable variables that act as implicit singletons (e.g. `let token` inside a factory object) are discouraged. Prefer the caller owning transient state.

3. **Entry-point wiring**: Resolve clients, config, and tokens at the application entry (`main.ts`, `main.rs` `fn main`, etc.) and pass them down explicitly.

4. **Standalone functions over opaque factories**: Prefer exported functions that take dependencies as parameters over factories that return closures holding hidden state — unless your ecosystem idiomatically uses something else (document the exception).

## Examples

### Bad (hidden state)

```typescript
export const createClient = () => {
  let token: string | null = null;

  const login = async () => {
    token = await authenticate();
  };

  const fetchData = async () => {
    if (!token) throw new Error('Not authenticated');
    return await get('/data', {
      headers: { Authorization: `Bearer ${token}` },
    });
  };

  return { login, fetchData };
};
```

### Good (explicit dependencies)

```typescript
export const login = async (): Promise<string> => {
  const result = await authenticate();
  return result.token;
};

export const fetchData = async (token: string): Promise<any> =>
  get('/data', { headers: { Authorization: `Bearer ${token}` } });
```

```typescript
// main.ts — entry point wires everything
const token = await login();
const data = await fetchData(token);
```

### Bad (importing config internally)

```typescript
export const buildPayload = () => {
  const config = getConfig(); // hidden dependency
  return { ...config.defaults, timestamp: Date.now() };
};
```

### Good (config as parameter)

```typescript
export const buildPayload = (defaults: Readonly<Defaults>): Payload => ({
  ...defaults,
  timestamp: Date.now(),
});
```

## Constraints

- **Avoid** module-level `let` used as shared mutable state across unrelated calls.
- **Avoid** factories that capture dependencies in closures when simple explicit parameters would do.
- **Prefer** impure dependencies (tokens, config, HTTP clients) visible in function signatures for code you control.
