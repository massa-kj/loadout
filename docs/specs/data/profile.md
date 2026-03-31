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
  features:
    <source_id>:
      <feature_name>: {}
      <feature_name>:
        version: "<string>"

bundle:            # optional — lists which bundles to apply
  use:
    - <bundle_name>

bundles:           # optional — named bundle definitions
  <bundle_name>:
    features:
      <source_id>:
        <feature_name>: {}

strategy:          # optional — may be omitted; defaults to Strategy::default()
  ...
```

`profile.features` is required. All other top-level keys are optional.

The `features` map uses **namespace grouping syntax**: the outer key is a `source_id`,
the inner key is a feature name. This is the only accepted syntax.
Bare feature names and canonical `source_id/name` direct form are rejected.

After expansion and normalization, all feature keys become canonical `source_id/name`.
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

### `features` (required)

An object where each key is a feature identifier (string).
The value is a feature configuration map (may be empty `{}`).

### Feature configuration map

Empty map `{}` is valid and equivalent to no configuration.

Optional fields:

* `version` (string) — Desired version of the feature.
  Interpretation is feature-specific. Core passes it to the feature script via
  `LOADOUT_FEATURE_CONFIG_VERSION` and records it in state.
  No format constraints are imposed by core.

## Semantics

A profile declares intent: which features should be present and with what configuration.

A profile does NOT describe how to install features, which backend to use,
or any platform-specific behavior. That belongs to strategy and feature scripts.

Features absent from the profile are treated as "not desired".
If such a feature exists in state, the planner will classify it as `destroy`.

All feature keys in the normalized profile are canonical `source_id/name` IDs.
Normalization (grouping expansion) is performed by the `config` crate before pipeline entry.
The planner, resolver, executor, and state never see raw config syntax.

## Validation Rules

* `profile.features` must be a map.
* The outer key (`source_id`) must be a non-empty string.
* The inner key (feature name) must be a non-empty string.
* The inner value must be a map (or empty `{}`).
* Duplicate canonical IDs produced after normalization are rejected.
* Bare feature names (keys without a `source_id` nesting) are not accepted.
* Canonical direct form (`source_id/name: {}` at the `features` top level) is not accepted.
* Unknown fields in the feature configuration map are permitted and ignored by core.

## Bundle Expansion

Bundles allow reusable feature sets to be shared across configs.

Merge order (lowest → highest priority):
1. Bundles in `bundle.use` list order — last entry wins on conflict.
2. `profile.features` overwrites all bundle-merged features.

`bundle.use` values are bundle names (plain strings).
Referencing an undefined bundle name is an error.

## Interaction with Planner

The profile is one of three inputs to the planner (alongside state and strategy).

The planner uses the profile as the "desired state":

* Feature in profile but not in state → classified as `create`
* Feature in state but not in profile → classified as `destroy`
* Feature in both with matching version → classified as `noop` or `strengthen`
* Feature in both with version mismatch → classified as `replace`

See `specs/algorithms/planner.md` for the full classification rules.

## Examples

Minimal config — profile only:

```yaml
# configs/linux.yaml
profile:
  features:
    local:
      git: {}
      bash: {}
```

Canonical IDs after normalization:

```
local/git
local/bash
```

Features with version, multiple sources:

```yaml
profile:
  features:
    core:
      git: {}
    local:
      node:
        version: "22.17.1"
      python:
        version: "3.12"
      myfeat: {}

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
    features:
      core:
        git: {}
  work:
    features:
      dev:
        terraform: {}

profile:
  features:
    local:
      nvim: {}      # profile.features always wins over bundles
```
