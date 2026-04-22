# @monodon/rust

Open this file when creating, scaffolding, or configuring a Rust crate in an Nx workspace that uses this plugin.

## What It Is

`@monodon/rust` is an Nx plugin that integrates Cargo and Rust projects into the Nx workspace. It provides generators to scaffold binary and library crates and executors that wrap `cargo build`, `cargo test`, `cargo clippy`, and `cargo run` with Nx caching and target-dir management.

- GitHub: <https://github.com/cammisuli/monodon/tree/main/packages/rust>
- Registered in `nx.json` as `"@monodon/rust"` when installed.

## Generators

### Library

```
npx nx generate @monodon/rust:library <name>
```

Creates a Rust library crate. Use `--napi` to set up napi bindings for Node.js interop.

### Binary

```
npx nx generate @monodon/rust:binary <name>
```

Creates a standalone Rust binary application.

## Standard Executors

| Target | Executor | Purpose |
|---|---|---|
| `build` | `@monodon/rust:build` or `@monodon/rust:check` | Compile the crate (`check` skips codegen, faster for CI) |
| `test` | `@monodon/rust:test` | Run `cargo test` |
| `lint` | `@monodon/rust:lint` | Run `cargo clippy` |
| `run` | `@monodon/rust:run` | Run `cargo run` (binaries only) |

### Common Executor Options

All executors accept these additional options:

| Option | Example | Notes |
|---|---|---|
| `toolchain` | `stable`, `nightly` | Defaults to `stable` |
| `target` | `aarch64-apple-darwin` | Cross-compilation target triple |
| `profile` | `dev`, `release` | Cargo build profile |
| `release` | `true` | Shorthand for `--profile=release` |
| `target-dir` | `dist/target/my_crate` | Redirects build artifacts out of the source tree |
| `features` | `bmp` | Comma-separated Cargo features |
| `all-features` | `true` | Enable all Cargo features |
| `args` | `-- --nocapture` | Forward additional arguments to the underlying command |

## Workspace conventions (customize for your repo)

- A workspace `Cargo.toml` at the repo root with `resolver = '2'` is common for multi-crate setups.
- New crates are typically listed in `[workspace].members` in that manifest.
- Build artifacts are often redirected under something like `dist/target/<project-name>` via `target-dir`, keeping the source tree clean.
- A `build` target might use `@monodon/rust:check` for fast type-check feedback; `test` may use `@monodon/rust:test` with a `production` configuration for `release: true`; `lint` may use `@monodon/rust:lint` (clippy).
- A `clean` target can wrap `cargo clean` via `nx:run-commands` if needed.

## project.json Template

Canonical target block for a new Rust crate. Replace `<crate-name>` with the actual crate name.

```json
{
  "name": "<crate-name>",
  "$schema": "../../../node_modules/nx/schemas/project-schema.json",
  "projectType": "library",
  "sourceRoot": "<project-root>/src",
  "targets": {
    "build": {
      "executor": "@monodon/rust:check",
      "outputs": ["{options.target-dir}"],
      "options": {
        "target-dir": "dist/target/<crate-name>"
      }
    },
    "test": {
      "cache": true,
      "executor": "@monodon/rust:test",
      "outputs": ["{options.target-dir}"],
      "options": {
        "target-dir": "dist/target/<crate-name>"
      },
      "configurations": {
        "production": {
          "release": true
        }
      }
    },
    "lint": {
      "cache": true,
      "executor": "@monodon/rust:lint",
      "outputs": ["{options.target-dir}"],
      "options": {
        "target-dir": "dist/target/<crate-name>"
      }
    },
    "clean": {
      "executor": "nx:run-commands",
      "dependsOn": [],
      "options": {
        "command": "cargo clean",
        "cwd": "{projectRoot}"
      }
    }
  },
  "tags": []
}
```

After creating the `project.json`, add the crate path to the workspace `Cargo.toml` `[workspace].members` array (exact paths depend on your layout), for example:

```toml
[workspace]
members = [
  "packages/rust/existing_crate",
  "packages/rust/<crate-name>"
]
```

## Sources

- [GitHub: @monodon/rust](https://github.com/cammisuli/monodon/tree/main/packages/rust)
