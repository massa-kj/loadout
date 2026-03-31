# Environment Tests

Full-stack integration tests that run the loadout system in isolated environments.

Unlike unit and integration tests, these tests execute real feature scripts against
a clean OS installation to verify end-to-end behavior.

## Purpose

Verify state guarantees in a realistic environment:

* State initialization correctness
* Idempotent execution
* Safe uninstall (no untracked files removed)
* Version specification handling
* Version upgrade behavior

## E2E Runner

Scenarios are executed by the `loadout-e2e` binary (`tests/runtime/` crate).

`loadout-e2e` is a standalone binary copied into containers or Sandbox instances
alongside `loadout`. It deserialises the state file using `model::state::State`
for type-safe verification — no dependency on `jq` or other external tools.

```bash
# Inside the container
loadout-e2e minimal
loadout-e2e all
```

The Windows Sandbox environment still uses PowerShell scripts (`scenarios/*.ps1`).
Migration to a cross-compiled `loadout-e2e.exe` (`x86_64-pc-windows-msvc`) is
a future goal.

## Test Environments

### Linux — Docker

`linux/docker/` provides Docker-based testing on Ubuntu.

A four-stage Dockerfile manages the environment; scenarios are executed by
the `loadout-e2e` binary.

**Quick start:**

```bash
./tests/e2e/linux/docker/test.sh all
```

See [linux/docker/README.md](linux/docker/README.md) for full documentation.

### Windows — Windows Sandbox

`windows/` provides Windows Sandbox-based testing.

Each test launches a disposable Sandbox instance, installs WinGet, copies the
repository, and executes the scenario inside the Sandbox.

**Quick start (Windows only):**

```powershell
cd tests\e2e\windows\sandbox
.\test.ps1 all
```

See [windows/README.md](windows/README.md) for full documentation.

## Test Scenarios

Both environments cover the same set of scenarios:

| Scenario          | What it verifies                                              |
|-------------------|---------------------------------------------------------------|
| `minimal`         | State created, version correct, no duplicates                 |
| `idempotent`      | Second apply produces identical state                         |
| `lifecycle`       | Full cycle: base → full → reapply → shrink → empty            |
| `uninstall`       | Tracked files removed; untracked files preserved              |
| `version-install` | Version recorded in state after install                       |
| `version-upgrade` | Version mismatch triggers reinstall; state updated            |
| `version-mixed`   | Versioned and unversioned features coexist correctly          |

## Design Philosophy

These are **black-box tests**. They verify observable guarantees (state structure
and content), not internal implementation details.

See [docs/development/testing.md](../../docs/development/testing.md) for the
overall testing strategy.
