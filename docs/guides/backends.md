# Backend Development Guide

## Purpose

This guide explains how to implement a **script backend plugin** for loadout.

**For the authoritative interface contract, see [`specs/api/backend.md`](../specs/api/backend.md).**

---

## Backend Types

Loadout supports two types of backends:

### 1. Script Backends (primary mechanism)

**Examples:** `core/brew`, `core/apt`, `core/mise`, `core/npm`, `core/uv`, `core/scoop`, `core/winget`

**Implementation:** Shell scripts (`.sh` for Linux/macOS, `.ps1` for Windows).
- Receive resource data via environment variables (primary protocol) and optionally via JSON stdin (see [`specs/api/backend.md`](../specs/api/backend.md))
- Located in source-specific `backends/` directories (e.g. `backends/brew/`, `backends/apt/`)
- Built and loaded dynamically at runtime
- No `jq` dependency required (when using environment variables)
- Can optionally provide `env_pre.sh` / `env_post.sh` for execution environment contributions

**When to use:** All production backends, including core-maintained ones that ship with loadout.

### 2. Builtin Backends (Rust-native extension point)

**Implementation:** Rust structs implementing the `Backend` trait, compiled into the `loadout` binary.
- Registered via `crates/backends-builtin/src/lib.rs` (`register_builtins`)
- Currently **intentionally empty** — all production backends use the script-backend mechanism above
- Reserved as an extension point for future OS-level integrations (e.g. Windows registry, OS API probes) or internal test mocks

**When to use:** Only when shell scripts are genuinely insufficient (OS-level API probes, Windows registry, etc.).

**This guide covers script backends.** For Rust-native backend development, see `Backend` trait in `crates/backend-host/src/lib.rs`.

---

## Script Backend Structure

A script backend is a **directory** containing:

```
backends/<backend_name>/
├── backend.yaml     # Metadata (required)
├── apply.sh         # Install/upgrade operation (required)
├── remove.sh        # Uninstall operation (required)
├── status.sh        # Query installation state (required)
├── env_pre.sh       # Pre-action env delta (optional)
└── env_post.sh      # Post-action env delta (optional)
```

The three core scripts (`.sh` on Linux/macOS, `.ps1` on Windows) must be present and executable.

`env_pre.sh` and `env_post.sh` are **optional**. Add them only when your backend needs to
contribute environment variables to the executor session:
- **`env_pre.sh`** — runs before `apply.sh` (e.g., ensure `brew` is on PATH before invoking brew-backed packages)
- **`env_post.sh`** — runs after a successful `apply.sh` (e.g., expose newly-installed tool shims)

### Backend Discovery

Backend directories are discovered from source roots:

- **core source**: `{repo}/backends/`
- **local source**: `$XDG_CONFIG_HOME/loadout/backends/` (or `$HOME/.config/loadout/backends/`)
- **external sources**: `$XDG_DATA_HOME/loadout/sources/<source_id>/backends/`

Backend IDs follow the canonical format: `<source_id>/<backend_name>`.

Examples:
- `core/brew` (script backend in repo `backends/brew/`)
- `core/custom` (script backend in repo `backends/custom/`)
- `local/mypkg` (script backend in user config `backends/mypkg/`)

Bare backend names in strategy files are normalized to `core/<name>`.

---

## Implementing a Script Backend

### 1. Create `backend.yaml`

```yaml
api_version: 1
```

- **`api_version`** (required): Must be `1`. Breaking changes to the script protocol will increment this.

### 2. Implement `apply.sh`

**Purpose:** Install or upgrade a resource.

**Primary method: Environment variables**

```bash
#!/usr/bin/env bash
set -euo pipefail

# Resource data is available via environment variables
case "$LOADOUT_RESOURCE_KIND" in
    Package)
        mytool install "$LOADOUT_PACKAGE_NAME"
        ;;
    Runtime)
        runtimetool install "$LOADOUT_RUNTIME_NAME@$LOADOUT_RUNTIME_VERSION"
        ;;
    *)
        echo "Unsupported kind: $LOADOUT_RESOURCE_KIND" >&2
        exit 1
        ;;
esac
```

