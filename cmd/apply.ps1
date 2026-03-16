# cmd/apply.ps1 — CLI entry point for the apply command.
#
# This script is intentionally thin: argument parsing, platform guard,
# library loading, then a single call to Invoke-OrchestratorApply.
# All pipeline logic lives in core/lib/orchestrator.ps1.

[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [string]$ProfileFile
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
. "$global:LOADOUT_ROOT\core\lib\backend_registry.ps1"
. "$global:LOADOUT_ROOT\core\lib\planner.ps1"
. "$global:LOADOUT_ROOT\core\lib\policy_resolver.ps1"
. "$global:LOADOUT_ROOT\core\lib\declarative_executor.ps1"
. "$global:LOADOUT_ROOT\core\lib\executor.ps1"
. "$global:LOADOUT_ROOT\core\lib\resolver.ps1"
. "$global:LOADOUT_ROOT\core\lib\orchestrator.ps1"

# ── Platform guard ────────────────────────────────────────────────────────────

if ($global:LOADOUT_PLATFORM -ne "windows") {
    Log-Error "This script is for Windows only. On Linux/WSL, run loadout instead."
    exit 1
}

# ── Usage ─────────────────────────────────────────────────────────────────────

function _Show-ApplyUsage {
    Write-Host @"
Usage: loadout.ps1 apply <profile.yaml>

Apply a loadout profile to the system.

Arguments:
  profile.yaml    Path to the profile file

Examples:
  loadout.ps1 apply profiles\windows.yaml
"@
    exit 1
}

# ── Argument parsing ──────────────────────────────────────────────────────────

if ([string]::IsNullOrWhiteSpace($ProfileFile)) {
    _Show-ApplyUsage
}

Log-Task "Applying profile: $ProfileFile"

# ── Delegate to orchestrator ──────────────────────────────────────────────────

Invoke-OrchestratorApply -ProfileFile $ProfileFile
