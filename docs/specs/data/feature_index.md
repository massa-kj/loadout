# Feature Index Specification

## Purpose

The Feature Index is the parsed, merged, and validated representation of all available features.
It is produced by the Feature Index Builder and consumed by the Resolver and FeatureCompiler.

## Position in Pipeline

```
Source Registry + feature.yaml files
      ↓
  Feature Index Builder
      ↓
  Feature Index   ← this document
      ↓
  Resolver (dep fields only)
  FeatureCompiler (full spec)
```

## Schema

```json
{
  "schema_version": 1,
  "features": {
    "<canonical_feature_id>": {
      "spec_version": 1,
      "mode": "script | declarative",
      "description": "<human readable>",
      "source_dir": "<absolute path to feature directory>",
      "dep": {
        "depends": [ "<canonical_feature_id>", "..." ],
        "requires": [ { "name": "<capability>" } ],
        "provides": [ { "name": "<capability>" } ]
      },
      "spec": {
        "resources": [
          {
            "id": "<resource_id>",
            "kind": "package | runtime | fs",
            "<kind-specific fields>": "..."
          }
        ]
      }
    }
  }
}
```

`schema_version` must be `1` for this specification.

Feature keys are canonical IDs of the form `<source_id>/<name>` (e.g. `core/git`).

## Fields

### Top-level per feature

| Field | Required | Description |
|---|---|---|
| `spec_version` | yes | Feature schema version. Must be `1`. Builder aborts if absent or unknown. |
| `mode` | yes | `script` or `declarative`. Defaults to `script` if absent in feature.yaml but must be normalized by Builder. |
| `description` | no | Human-readable description string. |
| `source_dir` | yes | Absolute path to the feature directory (resolved by source registry). |
| `dep` | yes | Dependency fields (see below). May be empty object if no deps. |
| `spec` | conditional | Present for `declarative` mode features. May be absent for `script` mode features. |

### dep fields

Only these fields may be read by the Resolver. FeatureCompiler and Planner must not add fields here.

| Field | Description |
|---|---|
| `dep.depends[]` | Canonical feature IDs this feature depends on explicitly. |
| `dep.requires[]` | Capability names this feature requires from another feature. |
| `dep.provides[]` | Capability names this feature exposes to others. |

All `dep.depends` entries must be full canonical IDs after normalization.
The Feature Index Builder normalizes bare names to `<same_source>/<name>` before inclusion.

### spec fields (declarative mode)

`spec.resources` lists the resources that FeatureCompiler will expand into DesiredResourceGraph entries.

Resource format mirrors DesiredResourceGraph entries, except `desired_backend` is absent here
(it is resolved by FeatureCompiler using policy).

## Construction Rules

The Feature Index Builder:

1. Reads `feature.yaml` and optional `feature.<platform>.yaml` from each feature directory.
2. Merges platform-specific overrides on top of base (arrays are replaced, not appended).
3. Validates `spec_version` — aborts if absent or not `1`.
4. Normalizes `dep.depends` bare names to canonical IDs.
5. Sets `source_dir` from source registry resolution.
6. Aborts if a `declarative` mode feature has no `spec.resources`.

## Separation of Concerns

| Consumer | Permitted fields |
|---|---|
| Resolver | `dep.*` only |
| FeatureCompiler | `spec.*`, `mode`, `source_dir`, `dep.*` |
| Planner | does not read Feature Index directly |
| Executor | does not read Feature Index directly |

The Feature Index is not stored to disk between pipeline stages unless caching is explicitly implemented.

## Example

```json
{
  "schema_version": 1,
  "features": {
    "core/git": {
      "spec_version": 1,
      "mode": "script",
      "description": "Git version control system",
      "source_dir": "/home/user/loadout/features/git",
      "dep": {
        "depends": [],
        "requires": [ { "name": "package_manager" } ],
        "provides": []
      },
      "spec": {}
    },
    "core/neovim": {
      "spec_version": 1,
      "mode": "declarative",
      "description": "Neovim editor with config",
      "source_dir": "/home/user/loadout/features/neovim",
      "dep": {
        "depends": [ "core/git" ],
        "requires": [],
        "provides": []
      },
      "spec": {
        "resources": [
          {
            "id": "package:neovim",
            "kind": "package",
            "name": "neovim"
          },
          {
            "id": "fs:nvim-config",
            "kind": "fs",
            "path": "~/.config/nvim",
            "entry_type": "dir",
            "op": "link"
          }
        ]
      }
    }
  }
}
```
