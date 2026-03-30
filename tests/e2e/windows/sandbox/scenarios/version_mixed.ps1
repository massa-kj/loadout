#Requires -Version 5.1

<#
.SYNOPSIS
    Version mixed scenario - Mixed version/no-version features test

.DESCRIPTION
    Verifies:
    - Features with version specification record version in state
    - Features without version specification do not record version
    - Both types coexist correctly
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$StateFile = "state\state.json"
$FixturesDir = Join-Path $PSScriptRoot "..\..\..\..\fixtures\configs"

# Use test-specific strategy (no backup, standard backends)
$global:LOADOUT_STRATEGY_FILE = (Resolve-Path (Join-Path $FixturesDir "strategy.yaml")).Path
$ProfileMixed = (Resolve-Path (Join-Path $FixturesDir "profile-version-mixed.yaml")).Path

Write-Host "==> Version mixed scenario" -ForegroundColor Cyan
Write-Host ""

Write-Host "==> Running apply with mixed features" -ForegroundColor Green
.\loadout.ps1 apply $ProfileMixed

if ($LASTEXITCODE -ne 0) {
    throw "Apply command failed"
}

Write-Host ""
Write-Host "==> Checking state file existence" -ForegroundColor Green
if (-not (Test-Path $StateFile)) {
    throw "State file not created"
}

Write-Host "==> Validating JSON format" -ForegroundColor Green
try {
    $State = Get-Content $StateFile -Raw | ConvertFrom-Json
} catch {
    throw "Invalid JSON format: $_"
}

Write-Host "==> Verifying node has version in state" -ForegroundColor Green
$NodeFeature = $State.features.node
if (-not ($NodeFeature.PSObject.Properties.Name -contains "runtime")) {
    throw "node runtime metadata not found"
}

$NodeVersion = $NodeFeature.runtime.version
if ($NodeVersion -ne "20") {
    throw "Node version not recorded correctly: $NodeVersion"
}

Write-Host "    node version: $NodeVersion" -ForegroundColor Gray

Write-Host "==> Verifying git has no version in state" -ForegroundColor Green
$GitFeature = $State.features.git
if ($GitFeature.PSObject.Properties.Name -contains "runtime") {
    if ($GitFeature.runtime.PSObject.Properties.Name -contains "version") {
        $GitVersion = $GitFeature.runtime.version
        throw "git should not have version recorded: $GitVersion"
    }
}

Write-Host "    git: no version (correct)" -ForegroundColor Gray

Write-Host "==> Verifying powershell has no version in state" -ForegroundColor Green
$PowerShellFeature = $State.features.powershell
if ($PowerShellFeature.PSObject.Properties.Name -contains "runtime") {
    if ($PowerShellFeature.runtime.PSObject.Properties.Name -contains "version") {
        $PowerShellVersion = $PowerShellFeature.runtime.version
        throw "powershell should not have version recorded: $PowerShellVersion"
    }
}

Write-Host "    powershell: no version (correct)" -ForegroundColor Gray

Write-Host "==> Verifying all features installed" -ForegroundColor Green
$FeatureCount = $State.features.PSObject.Properties.Name.Count
if ($FeatureCount -lt 4) {
    throw "Not all features installed: $FeatureCount"
}

Write-Host "    Installed features: $FeatureCount" -ForegroundColor Gray

Write-Host ""
Write-Host "==> Version mixed scenario PASSED" -ForegroundColor Green
