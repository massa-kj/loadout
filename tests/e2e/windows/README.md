# Windows E2E Tests

E2E tests for loadout on Windows, executed inside Windows Sandbox.

## Overview

Tests run against pre-built host binaries (`loadout.exe`, `loadout-e2e.exe`)
inside a clean, ephemeral sandbox. Dummy backends are used — no WinGet, no
network access required.

The same `loadout-e2e` Rust binary that drives Linux Docker tests also drives
the Windows Sandbox tests, ensuring both platforms exercise identical scenario
logic.

## Prerequisites

* Windows 10/11 Pro or Enterprise
* Windows Sandbox enabled:
  **Turn Windows features on or off → Windows Sandbox**
  or run as Administrator:
  ```powershell
  Enable-WindowsOptionalFeature -Online -FeatureName Containers-DisposableClientVM -All
  ```
* Rust toolchain (for the initial binary build — `cargo build --release`)

## Quick Start

Run from PowerShell (Administrator) in the repository root:

```powershell
.\tests\e2e\windows\sandbox\test.ps1 all
.\tests\e2e\windows\sandbox\test.ps1 minimal
.\tests\e2e\windows\sandbox\test.ps1 shell   # interactive session
```

`test.ps1` automatically builds `loadout.exe` and `loadout-e2e.exe` if the
release binaries are not present. After the first build the check is fast.

## Test Scenarios

| Scenario         | Description                                               |
|------------------|-----------------------------------------------------------|
| `minimal`        | State created, version correct, no duplicates             |
| `idempotent`     | Second apply produces identical state                     |
| `lifecycle`      | Full cycle: base → full → idempotent → shrink → empty     |
| `uninstall`      | Tracked resources removed; untracked files preserved      |
| `version-install`| Version recorded in state after runtime install           |
| `version-upgrade`| Version mismatch triggers reinstall; state updated        |
| `version-mixed`  | Versioned and unversioned features coexist correctly      |

## How It Works

1. `test.ps1` ensures `target\release\loadout.exe` and `loadout-e2e.exe` exist
2. `create-wsb.ps1` generates `loadout.wsb` from the template
3. Windows Sandbox launches with:
   * Repository mounted read-only at `C:\host-loadout`
   * Log directory mounted read-write at `C:\logs` → `tests\e2e\windows\logs\`
4. Inside the sandbox, `run-in-sandbox.ps1`:
   * Copies the repo to `C:\loadout`
   * Installs binaries into `%LOCALAPPDATA%\loadout\bin\`
   * Sets `XDG_CONFIG_HOME` and `XDG_STATE_HOME` to `%APPDATA%` so that
     `loadout.exe` and `loadout-e2e.exe` agree on the same config/state paths
   * Copies configs, features, and dummy backends to `%APPDATA%\loadout\`
   * Runs `loadout-e2e.exe <scenario>`

## Path Convention (Windows)

| Purpose          | Path                                  |
|------------------|---------------------------------------|
| Config / features / backends | `%APPDATA%\loadout\`    |
| State file       | `%APPDATA%\loadout\state.json`        |
| Configs dir      | `%APPDATA%\loadout\configs\`          |
| Features dir     | `%APPDATA%\loadout\features\`         |
| Backends dir     | `%APPDATA%\loadout\backends\`         |

`loadout.exe` uses `%APPDATA%\loadout\` natively on Windows.
`loadout-e2e.exe` is aligned via `XDG_CONFIG_HOME=%APPDATA%` and
`XDG_STATE_HOME=%APPDATA%`.

## Logs

Log files are written to `tests\e2e\windows\logs\` on the host during sandbox
execution. They are excluded from version control.

## Troubleshooting

**Sandbox does not start**
Confirm Windows Sandbox is enabled and Hyper-V / virtualisation is active in
BIOS/UEFI.

**Script execution blocked**
```powershell
Set-ExecutionPolicy RemoteSigned -Scope CurrentUser
```

**`loadout.exe` not found inside Sandbox**
Ensure the release build exists:
```powershell
cargo build -p loadout -p loadout-e2e --release
```
or just re-run `test.ps1` — it builds automatically.

## File Structure

```
tests/e2e/windows/
├── README.md
└── sandbox/
    ├── test.ps1                # Top-level test runner
    ├── create-wsb.ps1          # Generates loadout.wsb from template
    ├── run-in-sandbox.ps1      # Runs inside the sandbox
    └── loadout.wsb.template    # Windows Sandbox configuration template
```