**Alternative method: JSON stdin (for complex cases)**

If you need structured data parsing with tools like `jq`:

```bash
#!/usr/bin/env bash
set -euo pipefail

# Optionally read JSON from stdin
resource=$(cat)
kind=$(echo "$resource" | jq -r '.kind')

case "$kind" in
    package)
        name=$(echo "$resource" | jq -r '.Package.name')
        mytool install "$name"
        ;;
    runtime)
        name=$(echo "$resource" | jq -r '.Runtime.name')
        version=$(echo "$resource" | jq -r '.Runtime.version')
        runtimetool install "$name@$version"
        ;;
esac
```

**Contract:**
- **Input:** Environment variables (`LOADOUT_RESOURCE_KIND`, `LOADOUT_PACKAGE_NAME`, etc.), and optionally JSON on stdin
- **Output:** None (stderr for logging only)
- **Exit code:** 0 = success, non-0 = failure
- **Idempotency:** Must succeed if resource is already at desired state

**Recommendation:** Use environment variables for simplicity. They eliminate the `jq` dependency and make scripts easier to read.

### 3. Implement `remove.sh`

**Purpose:** Uninstall a resource.

```bash
#!/usr/bin/env bash
set -euo pipefail

case "$LOADOUT_RESOURCE_KIND" in
    Package)
        mytool uninstall "$LOADOUT_PACKAGE_NAME" || true
        ;;
    Runtime)
        runtimetool uninstall "$LOADOUT_RUNTIME_NAME@$LOADOUT_RUNTIME_VERSION" || true
        ;;
    *)
        echo "Unsupported kind: $LOADOUT_RESOURCE_KIND" >&2
        exit 1
        ;;
esac
```

**Contract:**
- **Input:** Environment variables (primary), and optionally JSON on stdin
- **Output:** None (stderr for logging only)
- **Exit code:** 0 = success, non-0 = failure
- **Idempotency:** Must succeed if resource is already absent

### 4. Implement `status.sh`

**Purpose:** Query current installation state of a resource.

```bash
#!/usr/bin/env bash
set -euo pipefail

case "$LOADOUT_RESOURCE_KIND" in
    Package)
        if mytool list | grep -q "^$LOADOUT_PACKAGE_NAME\$"; then
            echo "installed"
        else
            echo "not_installed"
        fi
        ;;
    Runtime)
        if runtimetool list | grep -q "^$LOADOUT_RUNTIME_NAME@$LOADOUT_RUNTIME_VERSION\$"; then
            echo "installed"
        else
            echo "not_installed"
        fi
        ;;
    *)
        echo "unknown" # Unsupported kind
        ;;
esac
```

**Contract:**
- **Input:** Environment variables (primary), and optionally JSON on stdin
- **Output:** One of these exact strings on stdout (case-sensitive):
  - `installed` — Resource is present
  - `not_installed` — Resource is absent
  - `unknown` — Cannot determine (e.g., backend tool not available)
- **Exit code:** 0 = query succeeded, non-0 = query failed (treated as `unknown`)

---

## Implementing Env Lifecycle Scripts (Optional)

If your backend installs a tool that subsequent backends depend on (e.g., `brew`, `mise`),
you can add `env_pre.sh` and/or `env_post.sh` to contribute environment mutations to the
executor's running session.

### 5. Implement `env_pre.sh` (optional)

**Purpose:** Declare env mutations needed **before** `apply.sh` runs.

The executor calls this script first, merges the returned delta into the session env, then
exports updated variables to `apply.sh`'s subprocess. This allows `apply.sh` to use tools
that are already on PATH.

**Contract:**
- **Input:** Same environment variables as `apply.sh` (no JSON stdin)
- **Output:** JSON env delta payload on stdout, OR empty stdout (valid no-op)
- **Exit code:** 0 = success, non-0 = failure (non-fatal; executor warns and continues)
- **No external tools required:** construct JSON with bash heredoc (`no jq needed`)

