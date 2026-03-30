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

## Test Environments

### Linux — Docker

`linux/docker/` provides Docker-based testing on Ubuntu.

Each test spins up a fresh container, runs bootstrap, executes `loadout apply`,
and verifies the resulting state file.

**Quick start:**

```bash
./tests/e2e/linux/docker/test.sh all
```

See [linux/docker/README.md](linux/docker/README.md) for full documentation.

### Windows — Windows Sandbox

`windows/` provides Windows Sandbox-based testing.

Each test launches a disposable Sandbox instance, installs WinGet, copies the
repository, runs bootstrap, and executes the scenario inside the Sandbox.

**Quick start (from Windows):**

```powershell
cd tests\e2e\windows\sandbox
.\test.ps1 all
```

See [windows/README.md](windows/README.md) for full documentation.

## Test Scenarios

Both environments cover the same set of scenarios:

| Scenario         | What it verifies                                     |
|------------------|------------------------------------------------------|
| `minimal`        | State is created, version correct, no duplicates     |
| `idempotent`     | Second apply produces identical state                |
| `uninstall`      | Tracked files removed, untracked files preserved     |
| `version_install`| Version recorded in state after install              |
| `version_upgrade`| Version mismatch triggers reinstall, state updated   |
| `version_mixed`  | Versioned and unversioned features coexist correctly |

## Design Philosophy

These are **black-box tests**. They verify observable guarantees (state structure
and content), not internal implementation details.

See [docs/development/testing.md](../../docs/development/testing.md) for the
overall testing strategy.
