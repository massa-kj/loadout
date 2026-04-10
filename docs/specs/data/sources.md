# Sources Specification

## Scope

This document defines the normative contract for `sources.yaml` and `sources.lock.yaml`.

Covered: schema, source kinds, implicit sources, allow-list semantics, path resolution, and lock file format.

Not covered: source lifecycle commands (`add`, `update`, `remove`), git clone/update automation.

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

If `sources.yaml` does not exist, only implicit sources (`core`, `local`) are available.

`sources.lock.yaml` resides in the same directory as `sources.yaml`:

- Linux/WSL: `$XDG_CONFIG_HOME/loadout/sources.lock.yaml`
- Windows: `%APPDATA%\loadout\sources.lock.yaml`

If `sources.lock.yaml` does not exist, `type: git` sources are treated as unlocked
(no resolved commit recorded). `type: path` sources are never written to the lock file.

## Schema

### `sources.yaml`

```yaml
sources:
  - id: community
    type: git
    url: https://github.com/example/community-loadout.git
    ref:
      branch: main        # exactly one of: branch | tag | commit
    path: .               # repo-relative subdirectory (optional, default ".")
    allow:
      features:
        - node
        - python
      backends:
        - mise
  - id: mylab
    type: path
    path: ../loadout-mylab   # filesystem path (relative to sources.yaml, or absolute)
    allow:
      features:
        - node
      backends:
        - mise
```

### `sources.lock.yaml`

```yaml
sources:
  community:
    resolved_commit: abcdef1234567890abcdef1234567890abcdef12
    fetched_at: 2026-04-06T12:34:56Z
    manifest_hash: "sha256:..."
```

## Field Semantics — `sources.yaml`

### `sources`

List of external source definitions.
Each item defines one external source ID.

### `id`

Canonical source identifier.
Must be a non-empty string.
Must not be one of the reserved IDs.

### `type`

Source kind. One of:

| Value | Meaning |
|---|---|
| `git` | Remote git repository cloned to the data directory. |
| `path` | Local filesystem directory managed by the user. |

### `url` (type: git only)

Git repository URL.
Required for `type: git`; must not be specified for `type: path`.

### `ref` (type: git only)

Desired revision declaration. Exactly one of the following sub-fields must be set:

| Sub-field | Meaning |
|---|---|
| `branch` | Track the tip of this branch (floating). |
| `tag` | Pin to this tag. |
| `commit` | Pin to this full commit hash. |

If `ref` is omitted, no automatic synchronization is performed.

### `path`

| Source type | Semantics |
|---|---|
| `type: git` | Repo-relative subdirectory containing `features/` and/or `backends/`. Defaults to `"."` (repository root). Must be relative; `..` and absolute paths are forbidden. |
| `type: path` | Filesystem path to the local source directory. Relative paths are resolved relative to `sources.yaml`'s parent directory. Absolute paths are stored as-is after canonicalization. `~` expansion is supported. |

The directory must contain at least one of `features/` or `backends/`.

### `allow`

Allow-list for resources importable from the external source.
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

## Field Semantics — `sources.lock.yaml`

### `sources`

Map from source ID to lock entry. Only `type: git` sources appear here.

### `resolved_commit`

Full 40-character commit hash resolved at last `source update`.
Short hashes are not permitted.

### `fetched_at`

UTC timestamp in RFC 3339 format (`YYYY-MM-DDTHH:MM:SSZ`) recorded at last fetch.

### `manifest_hash`

SHA-256 hash of the source's loadout manifest files (`features/**/*.yaml`,
`backends/**/*.yaml`) at the time of last fetch.
Computed over the repo subtree specified by `path`, not the full repository.

## Path Resolution

Source directories are derived by source kind.

### Features

- `core/<name>` → `{repo}/features/<name>`
- `local/<name>` → config home `features/<name>`
- `<external>/<name>` (type: git) → data home `sources/<external>/features/<name>`
- `<external>/<name>` (type: path) → `<resolved_path>/features/<name>`

### Backends

- `core/<name>` → `{repo}/backends/<name>`
- `local/<name>` → config home `backends/<name>`
- `<external>/<name>` (type: git) → data home `sources/<external>/backends/<name>`
- `<external>/<name>` (type: path) → `<resolved_path>/backends/<name>`

## Resolution Rules

- Canonical IDs are authoritative.
- Bare names are normalized to `core/<name>` before resolver execution.
- `local` and external sources must be explicit in canonical IDs.
- No implicit fallback between `local`, external, and `core` is permitted.
- If a dependency references an external source item not allowed by `allow`, resolution must abort.
- If a backend ID references an external source item not allowed by `allow`, backend loading must abort.

## Validation Rules

### `sources.yaml`

- `sources` must be an array if present.
- Every `id` must be unique.
- Every `type` must be `git` or `path`.
- Reserved IDs must be rejected.
- Missing `allow` means deny-all.
- `type: git`: `url` is required and non-empty. `ref` sub-fields are mutually exclusive.
- `type: git`, `path` field: must be relative; `..` components and absolute paths are forbidden.
- `type: path`: `url` must not be specified. `path` is required and non-empty.
  The resolved directory must exist and must contain `features/` or `backends/`.
  The resolved real path must not equal the `local` source root.
- Unknown top-level fields in each source entry are reserved.

### `sources.lock.yaml`

- `resolved_commit` must be a full 40-character hex string.
- `fetched_at` must be in UTC RFC 3339 format.
- Lock entries for IDs not present in `sources.yaml` are ignored.

## Examples

### `type: git` — tracking a branch

```yaml
sources:
  - id: community
    type: git
    url: https://github.com/example/community-loadout.git
    ref:
      branch: main
    allow:
      features: "*"
      backends:
        - npm
        - uv
```

### `type: git` — pinned to a commit

```yaml
sources:
  - id: tools
    type: git
    url: https://github.com/example/tools-loadout.git
    ref:
      commit: abcdef1234567890abcdef1234567890abcdef12
    allow:
      features:
        - node
        - python
```

### `type: path` — local development source

```yaml
sources:
  - id: mylab
    type: path
    path: ~/projects/loadout-mylab
    allow:
      features:
        - mypkg
```