**Example: brew backend prepending its bin directory**

```bash
#!/usr/bin/env bash
# backends/brew/env_pre.sh
set -euo pipefail

BREW_PREFIX="$(brew --prefix 2>/dev/null || true)"
if [[ -z "$BREW_PREFIX" ]]; then
    # brew not installed yet, or not found; nothing to contribute
    exit 0
fi

cat <<JSON
{
  "schema_version": 1,
  "mutations": [
    { "op": "prepend_path", "key": "PATH", "entries": ["${BREW_PREFIX}/bin", "${BREW_PREFIX}/sbin"] }
  ],
  "evidence": { "kind": "probed", "command": "brew --prefix" }
}
JSON
```

### 6. Implement `env_post.sh` (optional)

**Purpose:** Declare env mutations that become available **after** a successful `apply.sh`.

Use this when installing a tool via `apply.sh` reveals new paths (e.g., a runtime manager
installing shims that subsequent backends need).

**Example: mise backend adding shims after runtime install**

```bash
#!/usr/bin/env bash
# backends/mise/env_post.sh
set -euo pipefail

MISE_SHIMS="$(mise shims --dirs 2>/dev/null | head -n1 || true)"
if [[ -z "$MISE_SHIMS" ]]; then
    exit 0
fi

cat <<JSON
{
  "schema_version": 1,
  "mutations": [
    { "op": "prepend_path", "key": "PATH", "entries": ["${MISE_SHIMS}"] }
  ],
  "evidence": { "kind": "probed", "command": "mise shims --dirs" }
}
JSON
```

### Wire Format Reference

See [`specs/api/backend.md` — Env Lifecycle Scripts](../specs/api/backend.md) for the full JSON wire format
documentation, including all supported `op` values and `evidence` kinds.

Scripts receive resource data via environment variables:

### Common Variables (All Resource Kinds)

- **`LOADOUT_RESOURCE_ID`** — Unique resource identifier (e.g., `package:git`)
- **`LOADOUT_RESOURCE_KIND`** — Resource kind: `Package`, `Runtime`, or `Fs`
- **`LOADOUT_BACKEND_ID`** — Canonical backend ID (e.g., `core/brew`)

### Package Resources

When `LOADOUT_RESOURCE_KIND=Package`:

- **`LOADOUT_PACKAGE_NAME`** — Package name (e.g., `git`, `neovim`)

### Runtime Resources

When `LOADOUT_RESOURCE_KIND=Runtime`:

- **`LOADOUT_RUNTIME_NAME`** — Runtime name (e.g., `node`, `python`)
- **`LOADOUT_RUNTIME_VERSION`** — Version string (e.g., `20`, `3.12`)

### Fs Resources

When `LOADOUT_RESOURCE_KIND=Fs`:

- **`LOADOUT_FS_PATH`** — Destination path
- **`LOADOUT_FS_SOURCE`** — Source path (optional)
- **`LOADOUT_FS_ENTRY_TYPE`** — Entry type (e.g., `File`, `Directory`)
- **`LOADOUT_FS_OP`** — Operation (e.g., `Copy`, `Symlink`)

### Optional: JSON Stdin

Scripts **also** receive the full resource definition as JSON on stdin (for complex parsing cases).

See [`specs/api/backend.md`](../specs/api/backend.md) for complete protocol documentation.

---

## Common Pitfalls

### ❌ Do NOT read strategy inside a backend

Strategy is resolved by the ComponentCompiler **before** your backend is called.
The resource data you receive via environment variables already reflects the resolved `desired_backend`.

### ❌ Do NOT write state inside a backend

State is written by the Executor **after** your script exits successfully (exit 0).

### ❌ Do NOT assume resource versions are pinned

For `Package` resources, version is not provided (packages are typically unversioned).

