# Resolver Specification

## Scope

This document defines the input/output contract and algorithm for the resolver.

Covered: inputs, dependency model, graph construction, cycle detection, output contract, determinism.

Not covered: execution, state mutations, planner decision logic, feature metadata format (see `specs/data/feature_index.md`).

## Document Boundary

**What this document defines (source of truth):**
- Resolver inputs/outputs (Feature Index → ResolvedFeatureOrder)
- Dependency model semantics (depends/provides/requires)
- Graph construction algorithm (explicit edges + capability resolution)
- Cycle detection requirement (DAG enforcement)
- Forbidden operations (must not read resources, must not scan filesystem)
- Determinism guarantee (same inputs → same order)

**What Rust code defines (source of truth):**
- `ResolvedFeatureOrder` type (`crates/model/src/lib.rs`)
- Resolver function signature and algorithm (`crates/resolver/src/lib.rs`)
- Error types (`ResolverError` in `crates/resolver/src/lib.rs`)

**Cross-reference:**
- Implementation: `crates/resolver/src/lib.rs`
- Feature Index format: `docs/specs/data/feature_index.md`
- For field-level structure documentation, see rustdoc: `cargo doc --open`

## Inputs

The resolver receives:

* `feature_index` — parsed Feature Index produced by Feature Index Builder (see `specs/data/feature_index.md`)
* `desired_features` — list of canonical feature identifiers from the resolved profile

All resolver inputs must already be normalized to canonical IDs of the form `<source_id>/<name>`.
Bare names are normalized upstream to `core/<name>` before resolver execution.

**Permitted reads:**
The resolver reads **only** `dep` fields from the Feature Index: `depends`, `provides`, and `requires`.

**Forbidden reads:**
The resolver must NOT read `resources` fields. Resource definitions are the exclusive domain of FeatureCompiler.
The resolver must NOT scan the filesystem directly for feature metadata.

## Metadata Sources

Feature metadata is supplied via the Feature Index. The resolver does not read files directly.

For the Feature Index schema and construction rules, see `docs/specs/data/feature_index.md`.

## Dependency Model

**`depends`** — explicit feature dependency.
Use when the dependency is on a specific named feature.

```yaml
depends:
  - git
```

Normalization rules for `dep.depends`:

* bare name `git` in `core/neovim` → `core/git`
* bare name `helper` in `local/myfeat` → `local/helper`
* cross-source dependency must be explicit, e.g. `core/git` or `community/node`

**`provides` / `requires`** — capability-based dependency.
Use when a feature needs any provider of an abstract capability.

```yaml
# provider
provides:
  - name: package_manager

# consumer
requires:
  - name: package_manager
```

The resolver finds all features in the desired set that declare the matching `dep.provides` entry,
and injects them as implicit ordering dependencies of the requiring feature.

## Graph Construction

1. Read dep fields from the Feature Index for all desired features.
2. Normalize `dep.depends` entries to canonical IDs and build explicit dependency edges.
3. For each feature with `dep.requires`, find matching `dep.provides` among desired features.
   Inject found providers as implicit `depends` edges.
4. If a required capability has no provider in the desired set, abort with an error.
5. If an explicit dependency is not present in the desired set, abort with an error.

Source allow-list validation: External and `local` features are subject to source allow-list validation.
If the feature itself or any declared explicit dependency is not allowed by the source registry,
resolution must abort. This applies to both `type: git` and `type: path` external sources.

## Cycle Detection

The resolver performs depth-first search with an in-stack marker.
If a feature is encountered while it is already in the DFS stack, a cycle is detected and execution aborts.

Cycles are forbidden. The dependency graph must be a DAG.

## Output Contract

The resolver outputs a `ResolvedFeatureOrder`: a topologically sorted list of canonical feature identifiers.

* Install order: dependencies appear before dependents.
* Uninstall order: reverse of install order (managed by planner/executor).
* If a dependency declared in `dep.depends` is not present in the desired set, execution aborts.
* Resolver output is deterministic for the same canonical input set and Feature Index.
