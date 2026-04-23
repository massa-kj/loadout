# Planner Specification

## Scope

This document defines the normative contract for the planner.

Covered: planner boundary, inputs, phases (diff/classification/decision),
decision table, plan data model, ordering rules, and determinism guarantee.

Not covered: state schema (see `specs/data/state.md`),
backend API (see `specs/api/backend.md`), executor behavior.

## Document Boundary

**What this document defines (source of truth):**
- Planner purity boundary (must not execute, modify state, call backends)
- Decision table (classification → operation mapping)
- Ordering rules (dependency order, reverse order for destroy)
- Determinism guarantee (identical inputs → identical plan)
- Plan semantics (what each operation means)
- Executor constraints (must not re-classify)

**What Rust code defines (source of truth):**
- Plan struct definition (`Plan`, `PlanAction`, `Operation` in `crates/model/src/plan.rs`)
- Classification enum and planner algorithm (`crates/planner/src/lib.rs`)
- Error types (`PlannerError` in `crates/planner/src/lib.rs`)

**Cross-reference:**
- Implementation: `crates/planner/src/lib.rs`
- Data model: `crates/model/src/plan.rs`
- For field-level structure documentation, see rustdoc: `cargo doc --open`

## Planner Boundary

Planner is **pure**.

Planner must: read inputs, compute classification, produce a plan.
Planner must NOT: execute install/uninstall, modify state, call backends, modify filesystem.

Executor must: execute actions in plan order, commit state atomically.
Executor must NOT: decide actions, re-classify, override plan decisions.

## Inputs

Planner operates only on:

1. `desired_resource_graph` — compiled desired resources, grouped by component (produced by ComponentCompiler)
2. `state` — current authoritative state
3. `resolved_component_order` — topologically sorted component identifiers from `resolver::resolve_extended`.
   This order includes **both** desired components (install order) and resolvable state-only components
   (components in state but not in desired, included for correct reverse destroy ordering).
   State-only components whose `component.yaml` has been deleted are excluded (silently skipped by
   `resolve_extended`). See `docs/specs/algorithms/resolver.md` for the extended resolution contract.

Component identifiers in `desired_resource_graph`, `state`, and planner output are canonical IDs of the form
`<source_id>/<name>`. Planner does not normalize bare names; normalization happens before planner input construction.

Planner must NOT receive `profile` or `strategy` directly.
Backend resolution (`desired_backend` per resource) must be completed by ComponentCompiler before planning.

Planner must NOT depend on current time, environment randomness, or live backend results.
Planner must NOT call backend observation API.
Environment detection via backend observation API is performed by the `plan` command layer,
not by the planner itself, and does not affect classification decisions.

## Planner Phases

```
Diff → Classification → Decision
```

**Diff** — structural comparison of `desired_resource_graph` vs `state`.
Determines: components added, removed, changed (resource set mismatch, backend mismatch).
Planner never reads strategy; `desired_backend` is already embedded in `desired_resource_graph`.

**Classification** — converts diff into normalized cases.
Each component is classified into exactly one of:
`create | destroy | replace | replace_backend | strengthen | noop | blocked`

| Class | Condition |
|---|---|
| `create` | In desired, not in state |
| `destroy` | In state, not in desired |
| `replace` | In both; any desired resource is incompatible with recorded state resource (kind change, fs path/entry_type/op change, tool identity/version-constraint change, or destructive semantics change) |
| `replace_backend` | In both; backend mismatch on any existing resource |
| `strengthen` | In both; all conditions below are satisfied: (1) every resource id recorded in state exists in desired, (2) every shared resource is compatible, (3) desired contains at least one resource id not present in state, (4) no backend mismatch, version mismatch, or blocked condition applies — **`strengthen` is never generated for `managed_script` components** |
| `noop` | In both; desired resources and state resources are identical and all compatible |
| `blocked` | Unknown resource kind (`kind` not in supported set) or invariant violation recorded in state |

Compatibility rules for shared resources:
* `package`: name and backend must match; version difference → `replace`
* `runtime`: name, version, and backend must match; any difference → `replace`
* `fs`: uses correspondence table for `entry_type`+`op` compatibility (see below); `path` and `source.resolved` must match; `source_fingerprint` compared when both present; any incompatibility → `replace`
* `tool`: `verify.identity` contract must match; if `verify.version.constraint` is declared, it must also match; any difference → `replace`

### fs Compatibility Correspondence Table

The planner compares desired `(entry_type, op)` against state `entry_type` using this table:

| Desired `entry_type` | Desired `op` | Compatible state `entry_type` values |
|---|---|---|
| `file` | `link` | `symlink` |
| `dir` | `link` | `symlink` (Unix), `symlink` or `junction` (Windows) |
| `file` | `copy` | `file` |
| `dir` | `copy` | `dir` |

