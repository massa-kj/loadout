# Windows Environment Tests

Tests for validating loadout behavior in isolated Windows environments.

## Purpose

Verify loadout behavior in isolated Windows environments.

Tests focus on **state guarantees** defined in STATE_SPEC.md:

* State initialization correctness
* Idempotent execution
* No duplicate resources
* Absolute path invariants
* Version specification handling

## Sandbox-based Testing

Windows Sandbox provides a lightweight, disposable environment for testing.

### Prerequisites

* Windows 10 Pro/Enterprise (1903+) or Windows 11
* Windows Sandbox feature enabled
* Hyper-V enabled (required by Sandbox)

Enable Windows Sandbox:

```powershell
Enable-WindowsOptionalFeature -Online -FeatureName "Containers-DisposableClientVM" -All
```

### Test Scenarios

#### minimal.ps1

Verifies basic execution:

* State file is created
* Version field is correct
* Features are recorded
* No duplicates exist
* All paths are absolute

#### idempotent.ps1

Verifies determinism:

* Second apply does not change state
* No duplicate packages
* No duplicate files

#### uninstall.ps1

Verifies safe removal:

* State-tracked files are removed
* Non-tracked files are preserved
* State is properly cleaned
* Uninstall is idempotent

#### version_install.ps1

Verifies version specification installation:

* Features with version configuration are installed correctly
* Version is recorded in state runtime metadata
* Packages include version information

#### version_mixed.ps1

Verifies mixed version/no-version features:

* Features with version specification record version in state
* Features without version specification do not record version
* Both types coexist correctly

#### version_upgrade.ps1

Verifies version change behavior:

* Version mismatch triggers reinstall
* Old version is removed before new installation
* State is updated with new version and package

### Quick Start

#### Run all tests

```powershell
cd tests\environment\windows\sandbox
.\test.ps1 all
```

This will run all six scenarios sequentially.

Each test:
1. Generates a `.wsb` configuration
2. Copies repository to `C:\loadout`
3. Installs WinGet (via LogonCommand)
4. Copies repository to `C:\loadout`
5. Executes the test scenario inside Sandbox
6. Saves logs to `tests\environment\windows\logs\`

**Note**: Sandbox windows must be closed manually after each test completes.

#### Run specific test

```powershell
cd tests\environment\windows\sandbox
.\test.ps1 minimal
.\test.ps1 idempotent
.\test.ps1 uninstall
.\test.ps1 version-install
.\test.ps1 version-mixed
.\test.ps1 version-upgrade
```

#### Manual testing

For manual debugging without running a scenario:

```powershell
cd tests\environment\windows\sandbox
.\create-wsb.ps1  # No -Scenario parameter
.\loadout.wsb    # Opens Sandbox with WinGet + repo ready
```

Inside Sandbox, manually run:

```powershell
# Already at C:\loadout
.\platforms\windows\bootstrap.ps1
.\loadout.ps1 apply profiles\windows.yaml
Get-Content state\state.json
```

With a specific scenario:

```powershell
cd tests\environment\windows\sandbox
.\create-wsb.ps1 -Scenario minimal
.\loadout.wsb  # Opens Sandbox and runs scenario automatically
```

### Expected Behavior

All scenarios should:

* Execute without errors
* Exit with status 0
* Print "PASSED" at the end
* Save logs to `C:\logs\` (mapped to host)

Any failure indicates a violation of system guarantees.

### Design Principles

#### Test Isolation

* Each test runs in a fresh Sandbox instance
* Host repository mounted as read-only at `C:\host-loadout`
* Repository copied to `C:\loadout` for test execution
* WinGet installed fresh for each test
* No persistent state between tests

### Troubleshooting

#### Sandbox fails to start

Ensure Windows Sandbox is enabled:

```powershell
Get-WindowsOptionalFeature -Online -FeatureName "Containers-DisposableClientVM"
```

If not enabled, run:

```powershell
Enable-WindowsOptionalFeature -Online -FeatureName "Containers-DisposableClientVM" -All
Restart-Computer
```

#### WinGet not available in Sandbox

WinGet is automatically installed via LogonCommand. If this fails:

1. Check PowerShell window for error messages
2. Review Sandbox logs in `tests\environment\windows\logs\`
3. Verify network access is enabled in Sandbox
4. Manually test WinGet installation script

The installation process:
- Downloads VCLibs, UI.Xaml, and WinGet from official sources
- Installs dependencies and WinGet package
- Verifies installation by checking `winget --version`

#### PowerShell execution policy in Sandbox

If you encounter errors about script execution being disabled,
Set execution policy for the current session:

```powershell
Set-ExecutionPolicy Bypass -Scope Process
```

Then retry the command. This setting applies only to the current PowerShell session and does not affect system-wide policy.

#### Test hangs or fails

1. Check logs in `tests\environment\windows\logs\sandbox-*.log`
2. Run scenario manually in interactive Sandbox
3. Verify `profiles\windows.yaml` is valid

### Logs

Test execution logs are saved to:

```
tests\environment\windows\logs\sandbox-{timestamp}.log
```

Logs include:

* Full test output (stdout/stderr)
* PowerShell transcript
* State file contents (on success)
