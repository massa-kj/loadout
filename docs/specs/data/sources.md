# Sources Specification

## Scope

This document defines the normative contract for `sources.yaml`.

Covered: schema, source kinds, implicit sources, allow-list semantics, and path resolution.

Not covered: source lifecycle commands (`add`, `update`, `remove`), git clone/update automation, or non-git source types.

## Purpose

The source registry maps canonical IDs to concrete feature/backend directories.
It makes source resolution explicit and deterministic.

Core must not guess providers from filesystem location.
Profile and Strategy inputs determine source selection via canonical IDs.

## Source Kinds

Resolver handles three source kinds:

- `core` — implicit built-in source shipped with this repository
- `local` — implicit local override source under user config directory
- external sources — explicitly declared in `sources.yaml`

Reserved source IDs:

- `core`
- `local`
- `official` (reserved for future use)

Reserved source IDs must not appear in `sources.yaml`.

## File Location

`sources.yaml` path is platform-defined:

- Linux/WSL: `$XDG_CONFIG_HOME/loadout/sources.yaml`
- Linux/WSL fallback: `~/.config/loadout/sources.yaml`
- Windows: `%APPDATA%\loadout\sources.yaml`

`LOADOUT_SOURCES_FILE` may override the path.

If `sources.yaml` does not exist, only implicit sources (`core`, `local`) are available.

## Schema

```yaml
sources:
  - id: foo
    type: git
    url: https://github.com/foo/loadout-features
    commit: abcdefg
    allow:
      features:
        - node
      backends:
        - brew
```

## Field Semantics

### `sources`

List of external source definitions.
Each item defines one external source ID.

### `id`

Canonical source identifier.
Must be a non-empty string.
Must not be one of the reserved IDs.

### `type`

Currently only `git` is supported.

### `url`

Git repository URL for the external source.
Core does not clone or update this repository automatically.
Repository synchronization is handled outside the core execution path.

### `commit`

Pinned revision identifier for the external source.
Core does not fetch this revision automatically.
It is declarative metadata for external source lifecycle tooling.

### `allow`

Allow-list for resources importable from the external source.
Allow-list is mandatory in the safety model.
If omitted, the source is deny-all.

Supported forms:

```yaml
allow: "*"
```

Allows all features and all backends.

```yaml
allow:
  features: "*"
  backends: "*"
```

Allows all features or all backends by kind.

```yaml
allow:
  features:
    - node
    - python
  backends:
    - brew
```

Allows only the listed resource names.

## Path Resolution

Source directories are derived by source kind.

### Features

- `core/<name>` → `{repo}/features/<name>`
- `local/<name>` → config home `features/<name>`
- `<external>/<name>` → data home `sources/<external>/features/<name>`

### Backends

- `core/<name>` → `{repo}/backends/<name>`
- `local/<name>` → config home `backends/<name>`
- `<external>/<name>` → data home `sources/<external>/backends/<name>`

## Resolution Rules

- Canonical IDs are authoritative.
- Bare names are normalized to `core/<name>` before resolver execution.
- `local` and external sources must be explicit in canonical IDs.
- No implicit fallback between `local`, external, and `core` is permitted.
- If a dependency references an external source item not allowed by `allow`, resolution must abort.
- If a backend ID references an external source item not allowed by `allow`, backend loading must abort.

## Validation Rules

- `sources` must be an array if present.
- Every `id` must be unique.
- Every `type` must be `git`.
- Reserved IDs must be rejected.
- Missing `allow` means deny-all.
- Unknown top-level fields in each source entry are reserved.

## Examples

Minimal deny-all source:

```yaml
sources:
  - id: community
    type: git
    url: https://github.com/example/community-loadout
    commit: 0123456
```

All features allowed, only selected backends allowed:

```yaml
sources:
  - id: tools
    type: git
    url: https://github.com/example/tools-loadout
    commit: abcdef0
    allow:
      features: "*"
      backends:
        - npm
        - uv
```
