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
<feature_source_dir>/
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
| **Script** | Arbitrary shell script | Best-effort | System settings, templates, conditional logic |

Use **declarative mode** when possible (better error handling, atomic state, backend-agnostic).

Use **script mode** when:
- The operation cannot be expressed as resources (e.g., writing to `/etc/sudoers`)
- Significant conditional logic is required

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
