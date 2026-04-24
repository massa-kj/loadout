# Backend Plugin Specification (Rust Edition)

## Scope

This document defines the normative interface contract for backend plugins in the Rust implementation.

**Covered:**
- Required script operations (`apply.sh`, `remove.sh`, `status.sh`)
- Optional env lifecycle scripts (`env_pre.sh`, `env_post.sh`)
- Environment variable and JSON stdin protocol
- Plugin directory layout and metadata
- Determinism requirements and isolation rules

**Not covered:**
- How to implement a backend (see `guides/backends.md`)
- Backend trait internals (see `crates/backend-host/src/lib.rs`)

## Overview

Backend plugins are **shell script directories** that implement resource management operations (install/upgrade/uninstall packages, runtimes, etc.).

The Rust `ScriptBackend` loads these directories and invokes the scripts, passing **resource data via environment variables** (primary protocol) and **optionally as JSON on stdin** (for complex parsing).

Builtin backends (like `brew`, `apt`, `mise`) are implemented directly in Rust via the `Backend` trait and ship with the `loadout` binary. Script backends can extend or override these builtins.

---

## Plugin Directory Layout

```text
<backend_dir>/
  backend.yaml     # metadata (api_version)
  apply.sh         # install or upgrade         (required)
  remove.sh        # uninstall                  (required)
  status.sh        # query installation state   (required)
  env_pre.sh       # pre-action env delta        (optional, api_version 1+)
  env_post.sh      # post-action env delta       (optional, api_version 1+)
```

`apply.sh`, `remove.sh`, `status.sh` must be present and executable.
`env_pre.sh` and `env_post.sh` are **optional**. Backends without them continue to work unchanged.

---

## Metadata: `backend.yaml`

```yaml
api_version: 1
```

- **`api_version`** (integer, required): Must be `1`.
  Breaking changes to the script interface will increment this version.

Script backends with an unsupported `api_version` will be rejected at load time.

---

## Protocol: Environment Variables + JSON

### Primary Method: Environment Variables

All three scripts receive **resource data via environment variables**.

Scripts can access fields directly using shell variable expansion (no external tools required):

```bash
#!/usr/bin/env bash
brew install "$LOADOUT_PACKAGE_NAME"
```

#### Common Variables (All Resource Kinds)

- **`LOADOUT_RESOURCE_ID`** — Unique resource identifier (e.g., `package:git`)
- **`LOADOUT_RESOURCE_KIND`** — Resource kind: `Package`, `Runtime`, or `Fs`
- **`LOADOUT_BACKEND_ID`** — Canonical backend ID (e.g., `core/brew`)

#### Package Resources

When `LOADOUT_RESOURCE_KIND=Package`:

- **`LOADOUT_PACKAGE_NAME`** — Package name (e.g., `git`, `neovim`)
- **`LOADOUT_PACKAGE_VERSION`** — Version string (e.g., `3.12`). **Only set when a version is declared in the component.** Absent (not set) when the package is unversioned. Backend scripts must check before using (e.g., `if [ -n "${LOADOUT_PACKAGE_VERSION:-}" ]`).

#### Runtime Resources

When `LOADOUT_RESOURCE_KIND=Runtime`:

- **`LOADOUT_RUNTIME_NAME`** — Runtime name (e.g., `node`, `python`)
- **`LOADOUT_RUNTIME_VERSION`** — Version string (e.g., `20`, `3.12`)

#### Fs Resources

When `LOADOUT_RESOURCE_KIND=Fs`:

- **`LOADOUT_FS_PATH`** — Destination path
- **`LOADOUT_FS_SOURCE`** — Source path (optional)
- **`LOADOUT_FS_ENTRY_TYPE`** — Entry type (e.g., `File`, `Directory`)
- **`LOADOUT_FS_OP`** — Operation (e.g., `Copy`, `Symlink`)

### Optional Method: JSON Stdin

Scripts **also** receive the full resource definition as JSON on stdin.

This is provided for complex parsing cases where structured data is preferred.

**Most scripts do NOT need to use JSON.** Environment variables are simpler and eliminate the `jq` dependency.

#### JSON Schema Example (Package)

```json
{
  "id": "package:bash",
  "kind": "package",
  "Package": {
    "name": "bash",
    "desired_backend": "core/brew"
  }
}
```

#### JSON Schema Example (Runtime)

```json
{
  "id": "runtime:node@20",
  "kind": "runtime",
  "Runtime": {
    "name": "node",
    "version": "20",
    "desired_backend": "core/mise"
  }
}
```

