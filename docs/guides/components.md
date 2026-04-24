# Component Guide

## Purpose

This guide explains how to create and maintain a component module.

For state interaction contracts, see `specs/data/state.md`.
For dependency resolution mechanics, see `specs/algorithms/resolver.md`.

## Component Design Principles

One component = one logical responsibility (one tool, one runtime, one configuration unit).

Components must be:
* **Independent** — no dependency on other components' internals
* **Deterministic** — same inputs produce same result
* **Reversible** — everything installed can be uninstalled
* **Minimal** — scope should not expand silently

If a component grows too large, split it.
If behavior is needed in core, it belongs in core.

## Directory Structure

```
components/<name>/
├── component.yaml
├── component.<platform>.yaml   # optional: linux, wsl, windows
├── install.sh / install.ps1
├── uninstall.sh / uninstall.ps1
└── files/                 # configuration files, if any
```

No nested submodules. No cross-component imports.

The same layout is used for all source roots:

* built-in: `{repo}/components/<name>/`
* local: config home `components/<name>/`
* external: data home `sources/<source_id>/components/<name>/`

## component.yaml

```yaml
spec_version: 1
mode: script       # script | declarative (default: declarative; script is danger)
description: Brief description
depends:
  - git                    # explicit component dependency
  - core/bash              # explicit cross-source dependency
requires:
  - name: package_manager  # capability-based dependency
provides:
  - name: package_manager  # capability this component exposes
```

`spec_version` is required. Must be `1` for the current schema. ComponentCompiler aborts if absent or unknown.

`mode` determines how the component is executed:
* `declarative` — resources are compiled by ComponentCompiler from the `resources:` section and applied by the executor without scripts. **This is the default.** Prefer declarative unless scripts are unavoidable.
* `managed_script` — run `install.sh` / `uninstall.sh`, but resource verification and state updates are owned by the executor. Requires a `resources:` section with at least one `tool` resource. Safer than `script`, but still **danger** because arbitrary script execution remains.
* `script` — run `install.sh` / `uninstall.sh`. Must be declared explicitly. Use only when install logic cannot be expressed as resources and tool verification is not feasible.

The `dep` block (`depends`, `requires`, `provides`) is for **ordering only**.
No version constraints, no conditional logic, no commands.

For platform-specific deps, use `component.linux.yaml` / `component.wsl.yaml` / `component.windows.yaml`.
These are merged with `component.yaml` during Component Index Builder execution.

`depends` normalization rules:

* bare dependency name means same-source dependency
* cross-source dependency must use an explicit canonical ID
* do not rely on source search order or fallback

**Choosing `depends` vs `requires`:**

| Situation | Use |
|---|---|
| Need a specific named component first | `depends` |
| Need any package manager | `requires: [{name: package_manager}]` |
| Need any runtime manager | `requires: [{name: runtime_manager}]` |

## Dependency Model

`depends` — explicit component-to-component ordering. Use for concrete named dependencies.

`requires` / `provides` — capability-based ordering. Use when any provider suffices.
The resolver finds all profile components that `provides` the capability and injects them
as implicit dependencies. If no provider is in the profile, apply aborts.

> **Note:** `depends`, `requires`, `provides` are top-level keys in `component.yaml`.
> The Component Index Builder normalizes these into the `dep.*` structure used internally
> by the resolver. Developers write the flat form shown above.

**Choosing the right mode:**

| Mode | Use when |
|---|---|
| `declarative` | All resources are packages, runtimes, or files — first choice |
| `managed_script` | Tool must be installed via a script, but its installed location can be verified reliably |
| `script` | Install logic is fully imperative and cannot be reduced to a verifiable resource |

## Declarative Components

A declarative component uses `mode: declarative` and declares all its resources in the `resources:` list.
No `install.sh` / `uninstall.sh` scripts are needed — the executor handles all operations automatically.

**When to use declarative mode:**

- Only installing packages/runtimes/files
- Resources map directly to backend packages
- Component is easy to describe as a list
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

Backend selection is controlled by the **`strategy:`** section of your config file.
Components declare what they need; strategy determines which backend satisfies the requirement.

For backend implementation details, see [`guides/backends.md`](backends.md) and [`specs/api/backend.md`](../specs/api/backend.md).

