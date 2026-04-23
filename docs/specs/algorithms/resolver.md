# Resolver Specification

## Scope

This document defines the input/output contract and algorithm for the resolver.

Covered: inputs, dependency model, graph construction, cycle detection, output contract, determinism.

Not covered: execution, state mutations, planner decision logic, component metadata format (see `specs/data/component_index.md`).

## Document Boundary

**What this document defines (source of truth):**
- Resolver inputs/outputs (Component Index → ResolvedComponentOrder)
- Dependency model semantics (depends/provides/requires)
- Graph construction algorithm (explicit edges + capability resolution)
- Cycle detection requirement (DAG enforcement)
- Forbidden operations (must not read resources, must not scan filesystem)
- Determinism guarantee (same inputs → same order)

**What Rust code defines (source of truth):**
- `ResolvedComponentOrder` type (`crates/model/src/lib.rs`)
- `resolve` and `resolve_extended` function signatures and algorithms (`crates/resolver/src/lib.rs`)
- Error types (`ResolverError` in `crates/resolver/src/lib.rs`)

**Cross-reference:**
- Implementation: `crates/resolver/src/lib.rs`
- Component Index format: `docs/specs/data/component_index.md`
- For field-level structure documentation, see rustdoc: `cargo doc --open`

## Inputs

The resolver receives:

* `component_index` — parsed Component Index produced by Component Index Builder (see `specs/data/component_index.md`)
* `desired_components` — list of canonical component identifiers from the resolved profile

All resolver inputs must already be normalized to canonical IDs of the form `<source_id>/<name>`.
Bare names are normalized upstream to `core/<name>` before resolver execution.

**Permitted reads:**
The resolver reads **only** `dep` fields from the Component Index: `depends`, `provides`, and `requires`.

**Forbidden reads:**
The resolver must NOT read `resources` fields. Resource definitions are the exclusive domain of ComponentCompiler.
The resolver must NOT scan the filesystem directly for component metadata.

## Metadata Sources

Component metadata is supplied via the Component Index. The resolver does not read files directly.

For the Component Index schema and construction rules, see `docs/specs/data/component_index.md`.

## Dependency Model

**`depends`** — explicit component dependency.
Use when the dependency is on a specific named component.

```yaml
depends:
  - git
```

Normalization rules for `dep.depends`:

* bare name `git` in `core/neovim` → `core/git`
* bare name `helper` in `local/mycomponent` → `local/helper`
* cross-source dependency must be explicit, e.g. `core/git` or `community/node`

**`provides` / `requires`** — capability-based ordering hint.
Use when a component should be installed after any provider of an abstract capability, if one is present.

```yaml
# provider
provides:
  - name: package_manager

# consumer
requires:
  - name: package_manager
```

The resolver finds all components in the desired set that declare the matching `dep.provides` entry,
and injects them as implicit ordering dependencies (before the requiring component).

`dep.requires` is **soft**: if no provider is present in the desired set, the ordering constraint is
silently omitted. This allows a component to use an externally installed backend without declaring it as
a loadout-managed component.

`dep.depends`, by contrast, is **hard**: the named component must be in the desired set, or resolution aborts.

## Graph Construction

1. Read dep fields from the Component Index for all desired components.
2. Normalize `dep.depends` entries to canonical IDs and build explicit dependency edges.
3. For each component with `dep.requires`, find matching `dep.provides` among desired components.
   Inject found providers as implicit ordering edges. If no provider is present, skip silently.
4. If an explicit dependency declared in `dep.depends` is not present in the desired set, abort with an error.

Source allow-list validation: External and `local` components are subject to source allow-list validation.
If the component itself or any declared explicit dependency is not allowed by the source registry,
resolution must abort. This applies to both `type: git` and `type: path` external sources.

## Cycle Detection

The resolver performs depth-first search with an in-stack marker.
If a component is encountered while it is already in the DFS stack, a cycle is detected and execution aborts.

Cycles are forbidden. The dependency graph must be a DAG.

## Output Contract

The resolver outputs a `ResolvedComponentOrder`: a topologically sorted list of canonical component identifiers.

* Install order: dependencies appear before dependents.
* Uninstall order: reverse of install order (managed by planner/executor).
* If a dependency declared in `dep.depends` is not present in the desired set, execution aborts.
* Resolver output is deterministic for the same canonical input set and Component Index.

## Extended Resolution (`resolve_extended`)

The pipeline also calls `resolve_extended` to produce a full order that includes both desired
components and state-only components (components recorded in state but not in the desired set,
i.e. about to be destroyed).

`resolve_extended` accepts:
- `desired_ids` — desired component IDs (same hard rules as `resolve`)
- `state_extras` — state-only component IDs not present in the desired set

Soft-handling rules for `state_extras`:
- Components not found in the Component Index (their `component.yaml` was deleted) are silently skipped.
- Dependencies of state-extra components that are absent from the combined set are silently omitted (not a `MissingDependency` error).

The returned full order is used by the planner to compute correct reverse destroy ordering when
both a component and its dependency are removed from the profile simultaneously.

The pipeline derives two orders from `resolve_extended`:
- `full_order` (desired + state-only) → passed to the planner
- `order` (desired-only, filtered from `full_order`) → passed to the compiler and executor

`resolve_extended` never errors on state-extra components; all hard errors apply only to desired components,
with the same rules as `resolve`.
