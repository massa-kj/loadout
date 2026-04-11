#Requires -Version 5.1

<#
.SYNOPSIS
    Run Windows Sandbox-based integration tests.

.DESCRIPTION
    Launches Windows Sandbox for each test scenario to verify loadout behavior
    in a clean, isolated environment.

    Uses pre-built host binaries (loadout.exe, loadout-e2e.exe) and dummy
    backends — no WinGet, no network access required inside the sandbox.

.PARAMETER Command
    Test command to execute:
    - all:             Run all test scenarios (default)
    - minimal:         State created, version correct, no duplicates
    - idempotent:      Second apply produces identical state
    - lifecycle:       Full multi-phase cycle (base > full > shrink > empty)
    - uninstall:       Tracked files removed; untracked files preserved
    - version-install: Version recorded in state after install
    - version-upgrade: Version mismatch triggers reinstall; state updated
    - version-mixed:   Versioned and unversioned components coexist correctly
    - shell:           Open an interactive Sandbox session (no scenario)
    - clean:           Remove generated .wsb files, logs, and release binaries

.PARAMETER Build
    Build host release binaries before running (cargo build --release).
    By default the script uses whatever binaries already exist in target\release\.

.EXAMPLE
    .\test.ps1 all
    Run all test scenarios using existing binaries

.EXAMPLE
    .\test.ps1 all -Build
    Build fresh binaries, then run all test scenarios

.EXAMPLE
    .\test.ps1 minimal
    Run the minimal scenario only

.EXAMPLE
    .\test.ps1 shell
    Open an interactive Sandbox for manual testing

.EXAMPLE
    .\test.ps1 clean
    Remove generated .wsb files, logs, and release binaries
#>

[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [ValidateSet("all", "minimal", "idempotent", "lifecycle", "uninstall",
                 "version-install", "version-mixed", "version-upgrade",
                 "shell", "clean")]
    [string]$Command = "all",

    [switch]$Build
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ScriptDir = $PSScriptRoot

# Color output helpers
function Write-Step { param([string]$m); Write-Host "==>" -ForegroundColor Green -NoNewline; Write-Host " $m" }
function Write-Info { param([string]$m); Write-Host "[INFO]" -ForegroundColor Blue -NoNewline; Write-Host " $m" }
function Write-Warn { param([string]$m); Write-Host "[WARN]" -ForegroundColor Yellow -NoNewline; Write-Host " $m" }

# Check that Windows Sandbox is available.
function Test-SandboxAvailable {
    # Get-WindowsOptionalFeature -Online requires elevation.
    # Fall back to checking for WindowsSandbox.exe when not running as admin.
    $isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)

    if ($isAdmin) {
        try {
            $f = Get-WindowsOptionalFeature -Online -FeatureName "Containers-DisposableClientVM" -ErrorAction Stop
            if ($f.State -ne "Enabled") {
                Write-Host ""
                Write-Warn "Windows Sandbox is not enabled"
                Write-Info "Enable it with:"
                Write-Host "  Enable-WindowsOptionalFeature -Online -FeatureName 'Containers-DisposableClientVM' -All" -ForegroundColor Yellow
                Write-Host ""
                return $false
            }
            return $true
        } catch {
            Write-Host ""
            Write-Warn "Windows Sandbox feature not found"
            Write-Info "Ensure you are running Windows 10 Pro/Enterprise (1903+) or Windows 11"
            Write-Host ""
            return $false
        }
    } else {
        # Non-admin fallback: check for the sandbox executable.
        $sandboxExe = "$env:SystemRoot\System32\WindowsSandbox.exe"
        if (Test-Path $sandboxExe) {
            return $true
        }
        Write-Host ""
        Write-Warn "Windows Sandbox feature not found"
        Write-Info "Ensure you are running Windows 10 Pro/Enterprise (1903+) or Windows 11"
        Write-Info "(Run as administrator to perform a definitive feature-state check)"
        Write-Host ""
        return $false
    }
}

