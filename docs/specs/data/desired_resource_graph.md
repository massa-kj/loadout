# DesiredResourceGraph Specification

## Purpose

DesiredResourceGraph is the structured representation of all resources that **should** exist
after a successful apply, grouped by feature.

It is produced by FeatureCompiler and consumed exclusively by Planner.

## Position in Pipeline

```
Feature Index + Policy
      ↓
  FeatureCompiler
      ↓
  DesiredResourceGraph   ← this document
      ↓
  Planner
```

## Schema

```json
{
  "schema_version": 1,
  "features": {
    "<canonical_feature_id>": {
      "resources": [
        {
          "id": "<resource_id>",
          "kind": "package | runtime | fs",
          "desired_backend": "<backend_id>",
          "<kind-specific fields>": "..."
        }
      ]
    }
  }
}
```

`schema_version` must be `1` for this specification.

Feature keys are canonical IDs of the form `<source_id>/<name>` (e.g. `core/git`).

## Resource ID

`id` is a stable, human-readable identifier unique within a feature's resource list.
Format: `<kind>:<name>` (e.g. `package:git`, `fs:gitconfig`, `runtime:node`).

Resource IDs are used by the planner for diff and by the executor for state recording.
Changing a resource `id` is a breaking change requiring state migration.

## Resource Kinds

### package

```json
{
  "id": "package:git",
  "kind": "package",
  "name": "git",
  "desired_backend": "brew"
}
```

| Field | Required | Description |
|---|---|---|
| `id` | yes | stable identifier |
| `kind` | yes | `package` |
| `name` | yes | package name as known to the backend |
| `desired_backend` | yes | resolved backend identifier |

### runtime

```json
{
  "id": "runtime:node",
  "kind": "runtime",
  "name": "node",
  "version": "20",
  "desired_backend": "mise"
}
```

| Field | Required | Description |
|---|---|---|
| `id` | yes | stable identifier |
| `kind` | yes | `runtime` |
| `name` | yes | runtime name |
| `version` | yes | version string (exact or constraint) |
| `desired_backend` | yes | resolved backend identifier |

### fs

```json
{
  "id": "fs:gitconfig",
  "kind": "fs",
  "source": "files/gitconfig",
  "path": "~/.gitconfig",
  "entry_type": "file",
  "op": "link"
}
```

| Field | Required | Description |
|---|---|---|
| `id` | yes | stable identifier |
| `kind` | yes | `fs` |
| `source` | no | path to the source file/dir, relative to the feature directory. Defaults to `files/<basename(path)>` if omitted. |
| `path` | yes | absolute or `~`-relative target path |
| `entry_type` | yes | `file` or `dir` |
| `op` | yes | `link` (symlink) or `copy` |

`fs` resources have no `desired_backend` (backend-independent).

## Backend Resolution

`desired_backend` is resolved by FeatureCompiler using policy before producing this graph.
The Planner must NOT re-resolve backends. The value in `desired_backend` is authoritative.

## Immutability

DesiredResourceGraph is immutable once produced by FeatureCompiler.
Neither Planner nor Executor may modify it.

## Unknown Resource Kinds

If a resource `kind` is not in the supported set (`package`, `runtime`, `fs`),
the Planner must classify the owning feature as `blocked`.

## Compatibility Rules (used by Planner)

| Kind | Compatible if |
|---|---|
| `package` | `name` and `desired_backend` match |
| `runtime` | `name`, `version`, and `desired_backend` all match |
| `fs` | `path`, `source`, `entry_type`, and `op` all match |

Any field difference in the above set implies incompatibility → `replace`.

## Example

```json
{
  "schema_version": 1,
  "features": {
    "core/git": {
      "resources": [
        {
          "id": "package:git",
          "kind": "package",
          "name": "git",
          "desired_backend": "brew"
        },
        {
          "id": "fs:gitconfig",
          "kind": "fs",
          "path": "~/.gitconfig",
          "entry_type": "file",
          "op": "link"
        }
      ]
    },
    "core/node": {
      "resources": [
        {
          "id": "runtime:node",
          "kind": "runtime",
          "name": "node",
          "version": "20",
          "desired_backend": "mise"
        }
      ]
    }
  }
}
```
