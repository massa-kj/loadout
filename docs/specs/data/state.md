# State Specification

## Scope

This document defines the normative contract for state.

Covered: schema, resource kinds, identity rules, invariants, state transition rules,
atomic commit rules, safety rules, and compatibility.

Not covered: profile semantics,strategy semantics, planner rules, backend selection.

## Document Boundary

**What this document defines (source of truth):**
- JSON schema semantics (what fields mean)
- State authority rules (who may read/write state)
- Safety constraints (what must/must not be in state)
- Identity rules and invariants (uniqueness, path constraints)
- State transition rules (atomic commit, corruption handling)
- Migration semantics (v2 → v3)

**What Rust code defines (source of truth):**
- Struct definitions and field types (`State`, `ComponentState`, `Resource`, `ResourceKind` in `crates/model/src/state.rs`)
- Parsing and validation logic (`crates/state/src/lib.rs`)
- JSON serialization format (via serde)

**Cross-reference:**
- Implementation: `crates/model/src/state.rs`, `crates/state/src/lib.rs`
- For field-level structure documentation, see rustdoc: `cargo doc --open`

## Purpose

State is the **single authority** for:

1. What resources were created by loadout execution.
2. What resources are safe to remove.
3. What backend must be used for deterministic removal.

State contains effects only. No desired state. No strategy No dependency graphs.

## File Location

Authoritative file path is platform-defined:

* Linux/WSL: `$XDG_STATE_HOME/loadout/state.json`
* Linux/WSL fallback: `~/.local/state/loadout/state.json`
* Windows: `%LOCALAPPDATA%\loadout\state.json`

The state path is not directly overridable by `LOADOUT_STATE_FILE` or `LOADOUT_STATE_DIR`.
Path customization must happen through platform-specific base directory variables such as `XDG_STATE_HOME`.

* Must be JSON encoded in UTF-8.
* Parent directory must be created if missing.
* `state.json` must be created (empty state) if missing.

## Schema

```json
{
  "version": 3,
  "components": {
    "<canonical_id>": {
      "resources": [ <resource_entry>, ... ]
    }
  }
}
```

`version` must be `3`. `components` must be an object.

Component keys are **canonical IDs** in the format `<source_id>/<name>`, e.g. `core/git`.
All bare names (legacy v2 state) must be prefixed with `core/` when migrating to v3.

### Resource kinds

State records four kinds of resources: `package`, `runtime`, `fs`, and `tool`.

For detailed field definitions and types, see `crates/model/src/state.rs` and `crates/model/src/tool.rs` (rustdoc).

**`package` — Managed artifacts installed by package managers**

```json
{
  "kind": "package",
  "id": "pkg:<name>",
  "backend": "<backend_id>",
  "package": { "name": "<string>", "version": "<string|null>" }
}
```

**Meaning:**
- `version: null` means version was not tracked at install time (not "latest")
- `backend` is required for deterministic removal (use same backend that installed)

**`runtime` — Version-managed language runtimes (e.g., Node.js, Python)**

```json
{
  "kind": "runtime",
  "id": "rt:<name>@<version>",
  "backend": "<backend_id>",
  "runtime": { "name": "<string>", "version": "<string>" }
}
```

**Meaning:**
- `version` is always required (unlike packages)
- `backend` is required for deterministic removal

**`fs` — Files, directories, symlinks managed by loadout**

```json
{
  "kind": "fs",
  "id": "fs:<logical_id>",
  "fs": {
    "path": "<absolute_path>",
    "entry_type": "file|dir|symlink|junction",
    "op": "copy|link"
  }
}
```

**Meaning:**
- No `backend` field (fs operations are backend-independent)
- `path` must be absolute
- `logical_id` must be stable within a component (used for diff matching)

**`tool` — Tools installed by `managed_script` components**

```json
{
  "kind": "tool",
  "id": "tool:<name>",
  "tool": {
    "name": "<string>",
    "verify": {
      "identity": { ... }
    },
    "observed": {
      "resolved_path": "<absolute_path|null>",
      "version": "<string|null>"
    }
  }
}
```

**Meaning:**
- No `backend` field: tool installation is performed by the component's install script, not a backend
- `verify.identity` records the identity contract used during install-time verification; required for state validity
- `observed.resolved_path` is the actual path observed at install time; must be absolute when present
- `observed.version` is the version string observed at install time; null if no version verify was declared
- `tool` resources may only appear in components with `mode: managed_script`

## Identity Rules