For `Runtime` resources, `LOADOUT_RUNTIME_VERSION` is always present but may be a constraint (e.g., "20", "20.x", ">=18").

### ✅ DO make operations idempotent

- **`apply.sh`**: If the resource is already installed at the desired state, return 0 (success).
- **`remove.sh`**: If the resource is already absent, return 0 (success).

This allows `loadout apply` to be run repeatedly without errors.

### ✅ DO use stderr for logging

- **stdout** is reserved for `status.sh` output (`installed` / `not_installed` / `unknown`)
- **stderr** is captured by loadout and shown to the user for diagnostics

Example:
```bash
echo "Installing package: $LOADOUT_PACKAGE_NAME" >&2  # Correct
echo "installed"                                       # Correct (status.sh only)
```

### ✅ DO exit with appropriate codes

- **0** = Success (operation completed or already at desired state)
- **Non-0** = Failure (operation failed, loadout will abort and report error)

---

## Testing a Backend

### Manual testing

1. Create a test component that uses your backend:

```yaml
# components/test-mybackend/component.yaml
spec_version: 1
mode: declarative
description: Test component for mybackend
dep:
  depends: []
  requires: []
  provides: []
spec:
  resources:
    - id: package:testpkg
      kind: package
      name: testpkg
```

2. Create a strategy that selects your backend:

```yaml
# test-strategy.yaml
packages:
  testpkg: local/mybackend
```

3. Run:

```bash
loadout plan --profile test-profile.yaml --strategy test-strategy.yaml
loadout apply --profile test-profile.yaml --strategy test-strategy.yaml
```

4. Verify:
- `apply.sh` is called with correct environment variables set
- Resource is installed successfully
- `status.sh` returns `installed`
- `remove.sh` uninstalls cleanly

### Integration testing

End-to-end tests are located in `tests/` (Rust-based). Add a test case that exercises your backend.

---

## Platform-Specific Backends

### Linux/macOS: `.sh` scripts

Use POSIX-compatible shell (`#!/usr/bin/env bash` or `#!/bin/sh`).

**Recommended approach:** Use environment variables (no external dependencies).

```bash
#!/usr/bin/env bash
set -euo pipefail
mytool install "$LOADOUT_PACKAGE_NAME"
```

**Alternative:** Parse JSON stdin for complex cases (requires `jq`, `python3`, or `node`).

```bash
#!/usr/bin/env bash
resource=$(cat)
name=$(echo "$resource" | jq -r '.Package.name')
mytool install "$name"
```

### Windows: `.ps1` scripts

Use PowerShell 5.1+ compatible syntax.

**Recommended approach:** Use environment variables.

```powershell
mytool install $env:LOADOUT_PACKAGE_NAME
```

**Alternative:** Parse JSON stdin.

```powershell
$resource = $input | ConvertFrom-Json
$name = $resource.Package.name
mytool install $name
```

**Note:** Windows backend directories must contain `.ps1` versions of all scripts:
- `apply.ps1`
- `remove.ps1`
- `status.ps1`

---

## Publishing a Backend

### For local backends

Place backend directory in `$XDG_CONFIG_HOME/loadout/backends/<name>/` (Linux/macOS) or `%LOCALAPPDATA%\loadout\backends\<name>\` (Windows).

Reference in strategy as `local/<name>`.

### For external source backends

1. Publish backend directory in your git repository under `backends/<name>/`
2. Users register your source via `sources.yaml`:

```yaml
sources:
  - id: mycorp
    url: https://github.com/mycorp/loadout-backends
    path: .
```

3. Reference in strategy as `mycorp/<name>`

---

## See Also

- **[`specs/api/backend.md`](../specs/api/backend.md)** — Authoritative interface contract
- **[`guides/components.md`](components.md)** — How components declare resource requirements
- **`crates/backend-host/src/lib.rs`** — Backend trait and ScriptBackend implementation (for Rust developers)
- **`crates/backends-builtin/`** — Builtin backend implementations (Rust examples)
