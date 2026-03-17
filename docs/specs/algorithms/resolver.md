# Resolver Specification

## Scope

This document defines the input/output contract and algorithm for the resolver.

Covered: inputs, metadata sources, dependency model, graph construction, cycle detection, output contract.

Not covered: execution, state mutations, planner decision logic.

## Inputs

The resolver receives:

* `feature_index` — parsed Feature Index produced by Feature Index Builder
* `desired_features` — list of canonical feature identifiers from the resolved profile

All resolver inputs must already be normalized to canonical IDs of the form `<source_id>/<name>`.
Bare names are normalized upstream to `core/<name>` before resolver execution.

The resolver reads only `dep` fields from the Feature Index:
`depends`, `provides`, and `requires`.

The resolver must NOT read `resources` fields. Resource definitions are the exclusive domain of FeatureCompiler.
The resolver must NOT scan the filesystem directly for feature metadata.

## Metadata Sources

Feature metadata is supplied via the Feature Index. The resolver does not read files directly.

For reference, the Feature Index Builder reads the following files from each feature directory
(determined via source registry):

1. `feature.yaml` — base metadata (always present)
2. `feature.<platform>.yaml` — platform-specific overrides (merged if present)

Platform resolution order:

* WSL: `feature.wsl.yaml` → `feature.linux.yaml` → none
* Linux: `feature.linux.yaml` → none
* Windows: `feature.windows.yaml` → none

Fields exposed in the Feature Index that the resolver may read (dep fields only):

* `dep.depends[]` — list of explicit feature identifiers
* `dep.provides[].name` — capability names this feature exposes
* `dep.requires[].name` — capability names this feature depends on

## Dependency Model

**`depends`** — explicit feature dependency.
Use when the dependency is on a specific named feature.

```yaml
depends:
  - git
```

Normalization rules for `dep.depends`:

* bare name `git` in `core/neovim` → `core/git`
* bare name `helper` in `user/myfeat` → `user/helper`
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

Source allow-list validation: External and `user` features are subject to source allow-list validation.
If the feature itself or any declared explicit dependency is not allowed by the source registry,
resolution must abort.

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
