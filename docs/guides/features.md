# Feature Guide

## Purpose

This guide explains how to create and maintain a feature module.

For state interaction contracts, see `specs/data/state.md`.
For dependency resolution mechanics, see `specs/algorithms/resolver.md`.

## Feature Design Principles

One feature = one logical responsibility (one tool, one runtime, one configuration unit).

Features must be:
* **Independent** — no dependency on other features' internals
* **Deterministic** — same inputs produce same result
* **Reversible** — everything installed can be uninstalled
* **Minimal** — scope should not expand silently

If a feature grows too large, split it.
If behavior is needed in core, it belongs in core.

## Directory Structure

```
features/<name>/
├── feature.yaml
├── feature.<platform>.yaml   # optional: linux, wsl, windows
├── install.sh / install.ps1
├── uninstall.sh / uninstall.ps1
└── files/                 # configuration files, if any
```

No nested submodules. No cross-feature imports.

The same layout is used for all source roots:

* built-in: `{repo}/features/<name>/`
* user: config home `features/<name>/`
* external: data home `sources/<source_id>/features/<name>/`

## feature.yaml

```yaml
spec_version: 1
mode: script       # script | declarative (default: declarative; script is danger)
description: Brief description
depends:
  - git                    # explicit feature dependency
  - core/bash              # explicit cross-source dependency
requires:
  - name: package_manager  # capability-based dependency
provides:
  - name: package_manager  # capability this feature exposes
```

`spec_version` is required. Must be `1` for the current schema. FeatureCompiler aborts if absent or unknown.

`mode` determines how the feature is executed:
* `declarative` — resources are compiled by FeatureCompiler from the `resources:` section and applied by the executor without scripts. **This is the default.** Prefer declarative unless scripts are unavoidable.
* `script` — run `install.sh` / `uninstall.sh`. Must be declared explicitly. Use only when install logic cannot be expressed as resources.

The `dep` block (`depends`, `requires`, `provides`) is for **ordering only**.
No version constraints, no conditional logic, no commands.

For platform-specific deps, use `feature.linux.yaml` / `feature.wsl.yaml` / `feature.windows.yaml`.
These are merged with `feature.yaml` during Feature Index Builder execution.

`depends` normalization rules:

* bare dependency name means same-source dependency
* cross-source dependency must use an explicit canonical ID
* do not rely on source search order or fallback

**Choosing `depends` vs `requires`:**

| Situation | Use |
|---|---|
| Need a specific named feature first | `depends` |
| Need any package manager | `requires: [{name: package_manager}]` |
| Need any runtime manager | `requires: [{name: runtime_manager}]` |

## Dependency Model

`depends` — explicit feature-to-feature ordering. Use for concrete named dependencies.

`requires` / `provides` — capability-based ordering. Use when any provider suffices.
The resolver finds all profile features that `provides` the capability and injects them
as implicit dependencies. If no provider is in the profile, apply aborts.

> **Note:** `depends`, `requires`, `provides` are top-level keys in `feature.yaml`.
> The Feature Index Builder normalizes these into the `dep.*` structure used internally
> by the resolver. Developers write the flat form shown above.

## Declarative Features

A declarative feature uses `mode: declarative` and declares all its resources in the `resources:` list.
No `install.sh` / `uninstall.sh` scripts are needed — the executor handles all operations automatically.

**When to use declarative mode:**

- Only installing packages/runtimes/files
- Resources map directly to backend packages
- Feature is easy to describe as a list
- You want plan-level accuracy (noop detection, replace)

### Backend Resolution

Declarative resources (packages and runtimes) require a **backend** to perform installation.

**Builtin backends** are Rust-native implementations compiled into the `loadout` binary:
- `core/brew` (Homebrew, macOS/Linux)
- `core/apt` (APT, Debian/Ubuntu)
- `core/mise` (mise, runtime version manager)
- `core/npm` (npm packages)
- `core/uv` (Python uv)
- `core/scoop` (Scoop, Windows)
- `core/winget` (winget, Windows)

**Script backends** are community extensions implemented as shell scripts (see [`guides/backends.md`](backends.md)):
- Located in `backends/<name>/` directories
- Communicate via JSON stdin/stdout protocol
- Useful for project-specific or custom package managers

Backend selection is controlled by **strategy** files (`strategies/<platform>.yaml`).
Features declare what they need; strategy determines which backend satisfies the requirement.

For backend implementation details, see [`guides/backends.md`](backends.md) and [`specs/api/backend.md`](../specs/api/backend.md).

### Declarative feature.yaml example

