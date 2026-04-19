# Direction (Under Consideration)

This document describes **possible** future directions. None of these are committed.
They are recorded so that design and contribution decisions can align with long-term thinking.

## Why Document Uncommitted Work?

* **Development** — Avoid over-investing in areas that may be replaced (e.g. deep shell optimizations if Rust is planned).
* **Design** — Keep current specs and architecture valid; direction docs do not override them.
* **Clarity** — Distinguish "current contract" (specs) from "exploration" (this document).

## Directions Under Consideration

### ~~Rust migration~~

**Implemented (Phase 1-6 complete)**

Core logic (resolver, planner, compiler, executor, state, backend dispatch, component execution) is now implemented in Rust.

* **Scope achieved:**
  - Single Rust binary (`loadout`)
  - Strongtyping for all data models (State, Profile, Strategy, Plan, etc.)
  - Cross-platform without shell/PowerShell duality for core logic
  - 16 crates with comprehensive test coverage (202 tests)
  - Plugin interfaces stabilized (JSON stdin/stdout protocol for backends)
  - Component scripts (`install.sh`/`uninstall.sh`) remain script-based
  - Platform bootstrap scripts (`platforms/linux/`, etc.) remain script-based
  - Backend plugins can be Rust-native (builtin) or script-based (community)

* **What remains:**
  - Optional: Embed builtin components into binary (currently loaded from filesystem)
  - Optional: External git source wiring (spec exists, execution path not yet implemented)
  - Ongoing: Migration of `mode: script` core components to `mode: declarative`

**For Rust implementation details, see:**
- `crates/` directory structure
- [docs/architecture/layers.md](../architecture/layers.md) — Repository Structure section
- [docs/rustdoc-map.md](../rustdoc-map.md) — docs ↔ Rust code mapping

### ~~Externalized profile / policy / component / backend~~

**Implemented (Phase 3-6)**

Profiles, policies, components, and backends can now be loaded from multiple sources:

* **Profiles/Policies:**
  - Platform defaults: `$XDG_CONFIG_HOME/loadout/profiles/` (Linux/WSL) or `%LOCALAPPDATA%\loadout\profiles\` (Windows)
  - Repository examples: `{repo}/profiles/` and `{repo}/policies/`
  
* **Components/Backends:**
  - `core` source: `{repo}/components/` and `{repo}/backends/`
  - `local` source: `$XDG_CONFIG_HOME/loadout/components/` and `backends/`
  - External sources: `$XDG_DATA_HOME/loadout/sources/<source_id>/` (schema defined, execution path deferred)

* **Canonical IDs:** All components and backends use `<source_id>/<name>` format (e.g., `core/git`, `local/mypkg`).
* **Source registry:** Manages discovery, allow-list validation, path resolution (see `crates/source-registry/`).

**Remaining work:**
- External git repository source execution path (spec exists in `docs/specs/data/sources.md`, not yet wired)

### ~~Declarative components~~ (implemented in Phase 4)

`mode: declarative` is now supported. Components declare resources in `component.yaml` and the executor
applies them without install/uninstall scripts.

See `docs/guides/components.md` for usage and `docs/specs/data/desired_resource_graph.md` for the resource schema.

Remaining open areas:
* Content-change detection for `fs` resources (requires `fs.source_hash` in state)
* Migration of existing `mode: script` core components to `mode: declarative`
* Runtime version constraints (range expressions, not just exact pins)

### ~~External git repository source~~

Allowing component/backend packs to be loaded from external git repositories.

* **Rationale** — Users can share and reuse component sets across machines without forking the repos.
* **Scope** — `loadout source add/update/remove` CLI; `sources.yaml` schema (spec exists); security model for untrusted code execution.
* **Status** — Deferred. `sources.yaml` schema and `source_registry` are in place; execution path not yet wired.
* **Prerequisite** — Security design for executing component scripts from external repos.

### ~~Bundle concept~~

A "bundle" is a named set of profiles/components that can be expanded together.

* **Rationale** — Simplify composing component sets (e.g. "work machine", "minimal server").
* **Scope** — `bundle.yaml` schema (example below); profile expansion; cycle guard.
* **Status** — Deferred. Orthogonal to current declarative component work.

```yaml
# Sketch: bundle.yaml
bundle:
  name: work
  extends: base
  components:
    - node
    - python
    - vscode
```
