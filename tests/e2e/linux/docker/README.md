# Docker-based Testing

Docker-based E2E tests for the loadout system on Linux (Ubuntu).

## Purpose

Verify state guarantees by running `loadout apply` in a fresh Ubuntu container.

Tests focus on:

* State initialization and JSON schema validity
* Idempotent execution (state unchanged after a second apply)
* Safe uninstall (no untracked files removed)
* Version specification recording and upgrade behaviour

These are **black-box tests**: only state content and structure are verified,
not internal implementation details.

## E2E Runner

Scenarios are executed by the `loadout-e2e` binary (`tests/runtime/` crate).

`loadout-e2e` deserialises the state file using `model::state::State` for
type-safe assertions — no dependency on `jq` or other external tools.
The same binary is intended to be reused for Windows once a cross-compiled
`loadout-e2e.exe` is available.

## Docker Image Stages

The `Dockerfile` uses a six-stage build. Three stages produce named images;
three are intermediate layers used only during the build:

| Stage | Image | Contents | Use case |
|-------|-------|----------|----------|
| `os-base` | — (intermediate) | Ubuntu + system deps, no source | Shared base |
| `os` | `loadout-os` | `os-base` + source | Manual install testing |
| `rust-toolchain` | — (intermediate) | `os-base` + Rust toolchain (no source) | Cached toolchain layer |
| `builder` | — (intermediate) | `rust-toolchain` + source + `cargo build` | Foundation for `dev` |
| `dev` | `loadout-dev` | `builder` + binaries + config | Try the latest source without a host build |
| `test` | `loadout-test` | `os` + host release binaries + config | Scenario execution / CI |

`rust-toolchain` is placed **before** `COPY . .` so that the rustup installation
is cached across source changes — it is only re-run when the Dockerfile itself changes.

`dev` runs `cargo build --release` inside Docker — no Rust toolchain required on the host.
BuildKit cache mounts keep the Cargo registry and `target/` between builds so only
changed crates are recompiled on subsequent runs.

`test` copies pre-built `target/release/loadout` and `target/release/loadout-e2e`
from the host, keeping the image small (no Rust toolchain inside).

## Quick Start

### Run scenarios

```bash
# Run all scenarios (recommended)
./tests/e2e/linux/docker/test.sh all

# Run a specific scenario
./tests/e2e/linux/docker/test.sh lifecycle
./tests/e2e/linux/docker/test.sh minimal
./tests/e2e/linux/docker/test.sh idempotent
./tests/e2e/linux/docker/test.sh uninstall
./tests/e2e/linux/docker/test.sh version-install
./tests/e2e/linux/docker/test.sh version-mixed
./tests/e2e/linux/docker/test.sh version-upgrade
```

`test.sh <scenario>` automatically runs `cargo build --release` on the host if
the binaries are absent, builds the `loadout-test` image, and runs
`loadout-e2e <scenario>` inside the container.

### Open an interactive shell

```bash
# test image: pre-built release binary
./tests/e2e/linux/docker/test.sh shell

# dev image: built from source inside Docker
./tests/e2e/linux/docker/test.sh dev-shell

# os image: bare Ubuntu, no loadout — starting point for manual install
./tests/e2e/linux/docker/test.sh os-shell
```

Commands available inside `shell` / `dev-shell`:

```bash
loadout apply --config ~/.config/loadout/configs/config-base.yaml
loadout-e2e minimal
loadout-e2e all
cat "$XDG_STATE_HOME/loadout/state.json"
```

### Build images only

```bash
# test image (uses host release binaries)
./tests/e2e/linux/docker/test.sh build

# dev image (cargo build inside Docker — requires network on first run)
./tests/e2e/linux/docker/test.sh build-dev

# os image (bare Ubuntu only)
./tests/e2e/linux/docker/test.sh build-os
```

### Clean up

```bash
./tests/e2e/linux/docker/test.sh clean
```

## Test Scenarios

### lifecycle

The most comprehensive scenario — verifies the full lifecycle in a single pass:

1. base apply — state initialised correctly
2. full apply — additional components installed
3. full apply (repeat) — idempotency confirmed
4. base apply — unwanted components removed safely
5. empty apply — all tracked resources removed

`all` runs this scenario as the primary test.

### minimal

Basic execution:

* State file created
* Version field correct (`version == 3`)
* Components recorded
* No duplicate resource IDs
* All `fs` paths are absolute

### idempotent

Determinism:

* Second apply does not change state

### uninstall

Safe removal — three sub-tests:

1. Partial uninstall (full → base profile)
2. Full uninstall (base → empty profile)
3. Idempotent uninstall (empty → empty)

Verifies that tracked files are removed and untracked files are preserved.

### version-install

Version specification:

* `runtime` resource in state records the installed version
* `package` resource name includes the version

### version-mixed

Coexistence of versioned and unversioned components:

* Components with a version spec record a `runtime` resource
* Components without a version spec do not

### version-upgrade

Version change:

* Version mismatch triggers reinstall
* State updated to reflect the new version

## File Structure

```
tests/e2e/linux/docker/
├── Dockerfile    # 6-stage build (os-base / os / rust-toolchain / builder / dev / test)
├── test.sh       # Test execution script
└── README.md     # This file
```

Scenario implementations live in `tests/runtime/src/scenarios/` (Rust).
Inside the container they are invoked as `loadout-e2e <scenario>`.

`.dockerignore` is located at the repository root (required by Docker).
