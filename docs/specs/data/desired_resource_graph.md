# DesiredResourceGraph Specification

## Purpose

DesiredResourceGraph is the structured representation of all resources that **should** exist
after a successful apply, grouped by component.

It is produced by ComponentCompiler and consumed exclusively by Planner.

## Document Boundary

**What this document defines (source of truth):**
- DesiredResourceGraph purpose and pipeline position
- Backend resolution authority (ComponentCompiler resolves, Planner must not re-resolve)
- Immutability constraint (Planner/Executor must not modify)
- Compatibility rules (used by Planner for classification)
- Unknown kind handling (Planner must block)
- Resource ID stability requirement (breaking change semantics)

**What Rust code defines (source of truth):**
- `DesiredResourceGraph` struct and types (`crates/model/src/desired_resource_graph.rs`)
- `DesiredResourceKind` enum variants (`Package`, `Runtime`, `Fs`)
- Field types and deserialization logic

**Cross-reference:**
- Implementation: `crates/model/src/desired_resource_graph.rs`
- For field-level structure documentation, see rustdoc: `cargo doc --open`

## Position in Pipeline

```
Component Index + strategy
      ↓
  ComponentCompiler
      ↓
  DesiredResourceGraph   ← this document
      ↓
  Planner
```

## Schema

```json
{
  "schema_version": 1,
  "components": {
    "<canonical_component_id>": {
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

Component keys are canonical IDs of the form `<source_id>/<name>` (e.g. `core/git`).

## Resource ID

`id` is a stable, human-readable identifier unique within a component's resource list.
Format: `<kind>:<name>` (e.g. `package:git`, `fs:gitconfig`, `runtime:node`).

Resource IDs are used by the planner for diff and by the executor for state recording.
Changing a resource `id` is a breaking change requiring state migration.

## Resource Kinds

DesiredResourceGraph supports three resource kinds: `package`, `runtime`, and `fs`.

For detailed field definitions and types, see `crates/model/src/desired_resource_graph.rs` (rustdoc).

### package

```json
{
  "id": "package:git",
  "kind": "package",
  "name": "git",
  "desired_backend": "brew"
}
```

**Meaning:**
- `desired_backend` is resolved by ComponentCompiler using strategy (source of truth)
- Planner uses this value for backend-mismatch detection

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

**Meaning:**
- `version` is always required (unlike packages)
- `desired_backend` is resolved by ComponentCompiler

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

**Meaning:**
- No `desired_backend` (fs operations are backend-independent)
- `source` defaults to `files/<basename(path)>` if omitted
- `op` values: `link` (symlink) or `copy`

## Backend Resolution

`desired_backend` is resolved by ComponentCompiler using strategy before producing this graph.
The Planner must NOT re-resolve backends. The value in `desired_backend` is authoritative.

## Immutability

DesiredResourceGraph is immutable once produced by ComponentCompiler.
Neither Planner nor Executor may modify it.

## Unknown Resource Kinds

If a resource `kind` is not in the supported set (`package`, `runtime`, `fs`),
the Planner must classify the owning component as `blocked`.

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
  "components": {
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
