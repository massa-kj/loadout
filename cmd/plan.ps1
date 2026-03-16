# cmd/plan.ps1 — CLI entry point for the plan command (Windows).
#
# Runs the full planning pipeline (profile → diff → classify → decide)
# without executing any changes. State is never modified.
#
# Usage:
#   loadout.ps1 plan <profile.yaml> [--show-noop]
#
# Output:
#   Actions that would be taken, colored by operation type.
#   noop entries are hidden unless --show-noop is specified.
#
# Exit codes:
#   0  — plan printed (may be all-noop)
#   1  — error (profile not found, resolver failure, etc.)

[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [string]$ProfileFile,

    [switch]$ShowNoop
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# ── Library loading ───────────────────────────────────────────────────────────

$ScriptRoot = $PSScriptRoot
$global:LOADOUT_ROOT = (Get-Item "$ScriptRoot\..").FullName

. "$global:LOADOUT_ROOT\core\lib\env.ps1"
. "$global:LOADOUT_ROOT\core\lib\logger.ps1"
. "$global:LOADOUT_ROOT\core\lib\state.ps1"
. "$global:LOADOUT_ROOT\core\lib\runner.ps1"
. "$global:LOADOUT_ROOT\core\lib\source_registry.ps1"
. "$global:LOADOUT_ROOT\core\lib\feature_index.ps1"
. "$global:LOADOUT_ROOT\core\lib\compiler.ps1"
. "$global:LOADOUT_ROOT\core\lib\resolver.ps1"
. "$global:LOADOUT_ROOT\core\lib\orchestrator.ps1"

# ── Platform guard ────────────────────────────────────────────────────────────

if ($global:LOADOUT_PLATFORM -ne "windows") {
    Log-Error "This script is for Windows only. On Linux/WSL, run loadout instead."
    exit 1
}

# ── Usage ─────────────────────────────────────────────────────────────────────

function _Show-PlanUsage {
    Write-Host @"
Usage: loadout.ps1 plan <profile.yaml> [-ShowNoop]

Show what 'apply' would do without making any changes.

Arguments:
  profile.yaml    Path to the profile file

Options:
  -ShowNoop       Also list noop (already up-to-date) features

Exit codes:
  0  Plan displayed successfully
  1  Error

Examples:
  loadout.ps1 plan profiles\windows.yaml
  loadout.ps1 plan profiles\windows.yaml -ShowNoop
"@
    exit 1
}

# ── Argument parsing ──────────────────────────────────────────────────────────

if ([string]::IsNullOrWhiteSpace($ProfileFile)) {
    _Show-PlanUsage
}

# ── Plan formatter ────────────────────────────────────────────────────────────

# Format-Plan <PlanJson> <ProfileFile> <ShowNoop>
# Format and print plan JSON to the console.
function Format-Plan {
    param(
        [Parameter(Mandatory=$true)]
        [string]$PlanJson,

        [Parameter(Mandatory=$true)]
        [string]$Profile,

        [bool]$ShowNoop = $false
    )

    $plan = $PlanJson | ConvertFrom-Json

    Write-Host ""
    Write-Host "Plan: $Profile" -ForegroundColor White
    Write-Host ""

    $hasOutput = $false

    # Print active operations in action order (destroy → replace → create)
    foreach ($action in $plan.actions) {
        switch ($action.operation) {
            "destroy" {
                Write-Host ("  {0,-16} {1}" -f "destroy", $action.feature) -ForegroundColor Red
                $hasOutput = $true
            }
            "replace" {
                $from = if ($action.details.from_version) { $action.details.from_version } else { "" }
                $to   = if ($action.details.to_version)   { $action.details.to_version   } else { "" }
                if ($from -and $to) {
                    Write-Host ("  {0,-16} {1,-20} {2} → {3}" -f "replace", $action.feature, $from, $to) `
                        -ForegroundColor Yellow
                } else {
                    Write-Host ("  {0,-16} {1}" -f "replace", $action.feature) -ForegroundColor Yellow
                }
                $hasOutput = $true
            }
            "replace_backend" {
                Write-Host ("  {0,-16} {1}" -f "replace_backend", $action.feature) -ForegroundColor Yellow
                $hasOutput = $true
            }
            "strengthen" {
                $addCount = if ($action.details.add_resources) { @($action.details.add_resources).Count } else { 0 }
                Write-Host ("  {0,-16} {1,-20} (+{2} resource(s))" -f "strengthen", $action.feature, $addCount) `
                    -ForegroundColor Cyan
                $hasOutput = $true
            }
            "create" {
                Write-Host ("  {0,-16} {1}" -f "create", $action.feature) -ForegroundColor Green
                $hasOutput = $true
            }
        }
    }

    # Print blocked entries
    foreach ($item in $plan.blocked) {
        $reason = if ($item.reason) { $item.reason } else { "" }
        Write-Host ("  {0,-16} {1,-20} {2}" -f "blocked", $item.feature, $reason) -ForegroundColor Red
        $hasOutput = $true
    }

    # Print noop entries when --verbose
    if ($ShowNoop) {
        foreach ($item in $plan.noops) {
            Write-Host ("  {0,-16} {1}" -f "noop", $item.feature) -ForegroundColor DarkGray
            $hasOutput = $true
        }
    }

    if ($hasOutput) { Write-Host "" }

    $s = $plan.summary
    $strengthen = if ((_Prop $s 'strengthen')) { $s.strengthen } else { 0 }
    Write-Host ("Summary: create={0}  destroy={1}  replace={2}  strengthen={3}  noop={4}  blocked={5}" -f `
        $s.create, $s.destroy, $s.replace, $strengthen, $s.noop, $s.blocked) -ForegroundColor White
    Write-Host ""

    $blockedCount = $s.blocked
    $activeCount  = $s.create + $s.destroy + $s.replace + $strengthen

    if ($blockedCount -gt 0) {
        Write-Host "$blockedCount blocked feature(s) — run 'apply' to see details." -ForegroundColor Red
        Write-Host ""
    } elseif ($activeCount -eq 0) {
        Write-Host "Nothing to do." -ForegroundColor DarkGray
        Write-Host ""
    } else {
        Write-Host "Run 'loadout.ps1 apply $Profile' to apply these changes." -ForegroundColor White
        Write-Host ""
    }
}

# ── Plan pipeline ─────────────────────────────────────────────────────────────

Log-Task "Planning profile: $ProfileFile"

# Load backend policy (non-fatal if policies dir is absent)
Backend-Registry-LoadPolicy

# Initialise (or migrate) state — read-only after this point
if (-not (State-Init)) {
    Log-Error "Failed to initialise state"
    exit 1
}

# Parse profile
$desiredFeatures = Read-Profile -ProfileFile $ProfileFile
if ($null -eq $desiredFeatures) { exit 1 }

# Build Feature Index: scans all registered sources, enriches with metadata
$_planIndex = Invoke-FeatureIndexBuild
if ($null -eq $_planIndex) { exit 1 }

# Filter desired features: separates valid from spec_version-blocked
$_svResult = Invoke-FeatureIndexFilter -FeatureIndexJson $_planIndex -DesiredFeatures $desiredFeatures
if ($null -eq $_svResult) { exit 1 }

# Resolve feature metadata from index (no file I/O) + topological sort
if (-not (Read-FeatureMetadata -FeatureIndexJson $_planIndex -Features $_svResult.Valid)) { exit 1 }

$sortedFeatures = Resolve-Dependencies -DesiredFeatures $_svResult.Valid
if ($null -eq $sortedFeatures) { exit 1 }

# Compile raw DesiredResourceGraph (assigns stable resource IDs only)
$_planDrg = Invoke-FeatureCompilerRun -FeatureIndexJson $_planIndex -SortedFeatures $sortedFeatures
if ($null -eq $_planDrg) {
    Log-Error "Compiler failed to produce a DesiredResourceGraph"
    exit 1
}

# Resolve desired_backend per resource via PolicyResolver
$_planRrg = Invoke-PolicyResolverRun -DrgJson $_planDrg
if ($null -eq $_planRrg) {
    Log-Error "PolicyResolver failed to produce a ResolvedResourceGraph"
    exit 1
}

# Plan: pure computation — no state writes
$planJson = Invoke-PlannerRun -DrgJson $_planRrg -SortedFeatures $sortedFeatures -ProfileFile $ProfileFile
if (-not $planJson) {
    Log-Error "Planner failed to produce a plan"
    exit 1
}
$planJson = Invoke-PlanInjectBlocked -PlanJson $planJson -BlockedExtraJson $_svResult.BlockedJson

# Display
Format-Plan -PlanJson $planJson -Profile $ProfileFile -ShowNoop $ShowNoop.IsPresent
