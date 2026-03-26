# Backend Plugin Specification (Rust Edition)

## Scope

This document defines the normative interface contract for backend plugins in the Rust implementation.

**Covered:**
- Required script operations (`apply.sh`, `remove.sh`, `status.sh`)
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
  apply.sh         # install or upgrade
  remove.sh        # uninstall
  status.sh        # query installation state
```

All three scripts must be present and executable.

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

---

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
- Must be **idempotent**: removing an absent resource must succeed without error.
- Must only remove what this backend installed. Must NOT remove resources managed by other backends.

**Example (using environment variables):**
```bash
#!/usr/bin/env bash
set -euo pipefail
brew uninstall "$LOADOUT_PACKAGE_NAME" 2>&1 || true
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
brew uninstall "$LOADOUT_PACKAGE_NAME" 2>&1 || true
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

