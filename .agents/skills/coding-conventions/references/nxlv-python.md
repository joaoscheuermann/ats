# @nxlv/python

Open this file when creating, scaffolding, or configuring a Python package in an Nx workspace that uses this plugin.

## What It Is

`@nxlv/python` is an Nx plugin that manages Python projects using the **uv** package manager (common in uv-based workspaces). It provides generators to scaffold new projects and executors for dependency management, building, linting, formatting, testing, and running commands inside a properly activated virtual environment.

- GitHub: <https://github.com/lucasvieirasilva/nx-plugins/tree/main/packages/nx-python>
- Registered in `nx.json` as `"@nxlv/python"` when installed.

## Generator

Scaffold a new Python project with uv:

```
npx nx generate @nxlv/python:uv-project <name> [options]
```

Key options:

| Option | Type | Default | Notes |
|---|---|---|---|
| `--directory` | string | same as name | Relative path for the project root |
| `--projectType` | string | `application` | `application` or `library` |
| `--packageName` | string | name | PyPI-style package name |
| `--moduleName` | string | name with `_` | Python module directory name |
| `--linter` | string | `ruff` | `flake8`, `ruff`, or `none` |
| `--unitTestRunner` | string | `pytest` | `pytest` or `none` |
| `--publishable` | boolean | `true` | Whether the package can be published |
| `--buildLockedVersions` | boolean | `true` | Pin versions in build output |
| `--buildBundleLocalDependencies` | boolean | `true` | Bundle local deps into the wheel |

## Standard Executors

Projects using this plugin typically expose these targets. Use the canonical executor names; do not invent parallel conventions without team agreement.

| Target | Executor | Purpose |
|---|---|---|
| `lock` | `@nxlv/python:lock` | Regenerate `uv.lock` without upgrading |
| `sync` | `@nxlv/python:sync` | Sync venv with locked dependencies |
| `add` | `@nxlv/python:add` | Add a dependency (`--name`, optional `--local`) |
| `update` | `@nxlv/python:update` | Upgrade a dependency |
| `remove` | `@nxlv/python:remove` | Remove a dependency |
| `install` | `@nxlv/python:install` | Install project into venv |
| `build` | `@nxlv/python:build` | Build sdist/wheel |
| `lint` | `@nxlv/python:ruff-check` | Lint with ruff |
| `format` | `@nxlv/python:ruff-format` | Format with ruff |
| `test` | `@nxlv/python:run-commands` | Often `uv run pytest tests/` |
| (custom) | `@nxlv/python:run-commands` | Any script that needs the venv activated |

### Dependency Commands

```
npx nx run <project>:add --name <dep>            # external dependency
npx nx run <project>:add --name <dep> --local     # local workspace dependency
npx nx run <project>:update --name <dep>
npx nx run <project>:remove --name <dep>
npx nx run <project>:lock
npx nx run <project>:sync
```

## Workspace conventions (customize for your repo)

- Prefer **uv** as the package manager when using this plugin.
- **ruff** is a typical linter/formatter pair with `@nxlv/python:ruff-check` and `@nxlv/python:ruff-format`.
- **pytest** is commonly invoked through `@nxlv/python:run-commands` with `uv run pytest tests/`.
- Custom scripts often use `@nxlv/python:run-commands` with `"command": "uv run ..."` and `"cwd": "{projectRoot}"`.
- Builds often set `lockedVersions: true`, `bundleLocalDependencies: true`, `publish: false` for internal packages.
- Prefer Nx targets for dependency changes instead of ad hoc `pip install` / `uv add` in a terminal, so the graph and lockfile stay consistent.

## project.json Template

Canonical target block for a new Python package. Replace `<module>` with the actual module directory name.

```json
{
  "name": "<project-name>",
  "$schema": "../../../node_modules/nx/schemas/project-schema.json",
  "projectType": "application",
  "sourceRoot": "<project-root>/<module>",
  "targets": {
    "lock": {
      "executor": "@nxlv/python:lock",
      "options": { "update": false }
    },
    "sync": {
      "executor": "@nxlv/python:sync",
      "options": {}
    },
    "add": {
      "executor": "@nxlv/python:add",
      "options": {}
    },
    "update": {
      "executor": "@nxlv/python:update",
      "options": {}
    },
    "remove": {
      "executor": "@nxlv/python:remove",
      "options": {}
    },
    "build": {
      "executor": "@nxlv/python:build",
      "outputs": ["{projectRoot}/dist"],
      "options": {
        "outputPath": "{projectRoot}/dist",
        "publish": false,
        "lockedVersions": true,
        "bundleLocalDependencies": true
      },
      "cache": true
    },
    "lint": {
      "executor": "@nxlv/python:ruff-check",
      "outputs": [],
      "options": {
        "lintFilePatterns": ["<module>", "tests"]
      },
      "cache": true
    },
    "format": {
      "executor": "@nxlv/python:ruff-format",
      "outputs": [],
      "options": {
        "filePatterns": ["<module>", "tests"]
      },
      "cache": true
    },
    "test": {
      "executor": "@nxlv/python:run-commands",
      "outputs": [
        "{workspaceRoot}/reports/{projectRoot}/unittests",
        "{workspaceRoot}/coverage/{projectRoot}"
      ],
      "options": {
        "command": "uv run pytest tests/",
        "cwd": "{projectRoot}"
      },
      "cache": true
    },
    "install": {
      "executor": "@nxlv/python:install",
      "options": {
        "silent": false,
        "args": "",
        "verbose": false,
        "debug": false
      }
    }
  },
  "tags": [],
  "release": {
    "version": {
      "generator": "@nxlv/python:release-version"
    }
  }
}
```

## Sources

- [GitHub: @nxlv/python](https://github.com/lucasvieirasilva/nx-plugins/tree/main/packages/nx-python)
