#Requires -Version 5.1

<#
.SYNOPSIS
    Version install scenario - Version specification installation test

.DESCRIPTION
    Verifies:
    - Features with version configuration are installed correctly
    - Version is recorded in state runtime metadata
    - Packages include version information
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$StateFile = "state\state.json"
$FixturesDir = Join-Path $PSScriptRoot "..\fixtures"

# Use test-specific policy (no backup, standard backends)
$global:LOADOUT_POLICY_FILE = (Resolve-Path (Join-Path $FixturesDir "policy.yaml")).Path
$ProfileVersion = (Resolve-Path (Join-Path $FixturesDir "profile-version-v20.yaml")).Path

Write-Host "==> Version install scenario" -ForegroundColor Cyan
Write-Host ""

Write-Host "==> Running apply with version-specified features" -ForegroundColor Green
.\loadout.ps1 apply $ProfileVersion

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

Write-Host "==> Verifying node is installed" -ForegroundColor Green
if (-not ($State.features.PSObject.Properties.Name -contains "node")) {
    throw "node feature not found in state"
}

Write-Host "==> Verifying node version recorded in state" -ForegroundColor Green
$NodeFeature = $State.features.node
if (-not ($NodeFeature.PSObject.Properties.Name -contains "runtime")) {
    throw "node runtime metadata not found in state"
}

$NodeVersion = $NodeFeature.runtime.version
if ($NodeVersion -ne "20") {
    throw "Node version not recorded correctly: $NodeVersion (expected: 20)"
}

Write-Host "    Recorded version: $NodeVersion" -ForegroundColor Gray

Write-Host "==> Verifying node package registered in state" -ForegroundColor Green
if (-not ($NodeFeature.PSObject.Properties.Name -contains "packages")) {
    throw "node packages not found in state"
}

$NodePackages = $NodeFeature.packages
$NodePackageWithVersion = $NodePackages | Where-Object { $_ -like "node@*" }
if (-not $NodePackageWithVersion) {
    throw "Node package with version not registered in state"
}

Write-Host "    Registered package: $NodePackageWithVersion" -ForegroundColor Gray

if ($NodePackageWithVersion -notmatch "node@20") {
    throw "Node package version incorrect: $NodePackageWithVersion"
}

Write-Host ""
Write-Host "==> Version install scenario PASSED" -ForegroundColor Green
