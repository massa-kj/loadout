# Windows Environment Tests

E2E tests for the loadout system on Windows (Windows Sandbox).

## Purpose

Verify that `loadout apply` works correctly on Windows by running tests in a
clean, isolated Windows Sandbox environment.

Tests focus on:

* Correct execution of Windows-specific backends (`winget`, `scoop`)
* State file creation and schema validity
* Safe uninstall behaviour

## Current State of Scenario Execution

Scenarios are currently driven by PowerShell scripts (`scenarios/*.ps1`).

A future goal is to reuse the `loadout-e2e` binary
(`tests/runtime/` Rust crate) once a cross-compiled `loadout-e2e.exe` is
available, aligning Windows tests with the Linux approach.

## Prerequisites

* Windows 10/11 Pro or Enterprise (Windows Sandbox must be enabled)
* Enable Windows Sandbox: **Turn Windows features on or off** → **Windows Sandbox**
* `loadout` binary built and accessible on `%PATH%`

## Quick Start

Run the top-level test script from PowerShell (as administrator):

```powershell
.\tests\e2e\windows\test.ps1 all
.\tests\e2e\windows\test.ps1 <scenario>
```

### Available commands

| Command | Description |
|---------|-------------|
| `all` | Run all scenarios |
| `<scenario>` | Run a specific scenario by name |
| `shell` | Open an interactive Sandbox session |
| `clean` | Remove log files |

## Test Scenarios

### lifecycle

Full lifecycle verification in a single run:

1. base apply — state initialised correctly
2. full apply — additional features installed
3. full apply (repeat) — idempotency confirmed
4. base apply — unwanted features removed safely
5. empty apply — all tracked resources removed

### minimal

Basic execution:

* State file created
* Schema version correct
* Features recorded

### idempotent

Determinism:

* Second apply does not change state

### uninstall

Safe removal:

* Tracked resources removed after apply with empty profile
* Untracked files preserved

## How Sandbox Works

Each test run:

1. Launches a fresh Windows Sandbox instance (no persistent state)
2. Copies the `loadout` binary and scenario scripts into the Sandbox
3. Runs the scenario inside the Sandbox
4. Captures the exit code and logs
5. Closes the Sandbox

The Sandbox is ephemeral — every run starts from a clean Windows install.

## Logs

Log files are written to `tests/e2e/windows/logs/` on the host.

They are excluded from version control (`.gitignore`).

## Troubleshooting

**Sandbox does not start**
Confirm that Windows Sandbox is enabled and that Hyper-V / virtualisation is
active in BIOS/UEFI.

**Script execution blocked**
Open PowerShell as administrator and run:
```powershell
Set-ExecutionPolicy RemoteSigned -Scope CurrentUser
```

**`loadout` not found inside Sandbox**
Ensure the binary was built (`cargo build --release`) and that `test.ps1`
correctly copies it into the Sandbox.

## File Structure

```
tests/e2e/windows/
├── test.ps1            # Test execution script
├── sandbox/
│   ├── setup.ps1       # Sandbox configuration
│   └── scenarios/      # PowerShell scenario scripts
└── README.md           # This file
```
