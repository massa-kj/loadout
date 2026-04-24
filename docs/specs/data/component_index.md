# Component Index Specification

## Purpose

The Component Index is the parsed, merged, and validated representation of all available components.
It is produced by the Component Index Builder and consumed by the Resolver and ComponentCompiler.

## Document Boundary

**What this document defines (source of truth):**
- Component Index purpose and pipeline position
- Separation of concerns (Resolver reads `dep.*` only, ComponentCompiler reads full spec)
- Construction rules (merge order, normalization, validation)
- Platform resolution order (wsl → linux, etc.)
- Permitted/forbidden operations per consumer

**What Rust code defines (source of truth):**
- `ComponentIndex`, `ComponentMeta`, `DepSpec`, `ComponentSpec` types (`crates/model/src/component_index.rs`)
- Component Index Builder implementation (`crates/component-index/src/lib.rs`)
- Field types and deserialization logic

**Cross-reference:**
- Implementation: `crates/component-index/src/lib.rs`
- Data model: `crates/model/src/component_index.rs`
- For field-level structure documentation, see rustdoc: `cargo doc --open`

## Position in Pipeline

```
Source Registry + component.yaml files
      ↓
  Component Index Builder
      ↓
  Component Index   ← this document
      ↓
  Resolver (dep fields only)
  params-validator + params-materializer (spec resources, params)
  for-each-expander (expands for_each resources before compiler)
  ComponentCompiler (full spec)
```

## Schema

```json
{
  "schema_version": 1,
  "components": {
    "<canonical_component_id>": {
      "spec_version": 1,
      "mode": "script | declarative",
      "description": "<human readable>",
      "source_dir": "<absolute path to component directory>",
      "dep": {
        "depends": [ "<canonical_component_id>", "..." ],
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

Component keys are canonical IDs of the form `<source_id>/<name>` (e.g. `core/git`).

## Schema Overview

```json
{
  "schema_version": 1,
  "components": {
    "<canonical_component_id>": {
      "spec_version": 1,
      "mode": "script | declarative",
      "description": "<human readable>",
      "source_dir": "<absolute path to component directory>",
      "dep": {
        "depends": [ "<canonical_component_id>", "..." ],
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

For detailed field types, see `crates/model/src/component_index.rs` (rustdoc).

## Key Fields and Semantics

**`mode`**
- Values: `script` or `declarative`
- `script` mode: Component uses `install.sh` / `uninstall.sh` scripts
- `declarative` mode: Component declares resources in `spec.resources` (no scripts)

**`dep` fields (Resolver reads these only)**
- `dep.depends`: Explicit component dependencies (canonical IDs). **Hard**: if the named component is absent from the desired set, resolution aborts.
- `dep.requires`: Capability names this component should be ordered after. **Soft (ordering-only)**: if no provider is present in the desired set, the ordering constraint is silently omitted. The backend may be installed externally.
- `dep.provides`: Capability names this component exposes (matched against `dep.requires` of other components).

**`spec` fields (ComponentCompiler reads these)**
- Present for `declarative` mode components
- Contains `resources` array (mirrors DesiredResourceGraph format without `desired_backend`)
- `package` resources may include an optional `version` field (e.g. `"3.12"`). When absent, the package is installed unversioned (latest). When present, `LOADOUT_PACKAGE_VERSION` is injected and the planner treats any version change as requiring replacement.
- Each resource may include an optional `for_each` field — see [for_each Contract](#for_each-contract) below.

**`params_schema` (Component Index Builder validates this)**
- Declares parameter names, types, defaults, and required parameters for declarative components.
- Only `declarative` mode supports `params_schema`.
- The `params_schema` field is forbidden in platform override files (`component.<platform>.yaml`).
- Supported `type` values: `string`, `source`, `array` (see [Array Type](#array-type)).

## for_each Contract

`for_each` is an optional field on a resource in `spec.resources`. It instructs the
**for-each-expander** pipeline stage to expand that resource into multiple resources — one per
element of a params array — before compilation.

**Format:** `for_each: "params.<key>"` where `<key>` is a key in the component's `params_schema`.

**Invariants (safety rules enforced by the expander — violation aborts the pipeline):**

| Rule | Error |
|---|---|
| `for_each` key must exist in resolved params | `UnknownParam` |
| Referenced param must be an array value | `NotAnArray` |
| The resource `id` must contain `${item}` | `ItemNotInId` |
| Each array element must be a string | `NonStringItem` |
| All expanded resource `id`s must be unique within the component | `DuplicateId` |

**`${item}` placeholder:**
- Replaced with each array element in: `id`, `name`, `version`, `path`, `source` fields.
- Any field that does not reference `${item}` uses the same literal value for all expanded resources.

**Consumption:** `for_each` is consumed entirely by the for-each-expander.
No `for_each` field survives into the `ExpandedComponentSpec` passed to the compiler.
The compiler and executor are unaware of `for_each`.

**Referencing:** Only `params.*` paths are valid. Environment variables, file references, and
literal arrays are not supported.

## Array Type

When a `params_schema` property uses `type: array`, its value in the profile must be a YAML
sequence of strings. The `items.type` field specifies the element type (currently only `string`
is supported by the for-each-expander).

Array params cannot be interpolated directly into string fields via `${params.<key>}`.
They must be consumed via `for_each` on a resource.

Example `params_schema` with an array parameter:

```yaml
params_schema:
  properties:
    versions:
      type: array
      items:
        type: string
  required:
    - versions
```

Example profile usage:

```yaml
components:
  core/node:
    params:
      versions:
        - "18.20.0"
        - "22.17.1"
```

Example resource using `for_each`:

```yaml
resources:
  - id: "rt:node@${item}"
    kind: runtime
    name: node
    version: "${item}"
    for_each: "params.versions"
```

This expands to two resources: `rt:node@18.20.0` and `rt:node@22.17.1`.


**Platform resolution order:**
- WSL: `component.wsl.yaml` → `component.linux.yaml` → `component.yaml`
- Linux: `component.linux.yaml` → `component.yaml`
- Windows: `component.windows.yaml` → `component.yaml`

## Construction Rules

The Component Index Builder:

1. Reads `component.yaml` and optional `component.<platform>.yaml` from each component directory.
2. Merges platform-specific overrides on top of base (arrays are replaced, not appended).
3. Validates `spec_version` — aborts if absent or not `1`.
4. Normalizes `dep.depends` bare names to canonical IDs.
5. Sets `source_dir` from source registry resolution.
6. Aborts if a `declarative` mode component has no `spec.resources`.

## Separation of Concerns

| Consumer | Permitted fields |
|---|---|
| Resolver | `dep.*` only |
| ComponentCompiler | `spec.*`, `mode`, `source_dir`, `dep.*` |
| Planner | does not read Component Index directly |
| Executor | does not read Component Index directly |

The Component Index is not stored to disk between pipeline stages unless caching is explicitly implemented.

## Example

```json
{
  "schema_version": 1,
  "components": {
    "core/git": {
      "spec_version": 1,
      "mode": "script",
      "description": "Git version control system",
      "source_dir": "/home/user/loadout/components/git",
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
      "source_dir": "/home/user/loadout/components/neovim",
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
            "id": "package:python",
            "kind": "package",
            "name": "python",
            "version": "3.12"
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
