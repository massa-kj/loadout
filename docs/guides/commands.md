# CLI Command Reference

This document describes every `loadout` subcommand: what it does, when to use it,
and which flags it accepts.

For the big-picture workflow see [usage.md](./usage.md).

---

## Command Groups

| Group | Purpose |
|---|---|
| [Execution](#execution) | Apply or preview changes to the system |
| [State](#state) | Inspect and migrate the state file |
| [Context](#context) | Set the active config shorthand |
| [Config](#config) | Read declared configs |
| [Feature](#feature) | Explore available features |
| [Backend](#backend) | Explore available backends |
| [Source](#source) | Inspect declared sources |
| [Diagnostics](#diagnostics) | Health-check the loadout environment |
| [Shell](#shell) | Generate shell completions |

---

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
| `--verbose` | Show per-feature detail in the plan output. |

**Config resolution:**
- Bare name (e.g. `linux`) → `$XDG_CONFIG_HOME/loadout/configs/linux.yaml`
- Value containing `.yaml` or `.yml` → treated as a literal path
- Omitted → falls back to the active context (see [`context set`](#context-set))

---

### `loadout apply`

```
loadout apply [-c <NAME|PATH>] [--verbose] [-y]
```

Executes the plan: installs, updates, and removes features as needed.
Atomically commits state after each successful feature.

| Flag | Description |
|---|---|
| `-c, --config <NAME\|PATH>` | Config to use. Defaults to the active context. |
| `--verbose` | Show per-feature detail. |
| `-y, --yes` | Skip the confirmation prompt. |

Feature-level failures are non-fatal: the run continues and a summary is printed.
Exit code is non-zero if any feature failed.

---

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

---

## State

### `loadout state show`

```
loadout state show [--output text|json]
```

Prints a summary of the current state: version, number of installed features and
resources. With `--output json`, outputs the full raw state JSON.

| Flag | Description |
|---|---|
| `--output <FORMAT>` | `text` (default) or `json`. |

---

### `loadout state migrate`

```
loadout state migrate [--dry-run]
```

Upgrades the state file from an older schema version to the current version (v3).
Run this after upgrading loadout if `plan` or `apply` reports a migration error.

| Flag | Description |
|---|---|
| `--dry-run` | Show what would change without writing. |

---

## Context

The active context is the default config used by `plan` and `apply` when `-c` is
omitted. It is stored as a bare config name in `$XDG_CONFIG_HOME/loadout/current`.

### `loadout context show`

```
loadout context show
```

Prints the currently active context name, or a message if none is set.

---

### `loadout context set`

```
loadout context set <NAME>
```

Sets the active context to the given config name.

```sh
loadout context set linux
```

After this, `loadout plan` and `loadout apply` (without `-c`) use `linux.yaml`.

---

### `loadout context unset`

```
loadout context unset
```

Clears the active context. Subsequent `plan` / `apply` calls require `-c`.

---

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

---

### `loadout config show`

```
loadout config show [<NAME>] [--output text|json]
```

Shows the resolved feature list for a config file.
When `<NAME>` is omitted, the active context is used.

| Argument | Description |
|---|---|
| `<NAME>` | Config name or path. Defaults to the active context. |
| `--output <FORMAT>` | `text` (default) or `json`. |

---

## Feature

### `loadout feature list`

```
loadout feature list [--source <ID>] [--output text|json]
```

Lists all available features discovered from all source roots (`local` and any
external sources declared in `sources.yaml`).
Features are grouped by source in text mode.

| Flag | Description |
|---|---|
| `--source <ID>` | Filter to one source (e.g. `--source local`). |
| `--output <FORMAT>` | `text` (default) or `json`. JSON is the full feature index keyed by canonical ID. |

---

### `loadout feature show`

```
loadout feature show <ID> [--output text|json]
```

Shows details for a single feature: mode, description, source directory,
dependencies, and declared resources (declarative mode only).

| Argument | Description |
|---|---|
| `<ID>` | Canonical feature ID, e.g. `local/nvim` or `core/git`. |
| `--output <FORMAT>` | `text` (default) or `json`. |

---

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

---

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

---

## Source

### `loadout source list`

```
loadout source list [--output text|json]
```

Lists all sources: the two implicit sources (`core` and `local`) plus any external
sources declared in `sources.yaml`.

| Flag | Description |
|---|---|
| `--output <FORMAT>` | `text` (default) or `json`. JSON includes `id`, `kind`, `url`, `commit`, `allow`, `local_path`. |

---

### `loadout source show`

```
loadout source show <ID> [--output text|json]
```

Shows details for a single source.

| Argument | Description |
|---|---|
| `<ID>` | `core`, `local`, or an external source ID declared in `sources.yaml`. |
| `--output <FORMAT>` | `text` (default) or `json`. |

---

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

---

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

---

## Common Patterns

### First-time setup

```sh
loadout context set linux
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
loadout feature list --source local # what local features are available
loadout source list                 # which sources are active
```

### Using JSON output in scripts

```sh
# List installed features as JSON
loadout state show --output json | jq '.features | keys'

# Get details of a specific feature
loadout feature show local/nvim --output json | jq '.mode'
```