Within a single component: `resource.id` must be unique.
Across components: the pair `(component_id, resource.id)` must be unique.
The same `fs.path` must NOT be recorded by multiple components.
`tool` resources are component-owned: no uniqueness constraint across components on `tool.name` or `tool.observed.resolved_path`.

## Invariants

Core must validate all invariants before execution. If any fails, execution must abort.

1. `version` must be `3`.
2. `components` must be an object.
3. Each component entry must contain a `resources` array.
4. Each resource must have a valid `kind` and matching kind payload.
5. Within a component: no duplicate `resource.id`.
6. Across all components: no duplicate `fs.path`.
7. All `fs.path` values must be absolute.
8. All `tool.observed.resolved_path` values (when non-null) must be absolute.
9. State must reflect only successfully completed operations.

## State Transition Rules

1. State must be initialized before any execution.
2. State must be updated only after a successful component-level operation.
3. State must not be partially written.
4. If execution fails, state must remain unchanged.
5. Uninstall must operate strictly on recorded resources.
6. Only the state module may write authoritative state.

## Atomic Commit Rules

`state_commit_atomic(new_state)` must:

1. Write to `state.json.tmp`
2. Validate JSON parse
3. Validate invariants in load mode
4. Replace `state.json` via atomic rename
5. Remove temp file on success

Direct in-place edits are forbidden.
Commit unit is a single component operation (install or uninstall of one component).

## Safety Rules

Core must remove **only** resources recorded in state.

Core must NOT:
* scan the filesystem to discover removal targets
* infer backends for resources without a backend record

For `package`/`runtime` removal: use the recorded `backend`. If the backend cannot be loaded, abort.

For `fs` removal: remove only the exact tracked `fs.path`. Must not remove parent directories
unless the parent is itself explicitly tracked as a `fs` resource with `entry_type: dir`.

For `tool` removal: after the component's uninstall script exits 0, verify absence using only
`tool.observed.resolved_path`. Must NOT check all `verify.identity.expected_path.one_of` candidates —
only the single previously observed path. If the observed path still exists, the operation fails and
state is not modified.

**Destructive path guards** — The fs module must refuse removal of dangerous paths even if recorded:

* Linux/WSL: `/`, `/home`, `/usr`, `/etc`, `/var`, `/bin`, `/sbin`, `/opt`
* Windows: `C:\`, `C:\Windows`, `C:\Program Files`, `C:\Program Files (x86)`, user profile root

## Corruption Handling

If `state.json` cannot be parsed as JSON, is not UTF-8, has unknown/missing `version`,
or fails invariant checks: execution must abort. Automatic repair must NOT be performed.

## Unknown Kind Handling

Load mode: tolerate unknown `kind` values (preserve raw JSON, enforce structural validity).
Execute mode: reject execution of any component containing an unknown `kind`.
Other components are not blocked unless they depend on the blocked component.

## Compatibility and Migration

### v1 State

v1 state may be read for migration. Executing with v1 state requires an explicit `loadout migrate` command.

### v2 State

v2 state used bare component names as keys (e.g., `"git"`). v2 state cannot be executed directly in Phase 3+;
the `loadout migrate` command must be run first to upgrade to v3.

### v2 → v3 Migration

**Transformation:**
  1. For each component key in `components` object:
     - If the key contains `/` (already canonical), keep it unchanged.
     - Otherwise (bare name), prefix with `core/` to form canonical ID.
  2. Increment `version` from `2` to `3`.
  3. Preserve all resource entries unchanged.

**Example:**

```json
// v2 state
{
  "version": 2,
  "components": {
    "git": { "resources": [...] },
    "core/ruby": { "resources": [...] }
  }
}

// v3 state after migration
{
  "version": 3,
  "components": {
    "core/git": { "resources": [...] },
    "core/ruby": { "resources": [...] }
  }
}
```

**Mutual exclusivity:**
- Once v3 is committed, v2 state cannot be executed (commands will reject it).
- Migration is idempotent: running migrate on v3 state is a no-op.

A `loadout migrate` command must be side-effect free in dry-run mode (`--dry-run`),
back up existing state (timestamped), validate the migrated result, and commit atomically.
Profile YAML may optionally be updated with canonical IDs via `--profiles` flag.

## Prohibited Content

State must NOT contain: profile content, strategy content, dependency graphs,
runtime environment variables, or arbitrary plugin-defined keys at component level.

Plugins must not write arbitrary extensions into state directly.
Extension requires adding a new kind and updating core validation.
