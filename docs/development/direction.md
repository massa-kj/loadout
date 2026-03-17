# Direction (Under Consideration)

This document describes **possible** future directions. None of these are committed.
They are recorded so that design and contribution decisions can align with long-term thinking.

## Why Document Uncommitted Work?

* **Development** — Avoid over-investing in areas that may be replaced (e.g. deep shell optimizations if Rust is planned).
* **Design** — Keep current specs and architecture valid; direction docs do not override them.
* **Clarity** — Distinguish "current contract" (specs) from "exploration" (this document).

## Directions Under Consideration

### Rust migration

Reimplementing core (resolver, planner, executor, state, backend dispatch) in Rust.

* **Rationale** — Performance, single binary, stronger typing, cross-platform without shell/PowerShell duality.
* **Scope** — Core logic and possibly CLI; features and backends may remain script-based or become pluggable.
* **Status** — Exploratory. No timeline. Current shell/PowerShell implementation remains the reference.

### ~~Externalized profile / policy / feature / backend~~

Allowing profile, policy, feature definitions, and backends to be loaded from outside the repository (e.g. config directories, plugin paths, remote sources).

* **Rationale** — Users can maintain private profiles or third-party features without forking; separation of "loadout engine" vs "my config".
* **Scope** — Load paths, discovery, validation; compatibility with current in-repo layout.
* **Status** — Exploratory. Contract (profile/policy/state schema) would remain; only the source of files would change.

### ~~Declarative features~~ (implemented in Phase 4)

`mode: declarative` is now supported. Features declare resources in `feature.yaml` and the executor
applies them without install/uninstall scripts.

See `docs/guides/features.md` for usage and `docs/specs/data/desired_resource_graph.md` for the resource schema.

Remaining open areas:
* Content-change detection for `fs` resources (requires `fs.source_hash` in state)
* Migration of existing `mode: script` core features to `mode: declarative`
* Runtime version constraints (range expressions, not just exact pins)

### External git repository source

Allowing feature/backend packs to be loaded from external git repositories.

* **Rationale** — Users can share and reuse feature sets across machines without forking the repos.
* **Scope** — `loadout source add/update/remove` CLI; `sources.yaml` schema (spec exists); security model for untrusted code execution.
* **Status** — Deferred. `sources.yaml` schema and `source_registry` are in place; execution path not yet wired.
* **Prerequisite** — Security design for executing feature scripts from external repos.

### Bundle concept

A "bundle" is a named set of profiles/features that can be expanded together.

* **Rationale** — Simplify composing feature sets (e.g. "work machine", "minimal server").
* **Scope** — `bundle.yaml` schema (example below); profile expansion; cycle guard.
* **Status** — Deferred. Orthogonal to current declarative feature work.

```yaml
# Sketch: bundle.yaml
bundle:
  name: work
  extends: base
  features:
    - node
    - python
    - vscode
```
