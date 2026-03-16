#!/usr/bin/env pwsh
<#
.SYNOPSIS
Migrate the loadout state file to the latest schema version (v3).

.DESCRIPTION
Supports migration paths:
  v1 → v2 → v3   (via existing State-Migrate + State-MigrateV2ToV3)
  v2 → v3
  v3              (already current; nothing to do)

This command reads the state file DIRECTLY (bypassing State-Load, which
rejects v1/v2) so that migration can proceed even on outdated state.

.PARAMETER DryRun
Show what would change without committing.

.PARAMETER Profiles
Also normalize feature keys in profiles/*.yaml (bare → core/<name>).

.EXAMPLE
loadout migrate
loadout migrate -DryRun
loadout migrate -Profiles
#>

param(
    [switch]$DryRun,
    [switch]$Profiles
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:PSScriptFilePath = Split-Path -Parent $MyInvocation.MyCommand.Path
$LOADOUT_ROOT = Split-Path -Parent $script:PSScriptFilePath
$global:LOADOUT_ROOT = $LOADOUT_ROOT
$env:LOADOUT_ROOT = $LOADOUT_ROOT

# ── Library loading ───────────────────────────────────────────────────────────

try {
    . "$LOADOUT_ROOT\core\lib\env.ps1"
    . "$LOADOUT_ROOT\core\lib\logger.ps1"
    . "$LOADOUT_ROOT\core\lib\state.ps1"
} catch {
    Write-Error "Failed to load library: $_"
    exit 1
}

# ── Platform guard ────────────────────────────────────────────────────────────

switch ($env:LOADOUT_PLATFORM) {
    'windows' { }
    default {
        Log-Error "On Linux/WSL, use loadout migrate instead."
        exit 1
    }
}

# ── Helpers ───────────────────────────────────────────────────────────────────

function Show-MigrationDiff {
    param([object]$Before, [object]$After)

    $beforeKeys = @($Before.features.PSObject.Properties | ForEach-Object { $_.Name } | Sort-Object)
    $afterKeys  = @($After.features.PSObject.Properties | ForEach-Object { $_.Name } | Sort-Object)

    if (@(Compare-Object $beforeKeys $afterKeys).Count -eq 0) {
        Log-Info "  No feature key changes."
        return
    }

    Log-Info "  Feature key changes:"
    $removed = Compare-Object $beforeKeys $afterKeys | Where-Object { $_.SideIndicator -eq '<=' }
    foreach ($item in $removed) {
        Log-Info "    - $($item.InputObject)"
    }
    $added = Compare-Object $beforeKeys $afterKeys | Where-Object { $_.SideIndicator -eq '=>' }
    foreach ($item in $added) {
        Log-Info "    + $($item.InputObject)"
    }
}

function Update-ProfileFeatures {
    $profileDir = $global:LOADOUT_PROFILES_DIR
    if (-not $profileDir) {
        Log-Error "migrate -Profiles: LOADOUT_PROFILES_DIR is not set"
        return $false
    }
    if (-not (Test-Path $profileDir)) {
        Log-Warn "migrate: profiles directory not found: $profileDir (skipping)"
        return $true
    }

    if (-not (Get-Command yq -ErrorAction SilentlyContinue)) {
        Log-Error "migrate -Profiles: 'yq' is required but not installed."
        Log-Error "  Install via: mise install yq  or  choco install yq"
        return $false
    }

    foreach ($profileFile in (Get-ChildItem "$profileDir\*.yaml")) {
        Log-Info "  Processing profile: $($profileFile.Name)"

        if ($DryRun) {
            Log-Info "    (dry-run) would normalize bare names to canonical IDs in: $($profileFile.Name)"
        } else {
            try {
                # Use yq to rewrite feature names
                & yq eval '
                    .features = (.features | map(
                        if type == "string" then
                            if contains("/") then . else "core/" + . end
                        elif type == "object" then
                            if (.name | contains("/")) then . else .name = "core/" + .name end
                        else .
                        end
                    ))
                ' -i $profileFile.FullName
                Log-Success "  Updated: $($profileFile.Name)"
            } catch {
                Log-Error "migrate: failed to update profile $($profileFile.Name): $_"
                return $false
            }
        }
    }
    return $true
}

# ── Main ──────────────────────────────────────────────────────────────────────

Log-Task "Migrating loadout state"

# 1. Read the state file directly (do NOT call State-Init)
if (-not (Get-Command Get-DotfilesStateFilePath -ErrorAction SilentlyContinue)) {
    Log-Error "migrate: Get-DotfilesStateFilePath is not available"
    exit 1
}

$statePath = Get-DotfilesStateFilePath
$legacyStatePath = Join-Path $LOADOUT_ROOT "state\state.json"

$stateDir = Split-Path -Parent $statePath
if (-not (Test-Path $stateDir)) {
    New-Item -ItemType Directory -Path $stateDir -Force | Out-Null
}

# Legacy state physical move (copy + backup, keep original file).
if (-not (Test-Path $statePath) -and (Test-Path $legacyStatePath)) {
    $timestamp = Get-Date -Format 'yyyyMMdd_HHmmss'
    $legacyBackup = "$legacyStatePath.bak.$timestamp"

    Copy-Item -Path $legacyStatePath -Destination $legacyBackup -Force
    Copy-Item -Path $legacyStatePath -Destination $statePath -Force

    Log-Info "Legacy state copied: $legacyStatePath -> $statePath"
    Log-Info "Legacy backup created: $legacyBackup"
}

if (-not (Test-Path $statePath)) {
    Log-Info "No state file found at $statePath — nothing to migrate."
    Log-Info "A fresh v3 state will be created on the next 'loadout apply'."
    exit 0
}

try {
    $stateJson = Get-Content -Path $statePath -Raw | ConvertFrom-Json
} catch {
    Log-Error "migrate: state file is not valid JSON: $statePath"
    exit 1
}

# Keep the JSON for later use in State-MigrateV2ToV3
$script:StateData = $stateJson

# 2. Check current version
$currentVer = if ($stateJson.version) { $stateJson.version } else { "unknown" }

Log-Info "Current state version: $currentVer"
Log-Info "Target  state version: 3"

if ($currentVer -eq 3) {
    Log-Success "State is already at v3 — nothing to migrate."
    if ($Profiles) {
        Log-Task "Normalizing profile feature names"
        Update-ProfileFeatures | Out-Null
    }
    exit 0
}

# 3. Chain v1 → v2 → v3 if needed
if ($currentVer -eq 1) {
    Log-Info "Migrating: v1 → v2..."

    # Call State-Migrate (which works on $script:StateData)
    if (-not (State-Migrate)) {
        Log-Error "migrate: v1 → v2 migration failed"
        exit 1
    }

    Log-Success "  v1 → v2 transformation complete."
    $currentVer = 2
}

if ($currentVer -ne 2) {
    Log-Error "migrate: unexpected state version: $currentVer (expected 1 or 2)"
    exit 1
}

# 4. Show diff before transformation
Log-Info "Migrating: v2 → v3..."
Show-MigrationDiff -Before $script:StateData -After $($script:StateData | & {
    param($obj)
    # Simulate what State-MigrateV2ToV3 will do
    $v3 = _Invoke-TransformV2ToV3 -V2Object $obj
    $v3
})

# 5. Handle --dry-run
if ($DryRun) {
    Log-Info ""
    Log-Info "[dry-run] No changes written."
    if ($Profiles) {
        Log-Task "Normalizing profile feature names (dry-run)"
        Update-ProfileFeatures | Out-Null
    }
    exit 0
}

# 6. Commit atomically via State-MigrateV2ToV3
Log-Info "Writing migrated state..."
if (-not (State-MigrateV2ToV3)) {
    Log-Error "migrate: commit failed"
    exit 1
}

Log-Success "State successfully migrated to v3."

# 7. Optional profile normalization
if ($Profiles) {
    Log-Task "Normalizing profile feature names"
    if (-not (Update-ProfileFeatures)) {
        exit 1
    }
}

exit 0
