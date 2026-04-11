# Testing Strategy

## Test Types

Three levels of testing exist in this project:

**Unit tests** — pure function behavior (planner logic, state parsing, resolver sort)

**Integration tests** — multi-module interaction (orchestrator flow, state commit after apply)

**Environment tests** — Docker-based full installation scenarios

## Unit Tests

Unit tests cover stable public APIs and invariant-critical logic.

**Test runner:** `cargo test` (Rust-native)

Target crates:
* `crates/planner` — classification, decision table
* `crates/state` — schema validation, invariants, atomic commit
* `crates/resolver` — topological sort, cycle detection, capability injection
* `crates/source-registry` — canonical ID parsing, source path resolution, allow-list checks
* `crates/compiler` — DesiredResourceGraph output format, platform override, backend resolution
* `crates/executor` — resource routing by kind, all plan operations
* `crates/model` — data structure validation (State, Profile, Strategy, etc.)

Internal APIs must NOT be tested directly.
Tests must validate behavior, not implementation details.

## Integration Tests

Integration tests verify the orchestrator pipeline end-to-end with real file I/O
but without executing component scripts.

Scenarios to cover:
* Initial install (create)
* No-op (noop)
* Version mismatch (replace)
* Component removal (destroy)
* Dependency ordering

Test location: `tests/`

Tests that exercise path resolution must isolate XDG/AppData roots instead of modifying shared user paths.

## Environment Tests

| Environment | Description |
|-------------|-------------|
| [Environment](../../tests/e2e/README.md) | Full apply execution in isolated environments (Docker, Windows Sandbox) |
| [Linux (Docker)](../../tests/e2e/linux/docker/README.md) | Full apply execution in a fresh Ubuntu container |
| [Windows (Sandbox)](../../tests/e2e/windows/README.md) | Full apply execution in a disposable Windows Sandbox instance |

## Path Isolation Rules

Tests must not rely on repository-local state/profile/strategy paths as authoritative runtime paths.

Use:

* `XDG_CONFIG_HOME`
* `XDG_STATE_HOME`
* `XDG_DATA_HOME`

to redirect runtime paths in Linux/WSL tests.
On Windows tests, use disposable AppData/LocalAppData roots.

`LOADOUT_STATE_FILE` and `LOADOUT_STATE_DIR` must not be reintroduced in tests.

## What Must NOT Be Tested

Tests must validate behavior, not implementation details.

Do not test:
* Internal APIs (functions not listed in crate public API)
* Rust-specific implementation choices (use of `HashMap` vs `BTreeMap`, iterator style, etc.)
* Log output format
* OS-specific branching internals
* Error message wording (only error type/variant)

If a test breaks because of a refactor that preserves behavior, the test is testing the wrong thing.

## Breaking Changes

Tests and documentation must be updated when:
* A stable API changes signature or semantics
* The state schema changes (version bump required)
* The execution phase order changes
* The planner decision table changes

Such changes require coordinated updates: spec + test + doc in the same change.

## CI Strategy

{TODO}
