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
It does not install any features.

## Bootstrap

Bootstrap prepares the execution environment only.
It does not install features. Run `apply` after bootstrap to install your environment.

## Profiles

See [profiles default location](../specs/data/profile.md#File-Location).

Each profile declares which features should be present and (optionally) which version.

Feature keys use **namespace grouping syntax**: the outer key is the source id (`core`, `local`,
or an external source id declared in `sources.yaml`), the inner key is the feature name.

```yaml
# configs/linux.yaml
profile:
  features:
    core:
      git: {}
    local:
      node:
        version: "22.17.1"
      neovim: {}
```

Bundles let you reuse feature sets across configs:

```yaml
bundle:
  use:
    - base

bundles:
  base:
    features:
      core:
        git: {}

profile:
  features:
    local:
      node: {}    # profile.features takes priority over bundles
```

Edit your config to add or remove features, or change versions.

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
      features:
        - node
      backends:
        - npm
  - id: mylab
    type: path
    path: ~/projects/loadout-mylab
    allow:
      features:
        - mypkg
```

Place source content at:

* local features/backends: config home `features/`, `backends/`
* `type: git` external sources: data home `sources/<id>/features/`, `backends/`
* `type: path` external sources: `<path>/features/`, `<path>/backends/`

There is no implicit fallback across `core`, `local`, and external sources.
If you want a non-core source, reference it explicitly in the profile or strategy.

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

Output shows: features to create, destroy, or replace, plus any blocked features.

The plan command never modifies state.

## Apply Command

Execute the plan and apply changes to your environment:

```sh
./loadout apply
```

Apply runs planner → executor → state commit.
Each feature operation is committed atomically.
If a feature fails, execution aborts and state remains unchanged.

## Updating Environment

To install a new feature: add it to your profile, then run `apply`.

To remove a feature: remove it from your profile, then run `apply`.

To change a version: update the `version` field in your profile, then run `apply`.
The feature will be uninstalled and reinstalled at the new version.

## Troubleshooting

**Feature is blocked in plan output**
The feature has an unknown resource kind in state. Check the authoritative state file under your platform state directory for the affected feature.

**Dependency not found in profile**
A feature declares `requires` for a capability that no current feature provides.
Add the provider feature (e.g. `brew`, `mise`) to your profile.

**External feature or backend is rejected**
The source exists on disk but is not allowed by `sources.yaml`.
Add it to the relevant `allow.features` or `allow.backends` entry, or use `allow: "*"` if you intend to trust the entire source.

**State is corrupt**
If `apply` aborts with a state invariant error, do not modify state manually.
Check the error message for which invariant failed and restore from backup if available.
