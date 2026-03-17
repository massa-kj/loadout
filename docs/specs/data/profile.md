# Profile Specification

## Scope

This document defines the normative contract for profile files.

Covered: schema, field semantics, validation rules, and how profiles interact with the planner.

Not covered: how to write or manage profiles (see `guides/usage.md`).

## Schema

```yaml
features:
  <feature_id>: {}
  <feature_id>:
    version: "<string>"
```

Top-level key is `features`. All other top-level keys are reserved.

## File Location

Profile directory is platform-defined:

* Linux/WSL: `$XDG_CONFIG_HOME/loadout/profiles/`
* Linux/WSL fallback: `~/.config/loadout/profiles/`
* Windows: `%APPDATA%\loadout\profiles\`

`LOADOUT_PROFILES_DIR` may override the directory path.

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
or any platform-specific behavior. That belongs to policy and feature scripts.

Features absent from the profile are treated as "not desired".
If such a feature exists in state, the planner will classify it as `destroy`.

Feature key normalization rules:

* Bare feature key `git` is normalized to `core/git`
* Canonical feature key `core/git` is preserved as-is
* `user` and external source features must be explicit, e.g. `user/myfeat`, `community/node`

The normalized canonical IDs are what planner, resolver, executor, and state use internally.

## Validation Rules

* `features` must be an object.
* Each key must be a non-empty string.
* Each key must be either a bare name or a canonical ID of the form `<source_id>/<name>`.
* Each value must be a map (or null/empty, equivalent to `{}`).
* Unknown fields in the feature configuration map are permitted and ignored by core.

## Interaction with Planner

The profile is one of three inputs to the planner (alongside state and policy).

The planner uses the profile as the "desired state":

* Feature in profile but not in state → classified as `create`
* Feature in state but not in profile → classified as `destroy`
* Feature in both with matching version → classified as `noop` or `strengthen`
* Feature in both with version mismatch → classified as `replace`

See `specs/algorithms/planner.md` for the full classification rules.

## Examples

Minimal profile — feature with no configuration:

```yaml
features:
  git: {}
  bash: {}
```

Equivalent canonical form after normalization:

```yaml
features:
  core/git: {}
  core/bash: {}
```

Feature with version:

```yaml
features:
  node:
    version: "22.17.1"
  python:
    version: "3.12"
  user/myfeat: {}
```
