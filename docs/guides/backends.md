# Backend Development Guide

## Purpose

This guide explains how to implement a backend plugin.

For the interface contract you must satisfy, see `specs/api/backend.md`.

## Backend Structure

A backend is a script located at `backends/<name>.sh` (Linux/WSL)
or `backends/<name>.ps1` (Windows) within a source root.

Core loads backends by sourcing the file. The script must not produce side effects on load.

```
backends/
└── brew.sh       # Example: Homebrew backend
└── mise.sh       # Example: mise backend
```

Backend roots are source-specific:

* built-in: `{repo}/backends/`
* user: config home `backends/`
* external: data home `sources/<source_id>/backends/`

Backend IDs follow the same canonical format as features: `<source_id>/<name>`.
Bare backend names in policy are normalized to `core/<name>`.

## Implementing a Backend

Implement the required functions from `specs/api/backend.md`.

Minimal example:

```bash
backend_api_version() { echo "1"; }
backend_capabilities() { echo "package"; }

backend_package_install() {
    local name="$1"
    local version="${2:-}"
    # install $name using your tool
}

backend_package_uninstall() {
    local name="$1"
    # uninstall $name
}
```

Include the observation API if the backend supports drift detection during `loadout plan`:

```bash
backend_manager_exists() {
    command -v mytool >/dev/null 2>&1
}

backend_package_exists() {
    local name="$1"
    # return 0 if installed, 1 otherwise
}
```

## Backend Capabilities

Declare what your backend supports via `backend_capabilities`:

| Value | Required implementations |
|---|---|
| `package` | `backend_package_install`, `backend_package_uninstall`, `backend_package_exists` |
| `runtime` | `backend_runtime_install`, `backend_runtime_uninstall`, `backend_runtime_exists` |

A backend may support both.

## Testing a Backend

{TODO}

## Common Pitfalls

**Do not read policy inside a backend.**
Policy is resolved by core before dispatch. Your backend receives only the name and version.

**Do not write state inside a backend.**
State is written by the executor after your function returns successfully.

**Make install idempotent.**
If the package is already installed, the install function must succeed without error.

**Use `log_error` / `log_info` from core logger.**
Do not write to stdout for diagnostic messages; stdout is reserved for structured output.
