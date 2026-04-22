# Strategy Specification

## Scope

This document defines the normative contract for the `strategy:` section of a config file.

Covered: schema, backend resolution model, specificity rules, validation rules, and diagnostics contract.

Not covered: how backends work (see `specs/api/backend.md`),
or how to write strategies in practice (see `guides/usage.md`).

## Schema

Strategy is embedded as the optional `strategy:` section within a `config.yaml` file.
When the `strategy:` section is absent, `Strategy::default()` is used (no rules, backup enabled).

```yaml
# configs/linux.yaml
profile:
  components:
    ...

strategy:      # optional section
  groups:
    <group_name>:
      <kind>:           # "package" or "runtime"
        - <resource_name>

  rules:
    - match:
        kind: package | runtime   # required if component is present
        name: <resource_name>     # exact resource name (optional)
        component: <component_id> # canonical component ID (optional; requires kind)
        group: <group_name>       # group membership filter (optional; requires kind)
      use: <backend_id>

  fs:
    backup: true | false
    backup_dir: "<path>"
    fingerprint_policy: managed_only | all_copy | none   # default: all_copy
```

### `groups`

A group is a named, static set of resource names, keyed by kind.

```yaml
groups:
  npm_global:
    package:
      - eslint
      - prettier
```

Groups are pure enumeration. Conditions, version constraints, globs, and regexes are forbidden.
If filtering logic is needed in the future, it must be introduced as a separate concept (e.g. `predicates`).

### `rules`

Each rule has a `match:` selector and a `use:` backend ID.

Rules are evaluated against every resource. The **most specific match** wins.
If multiple rules have equal specificity, the **last rule wins** (tie-break by index).

### `match` Fields

| Field       | Type   | Description |
|-------------|--------|-------------|
| `kind`      | string | `"package"` or `"runtime"` only. `fs` / `tool` are not backend-resolved and are forbidden here. |
| `name`      | string | Exact resource name (package name or runtime name). |
| `component` | string | Canonical component ID (e.g. `core/cli-tools`). Requires `kind`. |
| `group`     | string | Group name defined in `groups:`. Requires `kind`. |

All fields are optional, but `component` requires `kind` to be present (see Validation Rules).

### `fingerprint_policy`

Controls which `copy` sources the materializer fingerprints for noop detection.

| Value | Behaviour |
|---|---|
| `all_copy` (default) | Fingerprint every source kind when `op = copy`. |
| `managed_only` | Only `component_relative` sources are fingerprinted. |
| `none` | Disable fingerprinting entirely; `copy` always produces `replace`. |

## File Location

Strategy is a section within the config file selected by `--config` / `-c`.
No standalone strategy file is used.

Config file location:

* Linux/WSL: `$XDG_CONFIG_HOME/loadout/configs/`
* Linux/WSL fallback: `~/.config/loadout/configs/`
* Windows: `%APPDATA%\loadout\configs\`

See `specs/data/profile.md` for config resolution rules.

## Backend Resolution Model

Strategy determines which backend is used for each resource installation.

Backend identifiers accept the same two forms as component identifiers:

* bare backend name, e.g. `brew`
* canonical backend ID, e.g. `core/brew`, `local/custompkg`

Bare backend names are normalized to `core/<name>` before backend loading.
`local` and external source backends must be explicit.

Resolution applies per resource:

* `package` resources — resolved from `rules` where `match.kind = "package"`
* `runtime` resources — resolved from `rules` where `match.kind = "runtime"`
* `fs` resources — no backend; handled by the fs module directly

## Specificity Decision Table

Each matching rule has a **specificity vector** `(has_component, has_kind, has_name, has_group)`.
Comparison is lexicographic (highest priority left). Not additive scoring.

| `match` fields                                  | Specificity vector |
|-------------------------------------------------|--------------------|
| _(no selector)_                                 | (0, 0, 0, 0)       |
| `kind`                                          | (0, 1, 0, 0)       |
| `kind` + `group`                                | (0, 1, 0, 1)       |
| `kind` + `name`                                 | (0, 1, 1, 0)       |
| `component` + `kind`                            | (1, 1, 0, 0)       |
| `component` + `kind` + `group`                  | (1, 1, 0, 1)       |
| `component` + `kind` + `name`                   | (1, 1, 1, 0)       |

**Key invariant**: `component` always outranks `name`, regardless of other axes.
`(1, 1, 0, 0) > (0, 1, 1, 0)` — a component+kind rule always beats a kind+name rule.

## Resolution Order

```
most specific rule wins (lexicographic specificity vector)
  →  tie-break: last rule wins (higher index)
  →  no match: execution aborts (NoBackend error)
```

## Validation Rules

1. `match.kind` must be `"package"` or `"runtime"`. Using `"fs"`, `"tool"`, or other values is forbidden.
2. `match.component` requires `match.kind` to be present. A component-only rule is forbidden.
3. `match.group` requires `match.kind` to be present. A group-only rule is forbidden. (Without `kind`, specificity is `(0,0,0,1)` which is lower than a kind-only rule `(0,1,0,0)`, so the rule can never be selected.)
4. `rules[*].use` must be a non-empty string.
5. `match.group` must reference a name defined in `strategy.groups`. Forward references are forbidden.
6. If `match.kind = "runtime"` and `match.group` is set, the referenced group must define a `"runtime"` key. Referencing a package-only group from a runtime rule is forbidden.

## Diagnostics Contract

For each resource, the following information must be traceable (e.g. via `--verbose` or `doctor`).
Diagnostics are a responsibility of the compiler layer; the planner must not be involved.

Output items:
- Evaluated rules count
- Matched rules (rule index and match content)
- Selected rule index
- Selected reason (result of specificity vector comparison)
- Competing rules (if tie-break occurred)
- No-match reason (if no rule matched)

Output example:

```text
resource: package:eslint (component=local/node-tools)
matched:
  [2] kind=package, group=npm_global -> core/npm
  [5] component=local/node-tools, kind=package, name=eslint -> local/custom-npm
selected:
  [5] because component+kind+name (1,1,1,0) outranks kind+group (0,1,0,1)
```
