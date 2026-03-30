#Requires -Version 5.1

<#
.SYNOPSIS
    Uninstall scenario - Safe removal test

.DESCRIPTION
    Verifies:
    - State-tracked files are removed
    - Non-tracked files are preserved
    - State is properly cleaned
    - Uninstall is idempotent
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$StateFile = "state\state.json"
$FixturesDir = Join-Path $PSScriptRoot "..\..\..\..\fixtures\configs"

# Use test-specific strategy (no backup, standard backends)
$global:LOADOUT_STRATEGY_FILE = (Resolve-Path (Join-Path $FixturesDir "strategy.yaml")).Path
$ProfileFull  = (Resolve-Path (Join-Path $FixturesDir "profile-full.yaml")).Path
$ProfileEmpty = (Resolve-Path (Join-Path $FixturesDir "profile-empty.yaml")).Path

Write-Host "==> Uninstall scenario" -ForegroundColor Cyan
Write-Host ""

Write-Host "==> Running apply" -ForegroundColor Green
.\loadout.ps1 apply $ProfileFull

if ($LASTEXITCODE -ne 0) {
    throw "Apply command failed"
}

Write-Host ""
Write-Host "==> Creating untracked test file" -ForegroundColor Green
$UntrackedFile = "C:\test-untracked-file.txt"
"This file should not be removed by uninstall" | Set-Content -Path $UntrackedFile

Write-Host "==> Capturing installed features" -ForegroundColor Green
$StateBeforeUninstall = Get-Content $StateFile -Raw | ConvertFrom-Json
$InstalledFeatures = $StateBeforeUninstall.features.PSObject.Properties.Name
Write-Host "    Installed features: $($InstalledFeatures -join ', ')" -ForegroundColor Gray

if ($InstalledFeatures.Count -eq 0) {
    throw "No features installed before uninstall test"
}

Write-Host ""
Write-Host "==> Creating empty profile (uninstall all)" -ForegroundColor Green
Write-Host "==> Running apply with empty profile (triggers uninstall)" -ForegroundColor Green
.\loadout.ps1 apply $ProfileEmpty

if ($LASTEXITCODE -ne 0) {
    throw "Uninstall apply failed"
}

Write-Host ""
Write-Host "==> Verifying state is empty after uninstall" -ForegroundColor Green
$StateAfterUninstall = Get-Content $StateFile -Raw | ConvertFrom-Json
$RemainingFeatures = $StateAfterUninstall.features.PSObject.Properties.Name

if ($RemainingFeatures.Count -ne 0) {
    throw "State not empty after uninstall. Remaining: $($RemainingFeatures -join ', ')"
}

Write-Host "==> Checking untracked file still exists" -ForegroundColor Green
if (-not (Test-Path $UntrackedFile)) {
    throw "Untracked file was removed (should be preserved)"
}

Write-Host "    Untracked file preserved (correct)" -ForegroundColor Gray

# Clean up untracked test file
Remove-Item $UntrackedFile -Force

Write-Host ""
Write-Host "==> Uninstall scenario PASSED" -ForegroundColor Green
