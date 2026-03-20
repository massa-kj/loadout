# Feature Index Specification

## Purpose

The Feature Index is the parsed, merged, and validated representation of all available features.
It is produced by the Feature Index Builder and consumed by the Resolver and FeatureCompiler.

## Document Boundary

**What this document defines (source of truth):**
- Feature Index purpose and pipeline position
- Separation of concerns (Resolver reads `dep.*` only, FeatureCompiler reads full spec)
- Construction rules (merge order, normalization, validation)
- Platform resolution order (wsl → linux, etc.)
- Permitted/forbidden operations per consumer

**What Rust code defines (source of truth):**
- `FeatureIndex`, `FeatureMeta`, `DepSpec`, `FeatureSpec` types (`crates/model/src/feature_index.rs`)
- Feature Index Builder implementation (`crates/feature-index/src/lib.rs`)
- Field types and deserialization logic

**Cross-reference:**
- Implementation: `crates/feature-index/src/lib.rs`
- Data model: `crates/model/src/feature_index.rs`
- For field-level structure documentation, see rustdoc: `cargo doc --open`

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

## Schema Overview

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
        "resources": [ ... ]
      }
    }
  }
}
```

For detailed field types, see `crates/model/src/feature_index.rs` (rustdoc).

## Key Fields and Semantics

**`mode`**
- Values: `script` or `declarative`
- `script` mode: Feature uses `install.sh` / `uninstall.sh` scripts
- `declarative` mode: Feature declares resources in `spec.resources` (no scripts)

**`dep` fields (Resolver reads these only)**
- `dep.depends`: Explicit feature dependencies (canonical IDs)
- `dep.requires`: Capability names this feature needs (abstract dependencies)
- `dep.provides`: Capability names this feature exposes (abstract provision)

**`spec` fields (FeatureCompiler reads these)**
- Present for `declarative` mode features
- Contains `resources` array (mirrors DesiredResourceGraph format without `desired_backend`)

**Platform resolution order:**
- WSL: `feature.wsl.yaml` → `feature.linux.yaml` → `feature.yaml`
- Linux: `feature.linux.yaml` → `feature.yaml`
- Windows: `feature.windows.yaml` → `feature.yaml`

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
