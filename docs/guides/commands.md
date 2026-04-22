# CLI Command Reference

This document describes every `loadout` subcommand: what it does, when to use it,
and which flags it accepts.

For the big-picture workflow see [usage.md](./usage.md).

## Command Groups

| Group | Purpose |
|---|---|
| [Execution](#execution) | Apply or preview changes to the system |
| [State](#state) | Inspect and migrate the state file |
| [Context](#context) | Set the active config shorthand |
| [Config](#config) | Manage configs |
| [Component](#component) | Manage components (list, show, edit, new, validate, import) |
| [Backend](#backend) | Manage backends (list, show, edit, new, validate, import) |
| [Source](#source) | Manage sources (add, remove, trust, untrust, update) |
| [Diagnostics](#diagnostics) | Health-check the loadout environment |
| [Shell](#shell) | Generate shell completions |

## Execution

### `loadout plan`

```
loadout plan [-c <NAME|PATH>] [--verbose]
```

Runs the full pipeline (load config → resolve → compile → plan) and prints what
`apply` would do, **without making any changes**.

Use this to preview changes before committing.

| Flag | Description |
|---|---|
| `-c, --config <NAME\|PATH>` | Config to use. Defaults to the active context. |
| `--verbose` | Show per-component detail in the plan output. |

**Config resolution:**
- Bare name (e.g. `linux`) → `$XDG_CONFIG_HOME/loadout/configs/linux.yaml`
- Value containing `.yaml` or `.yml` → treated as a literal path
- Omitted → falls back to the active context (see [`context set`](#context-set))

### `loadout apply`

```
loadout apply [-c <NAME|PATH>] [--verbose] [-y]
```

Executes the plan: installs, updates, and removes components as needed.
Atomically commits state after each successful component.

| Flag | Description |
|---|---|
| `-c, --config <NAME\|PATH>` | Config to use. Defaults to the active context. |
| `--verbose` | Show per-component detail. |
| `-y, --yes` | Skip the confirmation prompt. |

Component-level failures are non-fatal: the run continues and a summary is printed.
Exit code is non-zero if any component failed.

### `loadout activate`

```
loadout activate [--shell bash|zsh|fish|pwsh]
```

Reads the env plan cached by the last `apply` and outputs a shell snippet that
sets the environment variables. Evaluate the output in your shell:

```sh
# bash / zsh
eval "$(loadout activate)"

# fish
loadout activate --shell fish | source

# PowerShell
Invoke-Expression (loadout activate --shell pwsh)
```

The shell is auto-detected from `$SHELL` when `--shell` is omitted.

| Flag | Description |
|---|---|
| `--shell <SHELL>` | Explicit target shell: `bash`, `zsh`, `fish`, `pwsh` (alias: `powershell`). |

Error if `apply` has never been run (no cached env plan).

## State

### `loadout state show`

```
loadout state show [--output text|json]
```

Prints a summary of the current state: version, number of installed components and
resources. With `--output json`, outputs the full raw state JSON.

| Flag | Description |
|---|---|
| `--output <FORMAT>` | `text` (default) or `json`. |

### `loadout state migrate`

```
loadout state migrate [--dry-run]
```

Upgrades the state file from an older schema version to the current version (v3).
Run this after upgrading loadout if `plan` or `apply` reports a migration error.

| Flag | Description |
|---|---|
| `--dry-run` | Show what would change without writing. |

## Context

The active context is the default config used by `plan` and `apply` when `-c` is
omitted. It is stored as a bare config name in `$XDG_CONFIG_HOME/loadout/current`.

### `loadout context show`

```
loadout context show
```

Prints the currently active context name, or a message if none is set.

### `loadout context set`

```
loadout context set <NAME>
```

Sets the active context to the given config name.

```sh
loadout context set linux
```

After this, `loadout plan` and `loadout apply` (without `-c`) use `linux.yaml`.

### `loadout context unset`

```
loadout context unset
```

Clears the active context. Subsequent `plan` / `apply` calls require `-c`.

## Config

### `loadout config list`

```
loadout config list [--output text|json]
```

Lists all `.yaml` / `.yml` files in `$XDG_CONFIG_HOME/loadout/configs/`.
The active context is marked with `*` in text mode.

| Flag | Description |
|---|---|
| `--output <FORMAT>` | `text` (default) or `json`. JSON includes `name`, `path`, `active` fields. |

### `loadout config show`

```
loadout config show [<NAME>] [--output text|json]
```

Shows the resolved component list for a config file.
When `<NAME>` is omitted, the active context is used.

| Argument | Description |
|---|---|
| `<NAME>` | Config name or path. Defaults to the active context. |
| `--output <FORMAT>` | `text` (default) or `json`. |

### `loadout config edit`

```
loadout config edit
```

Opens the active context's config file in `$EDITOR` (falls back to `$VISUAL`,
then `vi`, then `nano` on Unix; `notepad` on Windows).

Requires an active context. Error if none is set:

```sh
loadout context set linux
loadout config edit   # opens ~/.config/loadout/configs/linux.yaml
```

### `loadout config init`

```
loadout config init <NAME>
```

Creates a new config file from the built-in template at
`$XDG_CONFIG_HOME/loadout/configs/<NAME>.yaml`. Fails if the file already exists.

```sh
loadout config init linux
loadout context set linux
loadout config edit   # fill in components
```

### `loadout config component add`

```
loadout config component add <ID> [-c <NAME|PATH>]
```

Adds a component to `profile.components` in the config file. When `-c` is omitted, the active context is used.

`<ID>` is a canonical component ID or a bare name:

| Input | Resolved |
|---|---|
| `git` | `local/git` |
| `local/git` | `local/git` |
| `core/node` | `core/node` |

```sh
loadout config component add git            # adds local/git to the active config
loadout config component add core/node -c work
```

> **Note:** Write operations parse and re-emit the YAML. YAML comments in the
> config file are **not preserved**. Use `config edit` if you need to keep
> comments.

### `loadout config component remove`

```
loadout config component remove <ID> [-c <NAME|PATH>]
```

Removes a component from `profile.components`. Prints `no change` if the component is not present. Same `-c` resolution and `<ID>` expansion as `component add`.

> **Note:** Write operations do not preserve YAML comments. See above.

### `loadout config raw show`

```
loadout config raw show [-c <NAME|PATH>]
```

Prints the raw YAML content of the config file as-is, including any comments.
When `-c` is omitted, the active context is used.

This command does **not** modify the file.

### `loadout config raw set`

```
loadout config raw set <PATH> <VALUE> [-c <NAME|PATH>]
```

Sets the value at a dot-separated YAML path. `<VALUE>` is parsed as YAML, so:

| Input | Result type |
|---|---|
| `{}` | empty mapping |
| `true` | boolean |
| `42` | integer |
| `"hello"` | string |

```sh
loadout config raw set profile.components.local.git '{}'
loadout config raw set strategy.rules '[{match: {kind: package}, use: local/brew}]'
```

Missing intermediate nodes are created as empty mappings.

> **Note:** Write operations do not preserve YAML comments.

### `loadout config raw unset`

```
loadout config raw unset <PATH> [-c <NAME|PATH>]
```

Removes the key at the dot-separated path. Prints `no change` if the key is not present.

```sh
loadout config raw unset strategy.rules
```

> **Note:** Write operations do not preserve YAML comments.

## Component

### `loadout component list`

```
loadout component list [--source <ID>] [--output text|json]
```

Lists all available components discovered from all source roots (`local` and any
external sources declared in `sources.yaml`).
Components are grouped by source in text mode.

| Flag | Description |
|---|---|
| `--source <ID>` | Filter to one source (e.g. `--source local`). |
| `--output <FORMAT>` | `text` (default) or `json`. JSON is the full component index keyed by canonical ID. |

### `loadout component show`

```
loadout component show <ID> [--output text|json]
```

Shows details for a single component: mode, description, source directory,
dependencies, and declared resources (declarative mode only).

| Argument | Description |
|---|---|
| `<ID>` | Canonical component ID, e.g. `local/nvim` or `core/git`. |
| `--output <FORMAT>` | `text` (default) or `json`. |

### `loadout component edit`

```
loadout component edit <NAME>
```

Opens the `component.yaml` of a local component in `$EDITOR`. Only `local` source components are editable.

`<NAME>` is a bare name or a `local/`-prefixed ID:

```sh
loadout component edit git         # opens components/git/component.yaml
loadout component edit local/git   # same
```

Components from external sources (e.g. `core/node`) cannot be edited this way.

### `loadout component new`

```
loadout component new <NAME> [--template declarative|script]
```

Scaffolds a new local component directory at
`$XDG_CONFIG_HOME/loadout/components/<NAME>/`.

| Flag | Description |
|---|---|
| `--template <TEMPLATE>` | `declarative` (default) or `script`. |

**`declarative`** — creates `component.yaml` with a commented `resources:` skeleton.

**`script`** — creates `component.yaml` with `mode: script` and stub `install.sh` /
`uninstall.sh` scripts (made executable on Unix).

```sh
loadout component new mypkg                       # declarative
loadout component new myscript --template script  # script mode
```

Fails with an error if the directory already exists.

### `loadout component validate`

```
loadout component validate <ID>
```

Validates a component's directory structure and `component.yaml`. Accepts a canonical
ID or a bare name (resolved to `local/<name>`).

Checks performed:
1. `component.yaml` is parseable and `spec_version` / `mode` are valid.
2. `install.sh` and `uninstall.sh` exist (script mode only).
3. Resource IDs are unique within the component (declarative mode).
4. Each `depends` entry is present in the full component index (warning if absent).

Exit code is non-zero if any **errors** are found. Warnings do not affect the exit code.

```sh
loadout component validate git         # validates local/git
loadout component validate local/nvim  # same with explicit source
```

### `loadout component import`

```
loadout component import <SOURCE/NAME> [--move-config] [--dry-run]
```

Copies a component from an external source into the `local` source directory
(`$XDG_CONFIG_HOME/loadout/components/<NAME>/`).

`<SOURCE/NAME>` must be a canonical ID pointing to an external source (not `local` or `core`).

| Argument / Flag | Description |
|---|---|
| `<SOURCE/NAME>` | Canonical component ID from an external source (e.g. `community/node`). |
| `--move-config` | Also rewrite all config files (`profile.components`, `bundles.*.components`) to reference `local/<NAME>` instead of the external source. |
| `--dry-run` | Show what would happen without writing any files. |

**Bare depends warning:** If the component's `dep.depends` contains bare names (entries without a `/`), a warning is printed. Bare depends are same-source relative references that may not resolve correctly after import. Consider converting them to canonical IDs (e.g. `local/helper`) or using `--help` for guidance.

```sh
loadout component import community/node              # copy to local; config unchanged
loadout component import community/node --move-config  # also rewrite config references
loadout component import community/node --dry-run    # preview only
```

## Backend

### `loadout backend list`

```
loadout backend list [--source <ID>] [--output text|json]
```

Lists all available script backends discovered from `local` and external source
directories. Built-in (Rust-native) backends are not listed; see `backends-builtin`.

| Flag | Description |
|---|---|
| `--source <ID>` | Filter to one source (e.g. `--source local`). |
| `--output <FORMAT>` | `text` (default) or `json`. |

### `loadout backend show`

```
loadout backend show <ID> [--output text|json]
```

Shows details for a single backend: source, directory, `api_version`, and which
scripts (`apply`, `remove`, `status`, `env_pre`, `env_post`) are present.

| Argument | Description |
|---|---|
| `<ID>` | Canonical backend ID, e.g. `local/mise` or `local/npm`. |
| `--output <FORMAT>` | `text` (default) or `json`. |

### `loadout backend edit`

```
loadout backend edit <NAME>
```

Opens the `backend.yaml` of a local backend in `$EDITOR`. Only `local` source backends are editable.

`<NAME>` is a bare name or a `local/`-prefixed ID:

```sh
loadout backend edit mise         # opens backends/mise/backend.yaml
loadout backend edit local/mise   # same
```

### `loadout backend new`

```
loadout backend new <NAME> [--platform unix|unix-windows]
```

Scaffolds a new local backend directory at
`$XDG_CONFIG_HOME/loadout/backends/<NAME>/`.

| Flag | Description |
|---|---|
| `--platform <PLATFORM>` | `unix` (default) or `unix-windows`. |

**`unix`** — creates `backend.yaml` + stub `apply.sh`, `remove.sh`, `status.sh`
(made executable on Unix).

**`unix-windows`** — also creates `apply.ps1`, `remove.ps1`, `status.ps1`.

```sh
loadout backend new mypkg                          # Unix scripts only
loadout backend new mypkg --platform unix-windows  # also PowerShell
```

Fails with an error if the directory already exists.

### `loadout backend validate`

```
loadout backend validate <ID>
```

Validates a backend's directory structure and `backend.yaml`. Accepts a canonical
ID or a bare name (resolved to `local/<name>`).

Checks performed:
1. `backend.yaml` is parseable and `api_version` is valid.
2. Required scripts are present for the current platform:
   `apply`, `remove`, and `status` (error if absent).

Exit code is non-zero if any errors are found.

```sh
loadout backend validate mise         # validates local/mise
loadout backend validate local/mypkg  # same with explicit source
```

### `loadout backend import`

```
loadout backend import <SOURCE/NAME> [--move-strategy] [--dry-run]
```

Copies a backend from an external source into the `local` source directory
(`$XDG_CONFIG_HOME/loadout/backends/<NAME>/`).

`<SOURCE/NAME>` must be a canonical ID pointing to an external source (not `local` or `core`).

| Argument / Flag | Description |
|---|---|
| `<SOURCE/NAME>` | Canonical backend ID from an external source (e.g. `community/brew`). |
| `--move-strategy` | Also rewrite all config files (strategy `rules[*].use`) to reference `local/<NAME>` instead of the external source. |
| `--dry-run` | Show what would happen without writing any files. |

```sh
loadout backend import community/brew                  # copy to local; strategy unchanged
loadout backend import community/brew --move-strategy  # also rewrite strategy references
loadout backend import community/brew --dry-run        # preview only
```

## Source

### `loadout source list`

```
loadout source list [--output text|json]
```

Lists all sources: the two implicit sources (`core` and `local`) plus any external
sources declared in `sources.yaml`.

| Flag | Description |
|---|---|
| `--output <FORMAT>` | `text` (default) or `json`. JSON includes `id`, `kind`, `url`, `ref_spec`, `resolved_commit`, `fetched_at`, `allow`, `local_path`. |

### `loadout source show`

```
loadout source show <ID> [--output text|json]
```

Shows details for a single source.

| Argument | Description |
|---|---|
| `<ID>` | `core`, `local`, or an external source ID declared in `sources.yaml`. |
| `--output <FORMAT>` | `text` (default) or `json`. |

### `loadout source edit`

```
loadout source edit
```

Opens `$XDG_CONFIG_HOME/loadout/sources.yaml` in `$EDITOR`. If the file does not
exist, a template is created first.

### `loadout source add git`

```
loadout source add git <URL> [--id <ID>] [--branch <BRANCH> | --tag <TAG> | --commit <COMMIT>] [--path <PATH>]
```

Registers a new `type: git` external source in `sources.yaml`.

| Argument / Flag | Description |
|---|---|
| `<URL>` | Git repository URL. |
| `--id <ID>` | Source ID. Derived from the repository name if omitted. |
| `--branch <BRANCH>` | Track the tip of this branch (floating ref). |
| `--tag <TAG>` | Pin to this tag. |
| `--commit <COMMIT>` | Pin to this full commit hash. |
| `--path <PATH>` | Repo-relative subdirectory for components/backends. Defaults to `"."`. |

Exactly one of `--branch`, `--tag`, `--commit` may be specified (all optional).
The repository is **not** cloned automatically; run `loadout source update <ID>` to fetch.

```sh
loadout source add git https://github.com/example/community-loadout.git --branch main
loadout source add git https://github.com/example/tools.git --id tools --tag v1.2.0
```

### `loadout source add path`

```
loadout source add path <PATH> [--id <ID>]
```

Registers a new `type: path` external source in `sources.yaml`.

| Argument / Flag | Description |
|---|---|
| `<PATH>` | Filesystem path. Relative paths are resolved from `sources.yaml`'s directory. `~` is expanded. |
| `--id <ID>` | Source ID. Derived from the directory name if omitted. |

The directory must contain at least one of `components/` or `backends/`.
The path must not resolve to the same real directory as the `local` source root.

```sh
loadout source add path ~/projects/loadout-mylab
loadout source add path ../loadout-mylab --id mylab
```

### `loadout source remove`

```
loadout source remove <ID> [--force]
```

Removes a source entry from `sources.yaml`.

Without `--force`, fails if the source is still referenced in any config file
(`profile.components`, `bundles`, `strategy`) or in the installed state.

With `--force`, removes unconditionally and cleans up the corresponding lock entry.

| Argument / Flag | Description |
|---|---|
| `<ID>` | Source ID to remove. |
| `--force` | Skip reference checks and remove unconditionally. |

### `loadout source trust`

```
loadout source trust <ID> [--components <CSV|*>] [--backends <CSV|*>]
```

Adds entries to the `allow` list of a source in `sources.yaml`.
At least one of `--components` or `--backends` must be specified.

Merges into the existing allow-list; duplicates are removed.
If `"*"` is already present for a dimension, no change is made.

| Argument / Flag | Description |
|---|---|
| `<ID>` | Source ID to trust. |
| `--components <CSV\|*>` | Comma-separated component names, or `*` to allow all. |
| `--backends <CSV\|*>` | Comma-separated backend names, or `*` to allow all. |

```sh
loadout source trust community --components '*'
loadout source trust community --backends brew,mise
```

### `loadout source untrust`

```
loadout source untrust <ID> [--components <CSV|*>] [--backends <CSV|*>] [--force]
```

Removes entries from the `allow` list of a source in `sources.yaml`.
At least one of `--components` or `--backends` must be specified.

Removing `"*"` (wildcard) requires `--force`.
Attempting to remove specific names when the current allow-list is `"*"` returns an error;
revoke the wildcard with `--force` first, then re-trust specific entries.
If the allow-list becomes empty after removal, the source reverts to deny-all (no `allow` field).

| Argument / Flag | Description |
|---|---|
| `<ID>` | Source ID to untrust. |
| `--components <CSV\|*>` | Comma-separated component names, or `*` to revoke all. |
| `--backends <CSV\|*>` | Comma-separated backend names, or `*` to revoke all. |
| `--force` | Required when revoking a wildcard (`*`). |

```sh
loadout source untrust community --backends brew      # remove brew from backends allow-list
loadout source untrust community --components '*' --force  # revoke all-components wildcard
```

### `loadout source update`

```
loadout source update <ID> [--to-commit <COMMIT>] [--relock]
```

Fetches the latest commits from a `type: git` source and updates `sources.lock.yaml`.
Only `type: git` sources are supported; `type: path` sources return an error.

| Argument / Flag | Description |
|---|---|
| `<ID>` | Source ID to update. Must be `type: git`. |
| `--to-commit <COMMIT>` | Check out this specific commit hash instead of following the declared `ref`. |
| `--relock` | Recompute `manifest_hash` and update the lock file without fetching or checking out. |

**Modes:**

- **Default (floating ref):** `git fetch --prune` → check out the tip of the declared `ref` (branch/tag/commit) → update lock.
- **`--to-commit`:** `git fetch` → `git checkout --detach <COMMIT>` → update lock.
- **`--relock`:** Skip fetch and checkout entirely; recompute `manifest_hash` from the current working tree and rewrite the lock entry.

The lock file records `resolved_commit` (full 40-character hash), `fetched_at` (UTC RFC 3339), and `manifest_hash` (SHA-256 over `components/**/*.yaml` + `backends/**/*.yaml`).

```sh
loadout source update community                          # follow declared ref
loadout source update community --to-commit abc123def   # pin to specific commit
loadout source update community --relock                 # refresh lock, no fetch
```

## Diagnostics

### `loadout doctor`

```
loadout doctor [-c <NAME|PATH>]
```

Runs a series of environment checks and prints a summary. Useful for diagnosing
why `plan` / `apply` / `activate` might fail.

Checks performed:

| Check | What it verifies |
|---|---|
| Platform | Detected platform (Linux / Windows / WSL) |
| Directories | `config_home`, `data_home`, `state_home`, `cache_home` exist or can be created |
| `state.json` | Readable and parseable (or absent — which is fine) |
| `sources.yaml` | Readable (absent is fine — defaults to empty) |
| Env cache | `env_plan.json` present (warn if missing and `activate` would fail) |
| `$SHELL` | Shell variable set |
| `LOADOUT_ROOT` | Set and points to a valid directory (if present) |
| Config file | Readable (if `-c` is given) |

`doctor` does **not** modify any state and does **not** override the planner.

| Flag | Description |
|---|---|
| `-c, --config <NAME\|PATH>` | Also check the specified config file for readability. |

## Shell

### `loadout completions`

```
loadout completions <SHELL>
```

Outputs a shell completion script for the given shell to stdout.
Supported shells: `bash`, `elvish`, `fish`, `powershell`, `zsh`.

```sh
# bash — append to .bashrc
loadout completions bash >> ~/.bashrc

# zsh — install as a completion file
loadout completions zsh > ~/.zfunc/_loadout

# fish
loadout completions fish > ~/.config/fish/completions/loadout.fish
```

## Common Patterns

### First-time setup

```sh
loadout config init linux
loadout context set linux
loadout config component add git
loadout config component add core/node
loadout plan         # preview
loadout apply        # install
eval "$(loadout activate)"
```

### Switching between configs

```sh
loadout context set work
loadout plan                # preview what changes
loadout apply -y            # apply without prompt
```

### Inspecting the environment

```sh
loadout doctor                      # overall health check
loadout state show                  # what is installed
loadout component list --source local # what local components are available
loadout source list                 # which sources are active
```

### Using JSON output in scripts

```sh
# List installed components as JSON
loadout state show --output json | jq '.components | keys'

# Get details of a specific component
loadout component show local/nvim --output json | jq '.mode'
```

### Editing configs and components

```sh
# Open the active config with $EDITOR (comments preserved)
loadout config edit

# Add/remove a component without opening an editor
loadout config component add local/git
loadout config component remove local/git -c work

# Low-level YAML access
loadout config raw show
loadout config raw set strategy.rules '[{match: {kind: package}, use: local/brew}]'
loadout config raw unset strategy.rules

# Edit a local component's component.yaml directly
loadout component edit git

# Edit a local backend's backend.yaml directly
loadout backend edit mise

# Edit sources.yaml (created from template if absent)
loadout source edit
```

### Using external sources

```sh
# Register a community source (not cloned yet)
loadout source add git https://github.com/example/community-loadout.git --branch main --id community

# Allow specific components/backends from the source
loadout source trust community --components 'node,python' --backends npm

# Clone and initialise the repo (writes sources.lock.yaml)
loadout source update community

# Use the component in your config
loadout config component add community/node

# Keep the source in sync later
loadout source update community          # follow declared branch
loadout source update community --to-commit abc123   # pin to a specific commit
loadout source update community --relock             # refresh lock hash only
```

### Importing an external resource into local

```sh
# Preview what importing community/node would do
loadout component import community/node --dry-run

# Copy community/node to local and rewrite all config references
loadout component import community/node --move-config

# Copy community/brew backend to local and rewrite all strategy references
loadout backend import community/brew --move-strategy
```
