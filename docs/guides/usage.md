# Usage Guide

## Installation

Clone the repository and run the bootstrap script for your platform.

```sh
# Linux / WSL
git clone https://github.com/massa-kj/loadout.git ~/loadout
cd ~/loadout
./platforms/linux/bootstrap.sh   # or platforms/wsl/bootstrap.sh
```

Bootstrap installs the minimum dependencies (git, jq, yq) and sets up the environment.
It does not install any components.

## Bootstrap

Bootstrap prepares the execution environment only.
It does not install components. Run `apply` after bootstrap to install your environment.

## Profiles

See [profiles default location](../specs/data/profile.md#File-Location).

Each profile declares which components should be present and (optionally) which version.

Component keys use **namespace grouping syntax**: the outer key is the source id (`core`, `local`,
or an external source id declared in `sources.yaml`), the inner key is the component name.

```yaml
# configs/linux.yaml
profile:
  components:
    core:
      git: {}
    local:
      node:
        version: "22.17.1"
      neovim: {}
```

Bundles let you reuse component sets across configs:

```yaml
bundle:
  use:
    - base

bundles:
  base:
    components:
      core:
        git: {}

profile:
  components:
    local:
      node: {}    # profile.components takes priority over bundles
```

Edit your config to add or remove components, or change versions.

See `specs/data/profile.md` for the full schema.

## Sources

See [source registry file default location](../specs/data/sources.md#File-Location).

Example `sources.yaml` with a git source and a local path source:

```yaml
sources:
  - id: community
    type: git
    url: https://github.com/example/community-loadout.git
    ref:
      branch: main
    allow:
      components:
        - node
      backends:
        - npm
  - id: mylab
    type: path
    path: ~/projects/loadout-mylab
    allow:
      components:
        - mypkg
```

Place source content at:

* local components/backends: config home `components/`, `backends/`
* `type: git` external sources: data home `sources/<id>/components/`, `backends/`
* `type: path` external sources: `<path>/components/`, `<path>/backends/`

There is no implicit fallback across `core`, `local`, and external sources.
If you want a non-core source, reference it explicitly in the profile or strategy.

### Source lifecycle

**Add** a new source and fetch it:

```sh
loadout source add git https://github.com/example/community-loadout.git --branch main
loadout source trust community --components '*'   # allow all components
loadout source update community                 # clone / fetch
```

**Keep up to date** (floating branch):

```sh
loadout source update community          # fetch latest, update lock
```

**Pin to a specific commit:**

```sh
loadout source update community --to-commit <full-hash>
```

**Refresh the lock hash without fetching** (e.g. after a manual `git` operation):

```sh
loadout source update community --relock
```

### Importing a resource into local

If you want to take full ownership of a component or backend from an external source,
use `import` to copy it into your `local` source directory:

```sh
# Copy component to local/ and rewrite all config references
loadout component import community/node --move-config

# Copy backend to local/ and rewrite all strategy references
loadout backend import community/brew --move-strategy

# Preview without writing
loadout component import community/node --dry-run
```

After import, the external source is no longer required for that resource.
If the imported component has bare depends (same-source relative references),
a warning is printed; review and convert them to canonical IDs if necessary.

## Policies

See [strategies default location](../specs/data/strategy.md#File-Location).

## State

See [state file default location](../specs/data/state.md#File-Location).

## Command Reference

For a complete list of all CLI commands and flags, see **[commands.md](./commands.md)**.

## Plan Command

Preview what would happen without making any changes:

```sh
./loadout plan
```

Output shows: components to create, destroy, or replace, plus any blocked components.

The plan command never modifies state.

## Apply Command

Execute the plan and apply changes to your environment:

```sh
./loadout apply
```

Apply runs planner → executor → state commit.
Each component operation is committed atomically.
If a component fails, execution aborts and state remains unchanged.

## Updating Environment

To install a new component: add it to your profile, then run `apply`.

To remove a component: remove it from your profile, then run `apply`.

To change a version: update the `version` field in your profile, then run `apply`.
The component will be uninstalled and reinstalled at the new version.

## Strategy

The `strategy:` section of your config file controls which backend handles each resource.

### Basic rule set

```yaml
# configs/linux.yaml
strategy:
  rules:
    - match:
        kind: package
      use: local/brew
    - match:
        kind: runtime
      use: local/mise
```

Rules are evaluated against every resource.
The **most specific match** wins; if multiple rules tie on specificity, the **last rule wins**.

### Per-name override

```yaml
strategy:
  rules:
    - match:
        kind: package
      use: local/brew
    - match:
        kind: package
        name: ripgrep
      use: local/apt
```

`name` + `kind` is more specific than `kind` alone, so `ripgrep` is routed to `apt`.

### Groups

Group a set of resource names and match them together:

```yaml
strategy:
  groups:
    npm_global:
      package:
        - eslint
        - prettier

  rules:
    - match:
        kind: package
      use: local/brew
    - match:
        kind: package
        group: npm_global
      use: local/npm
```

For the full specificity table and validation rules, see [specs/data/strategy.md](../specs/data/strategy.md).

## Troubleshooting

**Component is blocked in plan output**
The component has an unknown resource kind in state. Check the authoritative state file under your platform state directory for the affected component.

**Dependency not found in profile**
A component declares `requires` for a capability that no current component provides.
Add the provider component (e.g. `brew`, `mise`) to your profile.

**External component or backend is rejected**
The source exists on disk but is not allowed by `sources.yaml`.
Add it to the relevant `allow.components` or `allow.backends` entry, or use `allow: "*"` if you intend to trust the entire source.

**State is corrupt**
If `apply` aborts with a state invariant error, do not modify state manually.
Check the error message for which invariant failed and restore from backup if available.