### Declarative component.yaml example

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
  version: "1.2.3"        # optional: pin to a specific version
```

The `version` field is optional. When omitted, the latest available version is installed and no
version is recorded in state. When set, the version is recorded in state and a change triggers
a `replace` (uninstall + reinstall).

Like `runtime` resources, `version` may reference a param: `version: "${params.version}"`.
This is useful when the profile controls which version is pinned.

#### `runtime`

Installs a runtime via the resolved backend (e.g. mise).

```yaml
- kind: runtime
  id: runtime:node
  name: node
  version: "${params.version}"    # resolved from params_schema + profile params
```

The `version` field typically uses a `${params.version}` reference so that the profile
can control which version is installed. The component's `params_schema` provides a default.

#### `fs`

Deploys a file or directory from the component's `files/` directory.

```yaml
- kind: fs
  id: fs:gitconfig        # stable identifier
  source: files/.gitconfig  # relative to component directory (optional; defaults to files/<basename(path)>)
  path: ~/.gitconfig      # absolute or ~-relative target path
  entry_type: file        # file | dir
  op: link                # link (symlink/junction) | copy
```

`source` is optional. If omitted, the materializer resolves it to `files/<basename(path)>` relative
to the component directory before compilation. The resolved `source` is stored as a structured
`ConcreteFsSource` in the DesiredResourceGraph.

**Source path rules:**
- Component-relative paths (e.g. `files/.gitconfig`) — resolved relative to the component directory. Must not contain `..` segments that escape the component root.
- Home-relative paths (starting with `~/`) — resolved to the user's home directory.
- Absolute paths (starting with `/` or drive letter) — used as-is.

**`..` escape prevention:** Component-relative paths that would escape the component root
(e.g. `../other-component/files/secret`) are rejected at two layers:

1. **Materializer** (compile time) — rejects the path before building the DesiredResourceGraph.
2. **Executor** (apply time) — performs a defensive re-check on the already-resolved absolute path
   to guard against any inconsistency between materialized data and the running component directory.

**Fingerprint:** For `copy` resources, the materializer computes a SHA-256
fingerprint of the source. The planner uses this to detect unchanged sources and skip re-copy (noop).

- `entry_type: file` — SHA-256 of the file's byte content.
- `entry_type: dir` — deterministic tree hash: each file is recorded as `file:<rel-path>:<sha256>`,
  each empty directory as `dir:<rel-path>`, sorted lexicographically, then SHA-256 hashed.
  Symlinks inside the source directory are skipped.

The scope of fingerprinting is controlled by `strategy.fs.fingerprint_policy` (see `specs/data/strategy.md`).
With the default `all_copy`, all source kinds (`component_relative`, `home_relative`, `absolute`) are
fingerprinted. Set to `managed_only` to limit to `component_relative` only, or `none` to disable.

### Platform-specific resources

Use `component.linux.yaml` / `component.windows.yaml` to provide a platform-specific `resources:` list.
When a non-empty `resources:` list is present in the platform file, it completely replaces the base list
(not merged). This allows full override for components that need fundamentally different packages per platform.

```yaml
# component.linux.yaml
resources:
  - kind: package
    id: package:fd
    name: fd-find          # apt name differs from brew name
```

## Managed Script Components

Managed script components use `mode: managed_script`.

The component provides `install.sh` / `uninstall.sh` scripts for imperative installation,
and declares `tool` resources that the executor verifies and records in state.

**This mode is still `danger`** — the scripts execute arbitrary code. But unlike `mode: script`,
the executor owns verification and state: the tool must be present and identifiable after install,
and absent after uninstall, or the operation fails and state is unchanged.

**When to use `managed_script`:**
- Install must use an external script (curl-pipe, vendor install script, single-file binary placement)
- The resulting tool has a stable, verifiable location (absolute path or resolved command)
- Package semantics (`kind: package`) do not apply (no package manager backend is involved)

**Current restriction:** `managed_script` components may only declare `kind: tool` resources.
Mixing `package`, `runtime`, or `fs` resources in the same component is prohibited.

### `managed_script` `component.yaml` schema

```yaml
spec_version: 1
mode: managed_script
description: Brief description

provides:
  - name: package_manager   # optional capability export

scripts:
  install: install.sh       # required
  uninstall: uninstall.sh   # required

