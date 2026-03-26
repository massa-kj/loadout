# Docker-based Testing

This directory contains Docker-based integration tests for the loadout system.

## Purpose

Verify loadout behavior in a clean, isolated environment.

Tests focus on **state guarantees** defined in STATE_SPEC.md:

* State initialization correctness
* Idempotent execution
* No duplicate resources
* Absolute path invariants

## Philosophy

These tests are **black-box tests**.

They verify:

* State structure and content
* Execution determinism
* System guarantees

They do NOT verify:

* Internal implementation details
* Specific package manager behavior
* Feature-specific configuration

State is the single source of truth.

## Test Scenarios

### lifecycle.sh

Verifies the common lifecycle in one pass:

* base apply initializes valid state
* full apply expands the installed set
* second full apply is idempotent
* base apply removes no-longer-desired features safely
* empty apply removes all tracked resources safely

This is the preferred scenario for `all`, because environment tests may use networked package managers.
The narrower scenarios remain available for focused debugging.

### minimal.sh

Verifies basic execution:

* State file is created
* Version field is correct
* Features are recorded
* No duplicates exist
* All paths are absolute

### idempotent.sh

Verifies determinism:

* Second apply does not change state
* No duplicate packages
* No duplicate files

### uninstall.sh

Verifies safe removal:

* State-tracked files are removed
* Non-tracked files are preserved (filesystem scan prohibition)
* State is properly cleaned
* Uninstall is idempotent
* No destructive operations outside state authority

### version_install.sh

Verifies version specification installation:

* Features with version configuration are installed correctly
* Version is recorded in state runtime metadata
* Packages include version information

### version_mixed.sh

Verifies mixed version/no-version features:

* Features with version specification record version in state
* Features without version specification do not record version
* Both types coexist correctly

### version_upgrade.sh

Verifies version change behavior:

* Version mismatch triggers reinstall
* Old version is removed before new installation
* State is updated with new version and package

## Docker Image Stages

The `Dockerfile` uses a two-stage build:

| Stage | Image | Contents | Use case |
|-------|-------|----------|----------|
| `base` | `loadout-base` | Minimal OS + repo copy | Pre-release binary validation |
| `installed` | `loadout-test` | base + loadout binary + features/backends | Scenario tests |

The `base` stage includes the repository at `/tmp/loadout-repo` for testing pre-release binaries.

The `installed` stage includes:
- loadout binary installed at `~/.local/bin/loadout`
- features/ and backends/ directories at `~/.config/loadout/`
- Test fixtures at `~/.config/loadout/configs/`

Scenario tests use the `installed` image for fast execution.

## Quick Start

### Run all tests

```bash
./tests/environment/linux/docker/test.sh all
```

This will:
1. Build the bootstrapped image (`loadout-test`)
2. Run lifecycle scenario
3. Run version_install scenario
4. Run version_mixed scenario
5. Run version_upgrade scenario

### Run specific test

```bash
./tests/environment/linux/docker/test.sh minimal
./tests/environment/linux/docker/test.sh lifecycle
./tests/environment/linux/docker/test.sh idempotent
./tests/environment/linux/docker/test.sh uninstall
./tests/environment/linux/docker/test.sh version-install
./tests/environment/linux/docker/test.sh version-mixed
./tests/environment/linux/docker/test.sh version-upgrade
```

### Build images

```bash
# Build installed image (used by scenario tests)
./tests/environment/linux/docker/test.sh build

# Build base image only (for pre-release binary validation)
./tests/environment/linux/docker/test.sh build-base
```

### Clean up

```bash
./tests/environment/linux/docker/test.sh clean
```

### Interactive shell (for debugging)

```bash
# Shell in installed container (loadout ready to use)
./tests/environment/linux/docker/test.sh shell

# Shell in base container (pre-installation, for binary validation)
./tests/environment/linux/docker/test.sh base-shell
```

`shell` is useful for:
- Testing plan command: `loadout plan -c ~/.config/loadout/configs/config-base.yaml`
- Testing apply command: `loadout apply -c ~/.config/loadout/configs/config-base.yaml`
- Running a scenario manually: `./tests/environment/linux/docker/scenarios/minimal.sh`
- Inspecting state: `cat ~/.local/state/loadout/state.json`

`base-shell` is useful for:
- Testing pre-release binaries: `./target/debug/loadout --help`
- Validating binary before installation
- Testing from repository root without installation

## Expected Behavior

All scenarios should:

* Execute without errors
* Exit with status 0
* Print "PASSED" at the end

Any failure indicates a violation of system guarantees.

## Design Principles

### Why Docker?

* Reproducible clean environment
* No host system pollution
* Easy CI integration
* Platform consistency

### Why State-Based Verification?

From ARCHITECTURE.md:

> State is both input and output of execution.
> No other layer persists execution memory.

If state is correct, the system is correct.

### What These Tests Do NOT Cover

* Package manager availability
* Network failures
* External system changes
* Runtime state outside loadout scope

These are environmental concerns, not architectural guarantees.

## Adding New Scenarios

When adding new test scenarios:

1. Create `tests/environment/linux/docker/scenarios/<name>.sh`
2. Follow `set -euo pipefail` pattern
3. Verify state only — not implementation
4. Exit 1 on any violation
5. Print clear failure messages

Document guarantees being tested.

## File Structure

```
tests/environment/linux/docker/
├── Dockerfile           # Two-stage build (base / bootstrapped)
├── test.sh              # Test execution script
├── README.md            # This file
└── scenarios/           # Test scenarios (run against bootstrapped image)
    ├── minimal.sh       # Basic execution test
    ├── lifecycle.sh     # Consolidated apply/uninstall lifecycle test
    ├── idempotent.sh    # Determinism test
    ├── uninstall.sh     # Safe removal test
    ├── version_install.sh   # Version specification test
    ├── version_mixed.sh     # Mixed version/no-version test
    └── version_upgrade.sh   # Version change test

# Note: .dockerignore is in the repository root
# (Docker requires it at the build context root)
```
