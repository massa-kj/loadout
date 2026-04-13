# Component Script Specification (Script Mode)

## Scope

This document defines the normative interface contract for **script-mode components**.

**Covered:**
- Script naming and execution model (`install.sh`, `uninstall.sh`)
- Environment variable injection protocol
- Exit code contract and error handling
- Isolation rules and safety constraints

**Not covered:**
- Declarative-mode components (resource-based; see `docs/specs/data/profile.md`)
- How to implement components (see `docs/guides/components.md`)
- Component host internals (see `crates/component-host/src/lib.rs`)

## Overview

**Script-mode components** are arbitrary shell scripts that perform installation or configuration tasks outside the resource model (e.g., system-wide settings, file templating, conditional setup).

They receive **context via environment variables** (not JSON stdin/stdout) and must exit 0 on success.

Script-mode components **must NOT write to `state.json`**. State updates are the executor's responsibility.

## Component Directory Layout

```text
<component_source_dir>/
  component.yaml         # metadata (mode: script, dependencies, etc.)
  install.sh           # installation script
  uninstall.sh         # removal script
  files/               # optional: files to copy/template
    ...
```

- **`install.sh`** — executed by `loadout apply` (required)
- **`uninstall.sh`** — executed by `loadout prune` when the component is removed (required)

Both scripts must be present and executable for a script-mode component.

## Script Execution Protocol

### Environment Variables (Input)

The following environment variables are injected by the component host:

| Variable | Description | Example |
|---|---|---|
| `LOADOUT_COMPONENT_ID` | Canonical component ID | `core/git` |
| `LOADOUT_CONFIG_HOME` | User config directory (XDG/AppData) | `/home/user/.config/loadout` |
| `LOADOUT_DATA_HOME` | User data directory (XDG/AppData) | `/home/user/.local/share/loadout` |
| `LOADOUT_STATE_HOME` | User state directory (XDG/AppData) | `/home/user/.local/state/loadout` |

Scripts may use these to locate persistent files (e.g., `$LOADOUT_CONFIG_HOME/git/config`).

### Working Directory

The script is executed with its **component source directory** as the current working directory.

This allows scripts to reference local files:
```bash
cp files/gitconfig ~/.gitconfig
```

### Standard Streams

- **stdin:** Closed (not used)
- **stdout:** Captured for logging (informational messages)
- **stderr:** Captured for logging (error messages)

Scripts should write progress/debug output to **stdout** and errors to **stderr**.

### Exit Code Contract

- **0** — Success (component installed/uninstalled successfully)
- **Non-0** — Failure (operation failed; stderr captured for diagnostics)

## Script Operations

### `install.sh`

**Purpose:** Install or configure the component.

**Execution:** Run by `loadout apply` when the component is in the desired profile but not in state.

**Contract:**
- Must be **idempotent** if possible (though declarative idempotency is not guaranteed; scripts may be re-run).
- Must NOT modify `state.json` directly.
- Must exit 0 on success.

**Example:**
```bash
#!/usr/bin/env bash
set -euo pipefail

echo "Installing git configuration..."
cp files/gitconfig ~/.gitconfig
echo "Git configuration installed."
```

### `uninstall.sh`

**Purpose:** Remove or revert the component's changes.

**Execution:** Run by `loadout apply` when the component is removed from the desired profile.

**Contract:**
- Must be **idempotent** (safe to run even if the component is already removed).
- Must only remove changes made by this component's `install.sh` (not other components or user modifications).
- Must exit 0 on success.

**Example:**
```bash
#!/usr/bin/env bash
set -euo pipefail

echo "Uninstalling git configuration..."
rm -f ~/.gitconfig
echo "Git configuration removed."
```

## Safety and Isolation Rules

Script-mode components **must NOT**:
- Read or write `state.json` directly (state is managed by the executor)
- Interfere with other components' files (scope your changes)
- Assume specific execution order relative to other components (use `dependencies` in `component.yaml` for ordering)

