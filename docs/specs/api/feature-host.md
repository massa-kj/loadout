# Feature Script Specification (Script Mode)

## Scope

This document defines the normative interface contract for **script-mode features**.

**Covered:**
- Script naming and execution model (`install.sh`, `uninstall.sh`)
- Environment variable injection protocol
- Exit code contract and error handling
- Isolation rules and safety constraints

**Not covered:**
- Declarative-mode features (resource-based; see `docs/specs/data/profile.md`)
- How to implement features (see `docs/guides/features.md`)
- Feature host internals (see `crates/feature-host/src/lib.rs`)

## Overview

**Script-mode features** are arbitrary shell scripts that perform installation or configuration tasks outside the resource model (e.g., system-wide settings, file templating, conditional setup).

They receive **context via environment variables** (not JSON stdin/stdout) and must exit 0 on success.

Script-mode features **must NOT write to `state.json`**. State updates are the executor's responsibility.

## Feature Directory Layout

```text
<feature_source_dir>/
  feature.yaml         # metadata (mode: script, dependencies, etc.)
  install.sh           # installation script
  uninstall.sh         # removal script
  files/               # optional: files to copy/template
    ...
```

- **`install.sh`** — executed by `loadout apply` (required)
- **`uninstall.sh`** — executed by `loadout prune` when the feature is removed (required)

Both scripts must be present and executable for a script-mode feature.

## Script Execution Protocol

### Environment Variables (Input)

The following environment variables are injected by the feature host:

| Variable | Description | Example |
|---|---|---|
| `LOADOUT_FEATURE_ID` | Canonical feature ID | `core/git` |
| `LOADOUT_CONFIG_HOME` | User config directory (XDG/AppData) | `/home/user/.config/loadout` |
| `LOADOUT_DATA_HOME` | User data directory (XDG/AppData) | `/home/user/.local/share/loadout` |
| `LOADOUT_STATE_HOME` | User state directory (XDG/AppData) | `/home/user/.local/state/loadout` |

Scripts may use these to locate persistent files (e.g., `$LOADOUT_CONFIG_HOME/git/config`).

### Working Directory

The script is executed with its **feature source directory** as the current working directory.

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

- **0** — Success (feature installed/uninstalled successfully)
- **Non-0** — Failure (operation failed; stderr captured for diagnostics)

## Script Operations

### `install.sh`

**Purpose:** Install or configure the feature.

**Execution:** Run by `loadout apply` when the feature is in the desired profile but not in state.

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

**Purpose:** Remove or revert the feature's changes.

**Execution:** Run by `loadout apply` when the feature is removed from the desired profile.

**Contract:**
- Must be **idempotent** (safe to run even if the feature is already removed).
- Must only remove changes made by this feature's `install.sh` (not other features or user modifications).
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

Script-mode features **must NOT**:
- Read or write `state.json` directly (state is managed by the executor)
- Interfere with other features' files (scope your changes)
- Assume specific execution order relative to other features (use `dependencies` in `feature.yaml` for ordering)

Script-mode features **should**:
- Use `set -euo pipefail` to fail fast on errors
- Log progress to stdout for user visibility
- Write errors to stderr

## Feature Metadata: `feature.yaml`

```yaml
mode: script
dependencies:
  - core/bash
  - core/git
```

- **`mode`** (string, required): Must be `"script"` for script-mode features.
- **`dependencies`** (array of canonical feature IDs, optional): Features that must be installed before this one.

See `docs/specs/data/profile.md` for the full `feature.yaml` schema.

## Declarative vs. Script Mode

| Mode | Mechanism | Idempotency | Use Case |
|---|---|---|---|
| **Declarative** (default) | Resource graph → backends | Guaranteed | Packages, runtimes, files |
| **Script** | Arbitrary shell script | Best-effort | System settings, templates, conditional logic |

Use **declarative mode** when possible (better error handling, atomic state, backend-agnostic).

Use **script mode** when:
- The operation cannot be expressed as resources (e.g., writing to `/etc/sudoers`)
- Significant conditional logic is required

## Example: Script-Mode Feature

### `feature.yaml`
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

- **Implementation:** `crates/feature-host/src/lib.rs` (`run_install`, `run_uninstall`)
- **Executor integration:** `crates/executor/src/lib.rs`
- **Usage guide:** `docs/guides/features.md`
