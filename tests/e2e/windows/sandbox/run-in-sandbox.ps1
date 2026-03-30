#Requires -Version 5.1

<#
.SYNOPSIS
    Execute test scenario inside Windows Sandbox.

.DESCRIPTION
    This script runs inside Windows Sandbox (invoked by LogonCommand).
    It copies the repository, runs bootstrap, executes the test scenario,
    and saves logs.

.NOTES
    Environment variables set by .wsb configuration:
    - SCENARIO: Test scenario name (minimal, idempotent, uninstall, etc.)
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

# --- Logging setup ---
$LogDir = "C:\logs"
$Scenario = $env:SCENARIO

if (-not $Scenario) {
    Write-Host "ERROR: SCENARIO environment variable not set" -ForegroundColor Red
    Write-Host "This script should only be called with a scenario specified." -ForegroundColor Yellow
    Write-Host "For manual testing, use create-wsb.ps1 without -Scenario parameter." -ForegroundColor Yellow
    exit 1
}

$LogPath = Join-Path $LogDir "sandbox-$Scenario-$(Get-Date -Format 'yyyyMMdd-HHmmss').log"

if (Test-Path $LogDir) {
    Start-Transcript -Path $LogPath -Force | Out-Null
} else {
    Write-Host "WARN: Logs directory not available, logging to stdout only" -ForegroundColor Yellow
}

Write-Host ""
Write-Host "=====================================" -ForegroundColor Cyan
Write-Host "Windows Sandbox Test Execution" -ForegroundColor Cyan
Write-Host "=====================================" -ForegroundColor Cyan
Write-Host "Scenario: $Scenario" -ForegroundColor Cyan
Write-Host "Started: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')" -ForegroundColor Cyan
Write-Host "=====================================" -ForegroundColor Cyan
Write-Host ""

try {
    # Repository should already be copied by LogonCommand
    $WorkDir = "C:\loadout"
    if (-not (Test-Path $WorkDir)) {
        throw "Repository not found at $WorkDir. LogonCommand may have failed."
    }
    Set-Location $WorkDir
    
    # --- Verify WinGet (installed by LogonCommand) ---
    Write-Host "==> Verifying WinGet..." -ForegroundColor Green
    
    $wingetPath = Join-Path $env:LOCALAPPDATA "Microsoft\WindowsApps\winget.exe"
    if (Test-Path $wingetPath) {
        $testResult = & $wingetPath --version 2>&1
        Write-Host "    WinGet version: $testResult" -ForegroundColor Gray
    } else {
        throw "WinGet not found at: $wingetPath"
    }
    
    Write-Host ""
    
    # --- Run bootstrap ---
    Write-Host "==> Running bootstrap..." -ForegroundColor Green
    
    $BootstrapScript = ".\platforms\windows\bootstrap.ps1"
    & $BootstrapScript
    
    if ($LASTEXITCODE -ne 0) {
        throw "Bootstrap failed with exit code $LASTEXITCODE"
    }
    
    Write-Host ""
    
    # --- Run test scenario ---
    Write-Host "==> Running test scenario: $Scenario" -ForegroundColor Green
    
    $ScenarioScript = ".\tests\e2e\windows\sandbox\scenarios\$Scenario.ps1"
    if (-not (Test-Path $ScenarioScript)) {
        throw "Scenario script not found: $ScenarioScript"
    }
    
    & $ScenarioScript
    
    if ($LASTEXITCODE -ne 0) {
        throw "Scenario failed with exit code $LASTEXITCODE"
    }
    
    Write-Host ""
    Write-Host "=====================================" -ForegroundColor Green
    Write-Host "SUCCESS: $Scenario scenario passed" -ForegroundColor Green
    Write-Host "=====================================" -ForegroundColor Green
    Write-Host ""
    Write-Host "Please close this Sandbox window manually" -ForegroundColor Yellow
    Write-Host ""
    
} catch {
    Write-Host ""
    Write-Host "=====================================" -ForegroundColor Red
    Write-Host "FAILURE: $Scenario scenario failed" -ForegroundColor Red
    Write-Host "=====================================" -ForegroundColor Red
    Write-Host "Error: $_" -ForegroundColor Red
    Write-Host ""
    Write-Host "Check logs: $LogPath" -ForegroundColor Yellow
    Write-Host ""
    
    if (Test-Path $LogDir) {
        Stop-Transcript | Out-Null
    }
    
    exit 1
}

if (Test-Path $LogDir) {
    Stop-Transcript | Out-Null
}

# Keep window open for manual inspection
Read-Host "Press Enter to continue"