Script-mode components **should**:
- Use `set -euo pipefail` to fail fast on errors
- Log progress to stdout for user visibility
- Write errors to stderr

## Component Metadata: `component.yaml`

```yaml
mode: script
dependencies:
  - core/bash
  - core/git
```

- **`mode`** (string, required): Must be `"script"` for script-mode components.
- **`dependencies`** (array of canonical component IDs, optional): Components that must be installed before this one.

See `docs/specs/data/profile.md` for the full `component.yaml` schema.

## Declarative vs. Script Mode

| Mode | Mechanism | Idempotency | Use Case |
|---|---|---|---|
| **Declarative** (default) | Resource graph → backends | Guaranteed | Packages, runtimes, files |
| **Managed Script** | Script + core verify + core state | Verified | Tools installed via install scripts |
| **Script** | Arbitrary shell script | Best-effort | System settings, templates, conditional logic |

Use **declarative mode** when possible (better error handling, atomic state, backend-agnostic).

Use **managed script mode** when:
- Tool installation requires an external script (curl-pipe installer, vendor script, single-binary setup)
- The installed tool has a stable, verifiable path

Use **script mode** when:
- The operation cannot be expressed as resources (e.g., writing to `/etc/sudoers`)
- Significant conditional logic is required
- Tool verification after install is not feasible

## Managed Script Mode

`mode: managed_script` components combine imperative install/uninstall scripts with executor-owned
verification and state management.

### What the executor does

**On install (create or replace-install):**
1. Executes `install.sh`.
2. If exit code is 0: verifies all declared `tool` resources using their `verify.identity` contract.
3. If all verifications pass: records observed facts (`resolved_path`, `version`) in state and commits atomically.
4. If verification fails: operation is a failure; state is not modified.

**On uninstall (destroy or replace-uninstall):**
1. Executes `uninstall.sh`.
2. If exit code is 0: performs absence check — `tool.observed.resolved_path` must not exist on disk.
3. If all absence checks pass: removes the component's resources from state and commits atomically.
4. If any path still exists: operation is a failure; state is not modified.

### Constraints

Scripts for `managed_script` components are subject to the same isolation rules as `script` mode:

- Must NOT read or write `state.json` directly
- Must NOT remove files belonging to other components
- Must use `set -euo pipefail` (recommended)
- Must exit 0 on success, non-0 on failure

Additionally, for `managed_script` components:

- The `resources:` section is **required** and must contain at least one `kind: tool` entry
- Each `tool` resource **must** declare `verify.identity` (identity verification is mandatory)
- `scriptsinstall` and `scripts.uninstall` keys are required in `component.yaml`
- Only `kind: tool` resources are permitted; mixing `package`, `runtime`, or `fs` is prohibited

### Example: `component.yaml`

```yaml
spec_version: 1
mode: managed_script
description: Install Homebrew

provides:
  - name: package_manager

scripts:
  install: install.sh
  uninstall: uninstall.sh

resources:
  - kind: tool
    id: tool:brew
    name: brew
    verify:
      identity:
        type: resolved_command
        command: brew
        expected_path:
          one_of:
            - /home/linuxbrew/.linuxbrew/bin/brew
            - /opt/homebrew/bin/brew
```

See `docs/guides/components.md` for the full `tool` resource reference and more examples.

## Example: Script-Mode Component

### `component.yaml`
```yaml
mode: script
dependencies:
  - core/bash
```

### `install.sh`
```bash
#!/usr/bin/env bash
set -euo pipefail

echo "Configuring system locale..."
sudo localectl set-locale LANG=en_US.UTF-8
echo "Locale configured."
```

### `uninstall.sh`
```bash
#!/usr/bin/env bash
set -euo pipefail

echo "Skipping locale uninstall (system setting, not reverted)."
```

## See Also

- **Implementation:** `crates/component-host/src/lib.rs` (`run_install`, `run_uninstall`)
- **Executor integration:** `crates/executor/src/lib.rs`
- **Usage guide:** `docs/guides/components.md`