Scripts that need JSON can parse stdin with `jq` or other tools:

```bash
name=$(jq -r '.Package.name' <&0)
```

## Env Lifecycle Scripts (Optional)

`env_pre.sh` and `env_post.sh` allow a backend plugin to **declare execution
environment mutations** that the executor applies before and after calling
`apply.sh`. This enables downstream backends to use tools that were just
installed (e.g. brew PATH before calling a brew-backed backend, mise shims
after installing a runtime).

### When to use

| Script | Use case |
|---|---|
| `env_pre.sh` | Backend's own tool needs to be on PATH before `apply.sh` runs (e.g., `brew`, `mise` shims) |
| `env_post.sh` | After successful apply, expose newly-installed tools to subsequent backends (e.g., `mise` shims after runtime install) |

### Protocol

**Input:**
- Same environment variables as `apply.sh` (`LOADOUT_RESOURCE_KIND`, `LOADOUT_PACKAGE_NAME`, etc.)
- **No JSON on stdin** (unlike `apply.sh`, no stdin is provided)

**Output:**
- A JSON **env delta payload** on **stdout** (see wire format below)
- Empty stdout is a valid no-op (treated as "no env changes")
- **stderr** is forwarded to the user (diagnostic messages)

**Exit code:**
- **0** — Success (output may be empty)
- **Non-0** — Failure → executor emits a `ContributorWarning` and continues (non-fatal)

### Wire Format

```json
{
  "schema_version": 1,
  "mutations": [
    { "op": "prepend_path", "key": "PATH", "entries": ["/home/linuxbrew/.linuxbrew/bin"] },
    { "op": "set",          "key": "HOMEBREW_NO_AUTO_UPDATE", "value": "1" }
  ],
  "evidence": {
    "kind": "probed",
    "command": "brew --prefix"
  }
}
```

#### `mutations` — supported `op` values

| `op` | Fields | Description |
|---|---|---|
| `set` | `key`, `value` | Set a variable to an exact value |
| `unset` | `key` | Remove a variable |
| `prepend_path` | `key`, `entries: []` | Prepend entries to a PATH-style variable |
| `append_path` | `key`, `entries: []` | Append entries to a PATH-style variable |
| `remove_path` | `key`, `entries: []` | Remove specific entries from a PATH-style variable |

#### `evidence` — how the value was obtained

| `kind` | Additional fields | Meaning |
|---|---|---|
| `static_default` | _(none)_ | Hardcoded / well-known default path |
| `probed` | `command` | Value obtained by running a command (e.g. `brew --prefix`) |
| `config_file` | `path` | Value read from a configuration file |

`evidence` is optional. If omitted, `kind` defaults to `static_default`.

### Implementation Example (pure bash, no jq)

```bash
#!/usr/bin/env bash
# backends/brew/env_pre.sh
set -euo pipefail

BREW_PREFIX="$(brew --prefix 2>/dev/null || true)"
if [[ -z "$BREW_PREFIX" ]]; then
    exit 0  # brew not found — no env contribution
fi

cat <<JSON
{
  "schema_version": 1,
  "mutations": [
    { "op": "prepend_path", "key": "PATH", "entries": ["${BREW_PREFIX}/bin"] }
  ],
  "evidence": { "kind": "probed", "command": "brew --prefix" }
}
JSON
```

**Requirements:**
- No external tools (`jq` is NOT required).
  Use bash heredoc or `printf` / `echo` to produce JSON. All values must be known strings with no special characters, which makes raw string construction safe.
- Exit 0 with empty stdout if the tool is not installed (valid no-op).

### Error Handling

Failures in `env_pre.sh` / `env_post.sh` are **non-fatal**. The executor:
1. Emits a `ContributorWarning` event with the backend ID and reason.
2. Continues with the next action (does not abort the apply session).

This matches the `is_required() = false` behaviour used for optional contributors.

## Script Operations

### `apply.sh`

**Purpose:** Install or upgrade the resource.

**Input:** 
- Environment variables (primary: `LOADOUT_RESOURCE_ID`, `LOADOUT_PACKAGE_NAME`, etc.)
- JSON on stdin (optional)

**Output:** None (stderr for logging)

**Exit code:**
- **0** — Success (resource installed or already at desired state)
- **Non-0** — Failure (installation failed; stderr captured for logging)

**Contract:**
- Must be **idempotent**: applying the same resource multiple times must succeed without error.
- Must NOT uninstall other resources or modify unrelated state.

