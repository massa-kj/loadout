#Requires -Version 5.1

<#
.SYNOPSIS
    Execute test scenario inside Windows Sandbox using the loadout-e2e binary.

.DESCRIPTION
    This script runs inside Windows Sandbox (invoked by LogonCommand).
    It installs the pre-built loadout binaries, sets up the loadout config
    directory, and runs loadout-e2e.exe for the requested scenario.

    No network access, WinGet, or bootstrap required — dummy backends are used.

.NOTES
    Environment variable set by create-wsb.ps1 via LogonCommand:
    - SCENARIO: Test scenario name (minimal, idempotent, lifecycle, etc.)
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$Scenario = $env:SCENARIO
if (-not $Scenario) {
    Write-Host "ERROR: SCENARIO environment variable not set" -ForegroundColor Red
    Write-Host "This script should only be called with a scenario specified." -ForegroundColor Yellow
    exit 1
}

# --- Logging setup ---
$LogDir  = "C:\logs"
$LogPath = Join-Path $LogDir "sandbox-$Scenario-$(Get-Date -Format 'yyyyMMdd-HHmmss').log"
if (Test-Path $LogDir) {
    Start-Transcript -Path $LogPath -Force | Out-Null
} else {
    Write-Host "WARN: Logs directory not available, logging to stdout only" -ForegroundColor Yellow
}

Write-Host ""
Write-Host "=====================================" -ForegroundColor Cyan
Write-Host "Windows Sandbox Test Execution"       -ForegroundColor Cyan
Write-Host "=====================================" -ForegroundColor Cyan
Write-Host "Scenario: $Scenario"                  -ForegroundColor Cyan
Write-Host "Started : $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')" -ForegroundColor Cyan
Write-Host "=====================================" -ForegroundColor Cyan
Write-Host ""

try {
    # Repository is pre-copied to C:\loadout by LogonCommand.
    $WorkDir = "C:\loadout"
    if (-not (Test-Path $WorkDir)) {
        throw "Repository not found at $WorkDir. LogonCommand may have failed."
    }
    Set-Location $WorkDir

    # --- Install binaries into PATH ---
    Write-Host "==> Installing binaries..." -ForegroundColor Green
    $BinDir = Join-Path $env:LOCALAPPDATA "loadout\bin"
    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    Copy-Item "target\release\loadout.exe"     (Join-Path $BinDir "loadout.exe")     -Force
    Copy-Item "target\release\loadout-e2e.exe" (Join-Path $BinDir "loadout-e2e.exe") -Force
    $env:PATH = "$BinDir;$env:PATH"
    Write-Host "    Installed to: $BinDir" -ForegroundColor Gray

    # --- Align XDG env vars with Windows APPDATA ---
    # loadout.exe on Windows resolves config/state from %APPDATA%\loadout\.
    # loadout-e2e resolves them from XDG_CONFIG_HOME and XDG_STATE_HOME.
    # Setting both to %APPDATA% makes all tools agree on the same base path:
    #   config_dir  -> %APPDATA%\loadout\configs\
    #   state_file  -> %APPDATA%\loadout\state.json
    $env:XDG_CONFIG_HOME = $env:APPDATA
    $env:XDG_STATE_HOME  = $env:APPDATA
    $env:LOADOUT_REPO    = $WorkDir

    $LoadoutRoot = Join-Path $env:APPDATA "loadout"

    # --- Set up config, components, and backends ---
    Write-Host "==> Setting up loadout config..." -ForegroundColor Green
    $ComponentsDir = Join-Path $LoadoutRoot "components"
    $BackendsDir = Join-Path $LoadoutRoot "backends"
    $ConfigsDir  = Join-Path $LoadoutRoot "configs"
    New-Item -ItemType Directory -Force -Path $ComponentsDir, $BackendsDir, $ConfigsDir | Out-Null

    # Copy repo-bundled components and backends first.
    Copy-Item "components\*" $ComponentsDir -Recurse -Force
    Copy-Item "backends\*" $BackendsDir -Recurse -Force

    # Merge fixture backends/components on top (register as local/dummy-*).
    Copy-Item "tests\fixtures\backends\*" $BackendsDir -Recurse -Force
    Copy-Item "tests\fixtures\components\*" $ComponentsDir -Recurse -Force
    Copy-Item "tests\fixtures\configs\*"  $ConfigsDir  -Force

    Write-Host "    Config root : $LoadoutRoot" -ForegroundColor Gray
    Write-Host "    Configs     : $ConfigsDir"  -ForegroundColor Gray
    Write-Host ""

    # --- Run scenario via loadout-e2e ---
    Write-Host "==> Running scenario: $Scenario" -ForegroundColor Green
    & loadout-e2e.exe $Scenario

    if ($LASTEXITCODE -ne 0) {
        throw "loadout-e2e exited with code $LASTEXITCODE"
    }

    Write-Host ""
    Write-Host "=====================================" -ForegroundColor Green
    Write-Host "SUCCESS: $Scenario scenario passed"   -ForegroundColor Green
    Write-Host "=====================================" -ForegroundColor Green
    Write-Host ""
    Write-Host "Please close this Sandbox window manually" -ForegroundColor Yellow
    Write-Host ""

} catch {
    Write-Host ""
    Write-Host "=====================================" -ForegroundColor Red
    Write-Host "FAILURE: $Scenario scenario failed"   -ForegroundColor Red
    Write-Host "=====================================" -ForegroundColor Red
    Write-Host "Error: $_"                            -ForegroundColor Red
    Write-Host ""
    if (Test-Path $LogDir) { Write-Host "Check logs: $LogPath" -ForegroundColor Yellow }
    Write-Host ""
    if (Test-Path $LogDir) { Stop-Transcript | Out-Null }
    exit 1
}

if (Test-Path $LogDir) { Stop-Transcript | Out-Null }
Read-Host "Press Enter to continue"