resources:
  - kind: tool
    id: tool:<name>         # stable identifier; must not change
    name: <name>            # tool name (for display and state)
    verify:
      identity:             # required: must use a concrete identity type
        type: file          # file | resolved_command | directory | symlink_target
        path: /abs/path/to/tool
        executable: true    # (file type only)
      version:              # optional: only checked for planner compatibility
        command: <tool>
        args: ["--version"]
        parse:
          first_line_regex: "^([0-9]+\\.[0-9]+\\.[0-9]+)"
        constraint: ">=1.0.0"
```

### `tool` resource kind

`tool` resources represent tools installed by a `managed_script` component's scripts.
They differ from `package` resources in that no backend is involved; the script performs installation,
and the executor performs verification.

**`verify.identity`** is **required** for every `tool` resource. Supported identity types:

| type | Description |
|---|---|
| `file` | An absolute file path must exist (optionally: must be executable) |
| `directory` | An absolute directory path must exist |
| `resolved_command` | A command name resolved via PATH must match one of the `expected_path.one_of` entries |
| `symlink_target` | A symlink at a given path must resolve to the expected target |

`versioned_command` (version check without path verification) may NOT be used as the primary identity.
Every `tool` resource must have a concrete identity type (`file`, `resolved_command`, `directory`, or `symlink_target`).

**`verify.version`** is optional. When declared, it is used by the planner as a compatibility signal:
if the version constraint changes, the component is classified as `replace`. It has no effect at runtime
if it was not declared in the original desired state.

### Executor protocol for `managed_script`

**Install (create / replace-install):**

1. Run `install.sh` (must exit 0).
2. Verify all declared `tool` resources using their `verify.identity` contract.
3. If all verify passes: record observed facts (`resolved_path`, `version`) in state and commit atomically.
4. If any verify fails: the operation is a failure; state is unchanged.

**Uninstall (destroy / replace-uninstall):**

1. Run `uninstall.sh` (must exit 0).
2. Perform absence check: `tool.observed.resolved_path` must not exist.
3. If all absence checks pass: remove the component's resources from state and commit atomically.
4. If any absence check fails: the operation is a failure; state is unchanged.

`strengthen` is never generated for `managed_script` components.
Adding a `tool` resource to an existing component always triggers `replace`.

### Example

```yaml
# component.yaml
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

```bash
# install.sh
#!/usr/bin/env bash
set -euo pipefail
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
```

```bash
# uninstall.sh
#!/usr/bin/env bash
set -euo pipefail
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/uninstall.sh)"
```

## Script Component Constraints

Script components (`mode: script`) execute arbitrary shell code. To preserve system safety they are subject
to strict constraints.

The install script MUST:
* Install packages/runtimes via the abstraction layer (not by calling `brew`, `apt`, etc. directly)
* Place configuration files using `link_file` / `copy_file` abstractions
* Exit non-zero on failure

The install script MUST NOT:
* Write to state directly (`state.json`)
* Perform dependency resolution
* Detect platform manually
* Access files outside the component directory and the target home/config paths
* Read or write files of other components

State is written by the executor after install completes.

### When to use script mode

`mode: script` must be declared **explicitly** and is treated as a danger zone — it executes
arbitrary shell code with elevated trust. Use it only when:

* Custom setup logic is required that cannot be expressed as resources
* Conditional install steps are needed
* The component manages side effects not expressible in resource kinds

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

Configuration files must live in `components/<name>/files/`.

Use `link_file` (symlink) or `copy_file` (copy) for placement.
File operations are tracked in state automatically via the `fs` module.

## State Interaction Rules

Components must NOT access `state.json` directly.
State registration is handled by the executor after each successful component operation.

## Parameters (`params_schema`)

Declarative components can declare a `params_schema` to accept profile-injected values.
Parameters allow the same component template to be configured differently per profile
(e.g., different runtime versions on different machines).

### Schema declaration

Add `params_schema` to `component.yaml`:

```yaml
spec_version: 1
description: Node.js runtime
depends: []

params_schema:
  properties:
    version:
      type: string
      default: "22.17.1"

resources:
  - kind: runtime
    id: rt:node
    name: node
    version: "${params.version}"
```

### Schema fields

* `properties` (map) — Parameter definitions keyed by name.
  Each property has:
  * `type` — `string`, `enum`, `object`, or `array`
  * `default` (optional) — Default value when the profile omits this param.
