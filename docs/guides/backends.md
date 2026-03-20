# Backend Development Guide

## Purpose

This guide explains how to implement a **script backend plugin** for loadout.

**For the authoritative interface contract, see [`specs/api/backend.md`](../specs/api/backend.md).**

---

## Backend Types

Loadout supports two types of backends:

### 1. Builtin Backends (Rust-native)

**Examples:** `core/brew`, `core/apt`, `core/mise`, `core/npm`, `core/uv`, `core/scoop`, `core/winget`

**Implementation:** Written in Rust, compiled into the `loadout` binary.
- Located in `crates/backends-builtin/src/<backend>.rs`
- Implement the `Backend` trait defined in `crates/backend-host/src/lib.rs`
- No JSON protocol overhead (direct function calls)
- Preferred for performance-critical or complex backends

**When to use:** Core-maintained backends that ship with loadout.

### 2. Script Backends (Community extensions)

**Examples:** Custom backends created by users or external sources

**Implementation:** Shell scripts (`.sh` for Linux/macOS, `.ps1` for Windows).
- Receive resource data via environment variables (primary protocol) and optionally via JSON stdin (see [`specs/api/backend.md`](../specs/api/backend.md))
- Located in source-specific `backends/` directories
- Built and loaded dynamically at runtime
- No `jq` dependency required (when using environment variables)

**When to use:** Community-contributed backends, project-specific package managers, custom tooling.

**This guide covers script backends.** For builtin backend development, see `crates/backend-host/src/lib.rs` and existing implementations in `crates/backends-builtin/`.

---

## Script Backend Structure

A script backend is a **directory** containing:

```
backends/<backend_name>/
├── backend.yaml     # Metadata (required)
├── apply.sh         # Install/upgrade operation (required)
├── remove.sh        # Uninstall operation (required)
└── status.sh        # Query installation state (required)
```

All three scripts (`.sh` on Linux/macOS, `.ps1` on Windows) must be present and executable.

### Backend Discovery

Backend directories are discovered from source roots:

- **core source**: `{repo}/backends/`
- **user source**: `$XDG_CONFIG_HOME/loadout/backends/` (or `$HOME/.config/loadout/backends/`)
- **external sources**: `$XDG_DATA_HOME/loadout/sources/<source_id>/backends/`

Backend IDs follow the canonical format: `<source_id>/<backend_name>`.

Examples:
- `core/brew` (builtin, Rust-native)
- `core/custom` (script backend in repo `backends/custom/`)
- `user/mypkg` (script backend in user config `backends/mypkg/`)

Bare backend names in policy files are normalized to `core/<name>`.

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

## Environment Variables Reference

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

### ❌ Do NOT read policy inside a backend

Policy is resolved by the FeatureCompiler **before** your backend is called.
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

1. Create a test feature that uses your backend:

```yaml
# features/test-mybackend/feature.yaml
spec_version: 1
mode: declarative
description: Test feature for mybackend
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

2. Create a policy that selects your backend:

```yaml
# test-policy.yaml
packages:
  testpkg: user/mybackend
```

3. Run:

```bash
loadout plan --profile test-profile.yaml --policy test-policy.yaml
loadout apply --profile test-profile.yaml --policy test-policy.yaml
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

### For user backends

Place backend directory in `$XDG_CONFIG_HOME/loadout/backends/<name>/` (Linux/macOS) or `%LOCALAPPDATA%\loadout\backends\<name>\` (Windows).

Reference in policy as `user/<name>`.

### For external source backends

1. Publish backend directory in your git repository under `backends/<name>/`
2. Users register your source via `sources.yaml`:

```yaml
sources:
  - id: mycorp
    url: https://github.com/mycorp/loadout-backends
    path: .
```

3. Reference in policy as `mycorp/<name>`

---

## See Also

- **[`specs/api/backend.md`](../specs/api/backend.md)** — Authoritative interface contract
- **[`guides/features.md`](features.md)** — How features declare resource requirements
- **`crates/backend-host/src/lib.rs`** — Backend trait and ScriptBackend implementation (for Rust developers)
- **`crates/backends-builtin/`** — Builtin backend implementations (Rust examples)
