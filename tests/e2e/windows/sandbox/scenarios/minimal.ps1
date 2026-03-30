#Requires -Version 5.1

<#
.SYNOPSIS
    Minimal scenario - Basic execution test

.DESCRIPTION
    Verifies:
    - State file is created
    - Version field is correct
    - Features are recorded
    - No duplicates exist
    - All paths are absolute
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$StateFile = "state\state.json"
$FixturesDir = Join-Path $PSScriptRoot "..\..\..\..\fixtures\configs"

# Use test-specific strategy (no backup, standard backends)
$global:LOADOUT_STRATEGY_FILE = (Resolve-Path (Join-Path $FixturesDir "strategy.yaml")).Path
$ProfileBase = (Resolve-Path (Join-Path $FixturesDir "profile-base.yaml")).Path

Write-Host "==> Minimal scenario" -ForegroundColor Cyan
Write-Host ""

Write-Host "==> Running apply" -ForegroundColor Green
.\loadout.ps1 apply $ProfileBase

if ($LASTEXITCODE -ne 0) {
    throw "Apply command failed"
}

Write-Host ""
Write-Host "==> Checking state file existence" -ForegroundColor Green
if (-not (Test-Path $StateFile)) {
    throw "State file not created: $StateFile"
}

Write-Host "==> Validating JSON format" -ForegroundColor Green
try {
    $State = Get-Content $StateFile -Raw | ConvertFrom-Json
} catch {
    throw "Invalid JSON format: $_"
}

Write-Host "==> Checking version field" -ForegroundColor Green
if ($State.version -ne 1) {
    throw "Invalid state version: $($State.version)"
}

Write-Host "==> Checking features object exists" -ForegroundColor Green
if (-not $State.PSObject.Properties.Name -contains "features") {
    throw "Features object not found in state"
}

Write-Host "==> Checking no duplicate features" -ForegroundColor Green
$FeatureNames = $State.features.PSObject.Properties.Name
$UniqueNames = $FeatureNames | Select-Object -Unique
if ($FeatureNames.Count -ne $UniqueNames.Count) {
    throw "Duplicate features detected"
}

Write-Host "==> Checking absolute paths in files" -ForegroundColor Green
foreach ($featureName in $State.features.PSObject.Properties.Name) {
    $feature = $State.features.$featureName
    if ($feature.PSObject.Properties.Name -contains "files") {
        foreach ($file in $feature.files) {
            # Windows absolute paths: drive letter (C:\) or registry paths (HKLM:\, HKCU:\)
            if ($file -notmatch '^([A-Z]:\\|HK(LM|CU|CR|U|CC|PD):\\)') {
                throw "Non-absolute path detected: $file"
            }
        }
    }
}

Write-Host ""
Write-Host "==> Minimal scenario PASSED" -ForegroundColor Green