* `required` (list) — Param names that must be provided by the profile.
  A required param must not have a `default`.
* `additional_properties` (bool, default: `false`) — When `false`, unknown param keys are rejected.

### Supported types

| Type | Description | Example |
|---|---|---|
| `string` | Plain string value | `version: "22.17.1"` |
| `enum` | One of a fixed set of values | `channel: stable` with `enum: [stable, beta, nightly]` |
| `object` | Structured source reference | `{ kind: component_relative, path: files/config }` |
| `array` | List of string values | `versions: ["18", "22"]` — used with `for_each` |

`array` params may not be referenced directly in `${params.*}` placeholders — they are consumed
exclusively via `for_each` on a resource (see [for_each](#for_each) below).

### Profile usage

In the profile config, provide params under the component's `params` key:

```yaml
profile:
  components:
    local:
      node:
        params:
          version: "20.0.0"
```

Params with defaults may be omitted — the validator fills them in automatically.

### Template resolution

Resources use `${params.<key>}` placeholders that are replaced before compilation:

```yaml
resources:
  - kind: runtime
    id: rt:node
    name: node
    version: "${params.version}"    # replaced with "20.0.0" from profile
```

Partial substitution is supported: `"prefix-${params.version}-suffix"`.
Multiple references in one field are supported: `"${params.a}-${params.b}"`.

### for_each

`for_each` lets a single resource declaration expand into multiple resources — one per element
of an `array` param. This is the only way to consume an `array`-typed param.

**Format:** `for_each: "params.<key>"` where `<key>` refers to an `array` param.

**Required:** the resource `id` must contain `${item}`. The `${item}` placeholder is also
substituted in all other string fields of the resource.

```yaml
params_schema:
  properties:
    versions:
      type: array
      items:
        type: string
  required:
    - versions

resources:
  - kind: runtime
    id: "rt:node@${item}"
    name: node
    version: "${item}"
    for_each: "params.versions"
```

With profile params `versions: ["18", "22"]`, this expands to two resources:
`rt:node@18` (`version: "18"`) and `rt:node@22` (`version: "22"`).

**State:** each expanded resource is recorded independently. Removing an element from the
array on the next apply causes a `destroy` for the dropped resource while leaving others
unchanged.

**Restriction:** each expanded resource `id` must be unique within the component after
substitution, or the expansion is rejected with a `DuplicateId` error.

### Pipeline integration

The params pipeline runs between fs source materialization and compilation:

1. **Validate** — `params_validator` checks profile params against the schema,
   rejects unknown keys, validates types, and applies defaults.
2. **Materialize** — `params_materializer` replaces `${params.*}` references
   in all resource string fields, producing a `MaterializedComponentSpec`.
3. **Compile** — The compiler uses the materialized spec (if present) instead of the raw spec.

Components without `params_schema` skip this pipeline entirely.

### Restrictions

* `params_schema` is only supported for `declarative` mode components.
* Platform override files (`component.linux.yaml`, etc.) must not declare `params_schema`.
* Params must not change resource count, kind, or id (structural invariance) — except via `for_each`.
* `${params.*}` references in resource `id` fields are forbidden.
* `array` params may only be consumed via `for_each`; direct `${params.<key>}` interpolation is rejected.

## Component Naming Guidelines

Component names must be:
* Lowercase
* Tool-based (name of the tool, not the purpose)
* Stable identifiers that will not need to change

Avoid:
* Version-specific names (`node18`, `python3`)
* Temporary or placeholder names
* Ambiguous category names (`tools`, `utils`)

The component name becomes part of state identity.
Renaming a component is a breaking change requiring state migration.
Moving a component between sources also changes its canonical ID and is therefore a breaking change.

## Component Evolution

When modifying an existing component:
* Maintain uninstall compatibility with the previously recorded state
* Do not change resource identifiers (`id` fields) without a migration plan
* Do not silently expand scope — if new responsibilities are needed, consider a new component

Components must remain loosely coupled to the surrounding system.
Backend changes, state schema evolution, and platform additions must not require component rewrites.
This loose coupling is enforced by: using abstraction APIs rather than calling tools directly,
declaring dependencies rather than assuming presence, and letting the executor own state.