```yaml
spec_version: 1
mode: declarative
description: Install git and deploy gitconfig
depends:
  - bash

resources:
  - kind: package
    id: package:git
    name: git

  - kind: fs
    id: fs:gitconfig
    source: files/.gitconfig
    path: ~/.gitconfig
    entry_type: file
    op: link
```

### Resource kinds

#### `package`

Installs a package via the resolved backend.

```yaml
- kind: package
  id: package:ripgrep     # stable identifier; must not change
  name: ripgrep           # name as known to the backend (brew, apt, scoop, etc.)
```

#### `runtime`

Installs a runtime via the resolved backend (e.g. mise).

```yaml
- kind: runtime
  id: runtime:node
  name: node
  version: "22.0.0"       # exact version or constraint
```

#### `fs`

Deploys a file or directory from the feature's `files/` directory.

```yaml
- kind: fs
  id: fs:gitconfig        # stable identifier
  source: files/.gitconfig  # relative to feature directory (optional; defaults to files/<basename(path)>)
  path: ~/.gitconfig      # absolute or ~-relative target path
  entry_type: file        # file | dir
  op: link                # link (symlink/junction) | copy
```

`source` is optional. If omitted, the executor looks for `files/<basename(path)>`.  
For example, if `path: ~/.gitconfig` and `source` is omitted, the executor uses `files/.gitconfig`.

**Note:** The deployed `path` is recorded in state, but the `source` path is not stored.
If the source file changes, currently, the planner cannot detect it. Use `loadout apply` with `--replace` to force redeployment.
See `docs/specs/data/state.md` Known Limitations.

### Platform-specific resources

Use `feature.linux.yaml` / `feature.windows.yaml` to provide a platform-specific `resources:` list.
When a non-empty `resources:` list is present in the platform file, it completely replaces the base list
(not merged). This allows full override for features that need fundamentally different packages per platform.

```yaml
# feature.linux.yaml
resources:
  - kind: package
    id: package:fd
    name: fd-find          # apt name differs from brew name
```

## Script Feature Constraints

Script features (`mode: script`) execute arbitrary shell code. To preserve system safety they are subject
to strict constraints.

The install script MUST:
* Install packages/runtimes via the abstraction layer (not by calling `brew`, `apt`, etc. directly)
* Place configuration files using `link_file` / `copy_file` abstractions
* Exit non-zero on failure

The install script MUST NOT:
* Write to state directly (`state.json`)
* Perform dependency resolution
* Detect platform manually
* Access files outside the feature directory and the target home/config paths
* Read or write files of other features

State is written by the executor after install completes.

### When to use script mode

`mode: script` must be declared **explicitly** and is treated as a danger zone — it executes
arbitrary shell code with elevated trust. Use it only when:

* Custom setup logic is required that cannot be expressed as resources
* Conditional install steps are needed
* The feature manages side effects not expressible in resource kinds

## Uninstall Rules

The uninstall script must:
* Remove only resources tracked in state
* Use state APIs to retrieve tracked resources
* Exit non-zero on failure

The uninstall script must NOT:
* Remove untracked files
* Scan `files/` to discover what to remove
* Remove parent directories (unless explicitly tracked)

## File Management Rules

Configuration files must live in `features/<name>/files/`.

Use `link_file` (symlink) or `copy_file` (copy) for placement.
File operations are tracked in state automatically via the `fs` module.

## State Interaction Rules

Features must NOT access `state.json` directly.
State registration is handled by the executor after each successful feature operation.

## Version Handling

Version is passed in via `LOADOUT_FEATURE_CONFIG_VERSION`.
Features that support versioning read this variable and use it in install logic.
Features that do not support versioning ignore it.

Record installed version in state using `state_set_runtime` (for runtimes).

See `specs/data/state.md` for the runtime resource format.

## Feature Naming Guidelines

Feature names must be:
* Lowercase
* Tool-based (name of the tool, not the purpose)
* Stable identifiers that will not need to change

Avoid:
* Version-specific names (`node18`, `python3`)
* Temporary or placeholder names
* Ambiguous category names (`tools`, `utils`)

The feature name becomes part of state identity.
Renaming a feature is a breaking change requiring state migration.
Moving a feature between sources also changes its canonical ID and is therefore a breaking change.

## Feature Evolution

When modifying an existing feature:
* Maintain uninstall compatibility with the previously recorded state
* Do not change resource identifiers (`id` fields) without a migration plan
* Do not silently expand scope — if new responsibilities are needed, consider a new feature

Features must remain loosely coupled to the surrounding system.
Backend changes, state schema evolution, and platform additions must not require feature rewrites.
This loose coupling is enforced by: using abstraction APIs rather than calling tools directly,
declaring dependencies rather than assuming presence, and letting the executor own state.