**Example (using environment variables):**
```bash
#!/usr/bin/env bash
set -euo pipefail
brew install "$LOADOUT_PACKAGE_NAME"
```

---

### `remove.sh`

**Purpose:** Uninstall the resource.

**Input:**
- Environment variables (primary)
- JSON on stdin (optional)

**Output:** None (stderr for logging)

**Exit code:**
- **0** — Success (resource removed or already absent)
- **Non-0** — Failure (removal failed; stderr captured for logging)

**Contract:**
- Must be **idempotent**: removing an absent resource must succeed without error. Implement idempotency via a **pre-check** (`if ! tool list ...; then exit 0; fi`), not via `|| true` after the uninstall command. Using `|| true` swallows genuine errors (e.g. dependency conflicts) and causes the executor to clear state even when removal failed.
- Must only remove what this backend installed. Must NOT remove resources managed by other backends.

**Example (using environment variables):**
```bash
#!/usr/bin/env bash
set -euo pipefail
if ! brew list --formula "$LOADOUT_PACKAGE_NAME" &>/dev/null; then
    echo "Package not installed, skipping" >&2
    exit 0
fi
brew uninstall "$LOADOUT_PACKAGE_NAME"
```

---

### `status.sh`

**Purpose:** Query the current installation state of the resource.

**Input:**
- Environment variables (primary)
- JSON on stdin (optional)

**Output:** One of these **exact strings** on stdout (case-sensitive):
- `installed` — Resource is present and correctly installed
- `not_installed` — Resource is absent
- `unknown` — Unable to determine (e.g., backend tool not available)

**Exit code:**
- **0** — Status query succeeded (output must be one of the above)
- **Non-0** — Query failed (treated as `unknown`)

**Example (using environment variables):**
```bash
#!/usr/bin/env bash
set -euo pipefail
if brew list --formula "$LOADOUT_PACKAGE_NAME" &>/dev/null; then
    echo "installed"
else
    echo "not_installed"
fi
```

**Contract:**
- Must be **read-only** (no side effects).
- Must complete quickly (used by `plan` command to diff against desired state).

---

## Isolation Rules

Backend plugins **must NOT**:
- Read `state.json` or strategy files directly
- Communicate with other backend plugins
- Produce side effects outside their declared resource scope
- Contain orchestration logic or dependency resolution

State and strategy are managed by the executor; backends operate only on individual resources.

---

## Determinism Requirements

Given the same resource definition (same `name`/`version`), a backend must:
- Attempt the same operation
- Not branch on undeclared environment state

Non-determinism (e.g., race conditions, external network failures) may cause transient errors, but the backend's **intent** must be deterministic.

---

## Compatibility and Versioning

- **Current API version:** `1`
- Breaking changes to the JSON schema or script contract will increment `api_version`.
- New optional fields may be added to the resource JSON without a version bump (scripts must ignore unknown fields).
- `env_pre.sh` and `env_post.sh` were added in `api_version 1` as optional extensions;
  existing backends without these files continue to work unchanged.

Core will reject backends with an unsupported `api_version` at load time.

---

## Example: Minimal `brew` Backend

### `backend.yaml`
```yaml
api_version: 1
```

### `apply.sh`
```bash
#!/usr/bin/env bash
set -euo pipefail
brew install "$LOADOUT_PACKAGE_NAME"
```

### `remove.sh`
```bash
#!/usr/bin/env bash
set -euo pipefail
if ! brew list --formula "$LOADOUT_PACKAGE_NAME" &>/dev/null; then
    echo "Package not installed, skipping" >&2
    exit 0
fi
brew uninstall "$LOADOUT_PACKAGE_NAME"
```

### `status.sh`
```bash
#!/usr/bin/env bash
set -euo pipefail
if brew list --formula "$LOADOUT_PACKAGE_NAME" &>/dev/null; then
    echo "installed"
else
    echo "not_installed"
fi
```

### Notes

- **No `jq` required**: Scripts access resource data via environment variables.
- **JSON still available**: For complex cases, scripts can parse stdin with `jq` or other tools.
- **Idempotency**: `apply.sh` and `remove.sh` must succeed when run multiple times.

---

## See Also

- **Implementation:** `crates/backend-host/src/lib.rs` (`ScriptBackend`, `Backend` trait)
- **Builtin backends:** `crates/backends-builtin/src/`
- **Usage guide:** `docs/guides/backends.md`

