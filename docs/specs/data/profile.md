# Profile Specification

## Scope

This document defines the normative contract for profile files.

Covered: schema, field semantics, validation rules, and how profiles interact with the planner.

Not covered: how to write or manage profiles (see `guides/usage.md`).

## Schema

A config file (`config.yaml`) contains the following top-level sections:

```yaml
# configs/linux.yaml
profile:
  components:
    <source_id>:
      <component_name>: {}
      <component_name>:
        version: "<string>"

bundle:            # optional — lists which bundles to apply
  use:
    - <bundle_name>

bundles:           # optional — named bundle definitions
  <bundle_name>:
    components:
      <source_id>:
        <component_name>: {}

strategy:          # optional — may be omitted; defaults to Strategy::default()
  ...
```

`profile.components` is required. All other top-level keys are optional.

The `components` map uses **namespace grouping syntax**: the outer key is a `source_id`,
the inner key is a component name. This is the only accepted syntax.
Bare component names and canonical `source_id/name` direct form are rejected.

After expansion and normalization, all component keys become canonical `source_id/name`.
Source existence is not verified at parse time; it is verified at `SourceRegistry` construction.

## File Location

Config files are located in a platform-defined directory:

* Linux/WSL: `$XDG_CONFIG_HOME/loadout/configs/`
* Linux/WSL fallback: `~/.config/loadout/configs/`
* Windows: `%APPDATA%\loadout\configs\`

The config is selected with `--config` / `-c`:

```
loadout apply -c linux       →  {config_home}/configs/linux.yaml
loadout apply -c ./work.yaml →  ./work.yaml  (any value containing .yaml)
```

### `components` (required)

An object where each key is a component identifier (string).
The value is a component configuration map (may be empty `{}`).

### Component configuration map

Empty map `{}` is valid and equivalent to no configuration.

Optional fields:

* `version` (string) — Desired version of the component.
  Interpretation is component-specific. Core passes it to the component script via
  `LOADOUT_COMPONENT_CONFIG_VERSION` and records it in state.
  No format constraints are imposed by core.

## Semantics

A profile declares intent: which components should be present and with what configuration.

A profile does NOT describe how to install components, which backend to use,
or any platform-specific behavior. That belongs to strategy and component scripts.

Components absent from the profile are treated as "not desired".
If such a component exists in state, the planner will classify it as `destroy`.

All component keys in the normalized profile are canonical `source_id/name` IDs.
Normalization (grouping expansion) is performed by the `config` crate before pipeline entry.
The planner, resolver, executor, and state never see raw config syntax.

## Validation Rules

* `profile.components` must be a map.
* The outer key (`source_id`) must be a non-empty string.
* The inner key (component name) must be a non-empty string.
* The inner value must be a map (or empty `{}`).
* Duplicate canonical IDs produced after normalization are rejected.
* Bare component names (keys without a `source_id` nesting) are not accepted.
* Canonical direct form (`source_id/name: {}` at the `components` top level) is not accepted.
* Unknown fields in the component configuration map are permitted and ignored by core.

## Bundle Expansion

Bundles allow reusable component sets to be shared across configs.

Merge order (lowest → highest priority):
1. Bundles in `bundle.use` list order — last entry wins on conflict.
2. `profile.components` overwrites all bundle-merged components.

`bundle.use` values are bundle names (plain strings).
Referencing an undefined bundle name is an error.

## Interaction with Planner

The profile is one of three inputs to the planner (alongside state and strategy).

The planner uses the profile as the "desired state":

* Component in profile but not in state → classified as `create`
* Component in state but not in profile → classified as `destroy`
* Component in both with matching version → classified as `noop` or `strengthen`
* Component in both with version mismatch → classified as `replace`

See `specs/algorithms/planner.md` for the full classification rules.

## Examples

Minimal config — profile only:

```yaml
# configs/linux.yaml
profile:
  components:
    local:
      git: {}
      bash: {}
```

Canonical IDs after normalization:

```
local/git
local/bash
```

Components with version, multiple sources:

```yaml
profile:
  components:
    core:
      git: {}
    local:
      node:
        version: "22.17.1"
      python:
        version: "3.12"
      mycomponent: {}

strategy:
  runtime:
    default_backend: local/mise
```

Config using bundles:

```yaml
bundle:
  use:
    - base
    - work          # last entry wins on conflict

bundles:
  base:
    components:
      core:
        git: {}
  work:
    components:
      dev:
        terraform: {}

profile:
  components:
    local:
      nvim: {}      # profile.components always wins over bundles
```