# Check or build host release binaries.
# The sandbox mounts the repo read-only, so binaries must be in target\release\
# before launching.
#
# Default: verify binaries exist and abort with a hint if they don't.
# -ForceBuild: always run cargo build --release.
function Invoke-HostRelease {
    param([bool]$ForceBuild = $false)

    $RepoRoot   = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..\..")).Path
    $LoadoutBin = Join-Path $RepoRoot "target\release\loadout.exe"
    $E2eBin     = Join-Path $RepoRoot "target\release\loadout-e2e.exe"

    if ($ForceBuild) {
        Write-Step "Building host release binaries (cargo build --release)..."
        Push-Location $RepoRoot
        try {
            & cargo build -p loadout -p loadout-e2e --release
            if (-not $?) { throw "cargo build --release failed" }
        } finally {
            Pop-Location
        }
    } else {
        if (-not ((Test-Path $LoadoutBin) -and (Test-Path $E2eBin))) {
            Write-Host ""
            Write-Warn "Host release binaries not found in target\release\"
            Write-Info "Build them first with --build:"
            Write-Host "  .\test.ps1 $Command --build" -ForegroundColor Yellow
            Write-Host ""
            exit 1
        }
        Write-Info "Using existing host release binaries (pass --build to rebuild)"
    }
}

# Run a single test scenario inside Windows Sandbox.
function Invoke-TestScenario {
    param([string]$Scenario)

    Write-Step "Running $Scenario scenario..."

    $CreateWsbScript = Join-Path $ScriptDir "create-wsb.ps1"
    & $CreateWsbScript -Scenario $Scenario

    $WsbPath = Join-Path $ScriptDir "loadout.wsb"

    Write-Info "Launching Windows Sandbox..."
    Write-Info "Wait for the test to complete, then close the Sandbox window"
    Write-Host ""

    Start-Process -FilePath "WindowsSandbox.exe" -ArgumentList $WsbPath -Wait

    Write-Step "$Scenario scenario completed"
    Write-Host ""
}

# Open an interactive sandbox session (no scenario).
function Invoke-Shell {
    Write-Step "Opening interactive Sandbox session..."

    $CreateWsbScript = Join-Path $ScriptDir "create-wsb.ps1"
    & $CreateWsbScript  # no -Scenario: manual mode

    $WsbPath = Join-Path $ScriptDir "loadout.wsb"

    Write-Info "Launching Windows Sandbox for manual testing"
    Write-Info "Binaries will be available at: target\release\"
    Write-Info "Set `$env:SCENARIO and run: .\tests\e2e\windows\sandbox\run-in-sandbox.ps1"
    Write-Host ""

    Start-Process -FilePath "WindowsSandbox.exe" -ArgumentList $WsbPath -Wait
}

# Remove generated files and release binaries.
function Invoke-Clean {
    Write-Step "Cleaning up..."

    $WsbPath = Join-Path $ScriptDir "loadout.wsb"
    if (Test-Path $WsbPath) {
        Remove-Item $WsbPath -Force
        Write-Info "Removed: loadout.wsb"
    }

    $LogsDir = Join-Path $ScriptDir "..\logs"
    if (Test-Path $LogsDir) {
        Remove-Item $LogsDir -Recurse -Force
        Write-Info "Removed: logs/"
    }

    $RepoRoot = (Resolve-Path (Join-Path $ScriptDir "..\..\..\..")).Path
    foreach ($bin in @("loadout.exe", "loadout-e2e.exe")) {
        $p = Join-Path $RepoRoot "target\release\$bin"
        if (Test-Path $p) {
            Remove-Item $p -Force
            Write-Info "Removed: target\release\$bin"
        }
    }

    Write-Step "Clean complete"
}

# ── Main ─────────────────────────────────────────────────────────────────────

Write-Host ""
Write-Host "Windows Sandbox Integration Tests" -ForegroundColor Cyan
Write-Host "==================================" -ForegroundColor Cyan
Write-Host ""

if ($Command -eq "clean") {
    Invoke-Clean
    exit 0
}

if (-not (Test-SandboxAvailable)) { exit 1 }

# Check or build host binaries before any scenario (sandbox mounts repo read-only).
Invoke-HostRelease -ForceBuild $Build.IsPresent
Write-Host ""

switch ($Command) {
    "all" {
        $scenarios = @(
            "minimal", "idempotent", "lifecycle", "uninstall",
            "version-install", "version-upgrade", "version-mixed"
        )

        Write-Info "Running all scenarios: $($scenarios -join ', ')"
        Write-Host ""

        foreach ($s in $scenarios) {
            Invoke-TestScenario -Scenario $s
        }

        Write-Host ""
        Write-Step "All scenarios completed!"
        Write-Host ""
    }
    "shell" {
        Invoke-Shell
    }
    default {
        Invoke-TestScenario -Scenario $Command
    }
}
