#Requires -Version 5.1

<#
.SYNOPSIS
    Idempotent scenario - Determinism test

.DESCRIPTION
    Verifies:
    - Second apply does not change state
    - No duplicate packages
    - No duplicate files
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$StateFile = "state\state.json"
$FixturesDir = Join-Path $PSScriptRoot "..\..\..\..\fixtures\configs"

# Use test-specific strategy (no backup, standard backends)
$global:LOADOUT_STRATEGY_FILE = (Resolve-Path (Join-Path $FixturesDir "strategy.yaml")).Path
$ProfileBase = (Resolve-Path (Join-Path $FixturesDir "profile-base.yaml")).Path

Write-Host "==> Idempotent scenario" -ForegroundColor Cyan
Write-Host ""

Write-Host "==> First apply" -ForegroundColor Green
.\loadout.ps1 apply $ProfileBase

if ($LASTEXITCODE -ne 0) {
    throw "First apply failed"
}

Write-Host ""
Write-Host "==> Capturing first state" -ForegroundColor Green
$State1 = Get-Content $StateFile -Raw

Write-Host "==> Second apply" -ForegroundColor Green
.\loadout.ps1 apply $ProfileBase

if ($LASTEXITCODE -ne 0) {
    throw "Second apply failed"
}

Write-Host ""
Write-Host "==> Capturing second state" -ForegroundColor Green
$State2 = Get-Content $StateFile -Raw

Write-Host "==> Comparing states" -ForegroundColor Green
if ($State1 -ne $State2) {
    Write-Host "State changed between runs:" -ForegroundColor Red
    Write-Host "First state:" -ForegroundColor Yellow
    Write-Host $State1
    Write-Host "Second state:" -ForegroundColor Yellow
    Write-Host $State2
    throw "State changed on second apply (not idempotent)"
}

Write-Host "==> Checking for duplicate packages" -ForegroundColor Green
$StateObj = $State2 | ConvertFrom-Json
foreach ($featureName in $StateObj.features.PSObject.Properties.Name) {
    $feature = $StateObj.features.$featureName
    if ($feature.PSObject.Properties.Name -contains "packages") {
        $packages = $feature.packages
        $uniquePackages = $packages | Select-Object -Unique
        if ($packages.Count -ne $uniquePackages.Count) {
            throw "Duplicate packages found in feature: $featureName"
        }
    }
}

Write-Host "==> Checking for duplicate files" -ForegroundColor Green
foreach ($featureName in $StateObj.features.PSObject.Properties.Name) {
    $feature = $StateObj.features.$featureName
    if ($feature.PSObject.Properties.Name -contains "files") {
        $files = $feature.files
        $uniqueFiles = $files | Select-Object -Unique
        if ($files.Count -ne $uniqueFiles.Count) {
            throw "Duplicate files found in feature: $featureName"
        }
    }
}

Write-Host ""
Write-Host "==> Idempotent scenario PASSED" -ForegroundColor Green
