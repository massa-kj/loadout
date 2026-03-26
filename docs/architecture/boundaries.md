# Architectural Boundaries

## Planner vs Executor

Planner is **pure**. Executor is **impure**.

Planner: reads inputs, computes classification, produces a Plan. No side effects.
Executor: executes actions in plan order, calls feature scripts and backends, commits state.

The boundary is non-negotiable:

* Planner must not execute, modify state, or call backends.
* Executor must not decide actions or re-classify operations.
* The plan produced by the planner is the executor's only instruction set.

See `specs/algorithms/planner.md` for the full contract.

## Plugin Isolation

Backend plugins must not:

* read strategy directly
* write state directly
* communicate with other plugins
* produce side effects outside their designated install/uninstall scope

Plugins receive only what core passes them explicitly.
They are execution adapters, not decision makers.

## State Authority Enforcement

```
planner:   reads state only
executor:  mutates state (via state module only)
features:  must not read or modify state directly
backends:  must not read or modify state directly
```

Violation examples:

* A feature script writing to `state.json` → forbidden
* The planner modifying state to "reserve" a resource → forbidden
* A backend reading state to decide which version to install → forbidden

## Feature Independence

Features must not depend on each other's internals.
Interaction between features is expressed only through the dependency model (`depends`, `requires`/`provides`).
There are no shared files, shared state slots, or shared install scripts between features.

## Dependency Model Constraints

Dependency declarations are for ordering only.

* No version constraints in `depends`
* No conditional dependencies
* No runtime-computed dependencies
* Cycles are forbidden
* Depth must remain shallow

If a required capability (`requires`) has no provider in the current profile, apply aborts.
If a dependency or backend references an external source item blocked by the source registry, apply aborts.

## Prohibited (Non-Negotiable)

The following are forbidden under any circumstance:

* Writing to state outside the state module
* Direct package manager invocation inside feature scripts (`brew`, `apt`, `scoop`, etc.)
* Logic or OS branching inside profiles
* Filesystem scanning during uninstall to discover removal targets
* Cross-feature resource ownership (two features tracking the same `fs.path`)
* Deep dependency graphs or runtime-computed dependencies
* Backend plugins reading strategy or writing state directly

Violations require architectural review — they cannot be justified by convenience.

## Breaking the Boundaries

If a boundary must be revisited, it requires:

1. An architectural discussion (not just a code change)
2. Update to the relevant spec
3. Update to this document

Boundaries are not suggestions. They exist to preserve long-term maintainability.