Note: `junction` is a Windows-only NTFS reparse point. On Unix, `dir + link` only produces `symlink`.
The planner accepts both `symlink` and `junction` as compatible to avoid forced replace when
the same component is applied across platforms.

If desired `(entry_type, op)` is not in this table, classification is `replace`.
If state `entry_type` is not in the compatible set, classification is `replace`.

**Source comparison:** The planner compares `source.resolved` (desired) against `source.resolved` (state).
If state has no `source` recorded (legacy state), source comparison is skipped.

**Fingerprint comparison:** When both desired `source_fingerprint` and state `source_fingerprint`
are `Some`, the planner compares them. A mismatch → `replace`. If either is `None`, comparison is skipped
(no replace triggered). This enables noop detection for `copy` operations without forcing replace on
legacy state entries or non-eligible source types.

Fingerprints are produced by the materializer for `copy` sources according to `fingerprint_policy`:

- `all_copy` (default) — all source kinds are fingerprinted.
- `managed_only` — only `component_relative` sources.
- `none` — no fingerprints produced.

When fingerprinted, the hash method depends on `entry_type`:

- `entry_type: file` — SHA-256 of file byte content.
- `entry_type: dir` — deterministic tree hash (sorted `file:` / `dir:` records, SHA-256 of the joined list).

**Summary — compare targets by operation:**

| `op` | Compared fields |
|---|---|
| `link` | `path`, `source.resolved`, `entry_type + op` (via correspondence table) |
| `copy` | `path`, `source.resolved`, `entry_type + op` (via correspondence table), `source_fingerprint` (when both present) |

The distinction exists because `copy` resources benefit from fingerprint-based noop detection
(avoiding unnecessary re-copy when source content is unchanged), while `link` resources only
need structural field equality.

**`managed_script` components and `strengthen`:**
`strengthen` is never generated for `managed_script` components. If a `managed_script` component
has resources in desired that are not in state (e.g., a new `tool` resource was added), it is classified
as `replace` rather than `strengthen`. Reason: `managed_script` install/uninstall scripts operate at
component granularity and cannot add individual resources without executing the full install flow.

When in doubt between `strengthen` and `replace`, classify as `replace`.

**Decision** — maps classification to ordered action list using the decision table.
Must not call backends, modify state, or inspect filesystem.

## Decision Table

| Current State | Desired State | Action |
|---|---|---|
| ∅ | managed | `create` |
| managed | ∅ | `destroy` |
| managed(v1) | managed(v2, incompatible) | `replace` |
| managed(A) | managed(B, backend differs) | `replace_backend` |
| managed(subset) | managed(superset, compatible) | `strengthen` |
| managed | managed (identical) | `noop` |
| managed | managed (blocked kind) | `blocked` |

Table must be deterministic, total (every classification maps to an action), and explicit (no hidden fallbacks).

`strengthen` action details must include `add_resources` — the list of resources to install:

```json
{ "component": "core/git", "operation": "strengthen",
  "details": { "add_resources": [ { "kind": "fs", "id": "fs:gitconfig" } ] } }
```

The executor reads `details.add_resources` to determine what to install without re-reading `desired_resource_graph` directly.

## Plan Data Model

```json
{
  "actions": [
    { "component": "core/git", "operation": "create" },
    { "component": "core/node", "operation": "replace", "details": { "from_version": "18", "to_version": "20" } },
    { "component": "core/git", "operation": "strengthen", "details": { "add_resources": [ { "kind": "fs", "id": "fs:gitconfig" } ] } }
  ],
  "noops": [ { "component": "core/bash" } ],
  "blocked": [ { "component": "local/legacy", "reason": "unknown resource kind: registry" } ],
  "summary": { "create": 1, "replace": 1, "strengthen": 1, "destroy": 0, "blocked": 1 }
}
```

* `actions` — ordered list of operations to execute
* `noops` — components already correct; not in `actions`
* `blocked` — components skipped due to planner-level classification
* `summary` — counts per operation type

## Ordering Rules

1. `destroy` operations in reverse dependency order (dependents before their dependencies).
   Order is derived from the position of each component in `resolved_component_order`, which
   includes state-only components via `resolve_extended`. Components absent from the order
   (their `component.yaml` was deleted) fall back to alphabetical descending as a best-effort
   tie-breaker.
2. `replace` operations: uninstall first, then install
3. `create` operations in dependency order
4. `replace_backend` treated as `replace`

Ordering must be derived from resolver output. Must not rely on component script order.
Source location must not affect ordering; only canonical dependency edges may do so.

## Plan Command

`loadout plan` must: call planner, print plan, never execute actions, never modify state.

## Apply Interaction

`loadout apply` must: run planner → report blocked components → pass plan to executor → commit state.
Blocked components are skipped; non-blocked components continue.
Apply must not re-run classification inside the executor.

## Determinism Guarantee

Given identical `desired_resource_graph`, `state`, `resolved_component_order`, and `inventory`:
the planner must produce an identical plan. No randomness permitted.
