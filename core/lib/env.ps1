# -----------------------------------------------------------------------------
# Module: env
#
# Responsibility:
#   Define environment variables for loadout framework.
# -----------------------------------------------------------------------------

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Root directory of loadout
# Assumes this script is located at core/lib/env.ps1
$script:LOADOUT_ROOT = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$global:LOADOUT_ROOT = $script:LOADOUT_ROOT

# Platform detection
$global:LOADOUT_PLATFORM = "windows"

# XDG/AppData directories
$script:UserProfileBase = if ($env:USERPROFILE) { $env:USERPROFILE } else { $HOME }
$script:ConfigBase = if ($env:APPDATA) { $env:APPDATA } else { Join-Path $script:UserProfileBase "AppData\Roaming" }
$script:StateBase = if ($env:LOCALAPPDATA) { $env:LOCALAPPDATA } else { Join-Path $script:UserProfileBase "AppData\Local" }
$script:DataBase = if ($env:LOCALAPPDATA) { $env:LOCALAPPDATA } else { Join-Path $script:UserProfileBase "AppData\Local" }

$global:LOADOUT_CONFIG_HOME = Join-Path $script:ConfigBase "loadout"
$global:LOADOUT_STATE_HOME = Join-Path $script:StateBase "loadout"
$global:LOADOUT_DATA_HOME = Join-Path $script:DataBase "loadout"

# Get-DotfilesStateFilePath
# Return authoritative state file path.
function Get-DotfilesStateFilePath {
	return Join-Path $global:LOADOUT_STATE_HOME "state.json"
}

# Features directory
$global:LOADOUT_FEATURES_DIR = Join-Path $LOADOUT_ROOT "features"

# Maximum supported feature spec_version
$global:SUPPORTED_FEATURE_SPEC_VERSION = 1

# Profiles directory (override allowed)
if (-not (Get-Variable -Scope Global -Name LOADOUT_PROFILES_DIR -ErrorAction SilentlyContinue) -and -not $env:LOADOUT_PROFILES_DIR) {
	$global:LOADOUT_PROFILES_DIR = Join-Path $global:LOADOUT_CONFIG_HOME "profiles"
} elseif (-not (Get-Variable -Scope Global -Name LOADOUT_PROFILES_DIR -ErrorAction SilentlyContinue) -and $env:LOADOUT_PROFILES_DIR) {
	$global:LOADOUT_PROFILES_DIR = $env:LOADOUT_PROFILES_DIR
}

# Source registry file (override allowed)
if (-not (Get-Variable -Scope Global -Name LOADOUT_SOURCES_FILE -ErrorAction SilentlyContinue) -and -not $env:LOADOUT_SOURCES_FILE) {
	$global:LOADOUT_SOURCES_FILE = Join-Path $global:LOADOUT_CONFIG_HOME "sources.yaml"
} elseif (-not (Get-Variable -Scope Global -Name LOADOUT_SOURCES_FILE -ErrorAction SilentlyContinue) -and $env:LOADOUT_SOURCES_FILE) {
	$global:LOADOUT_SOURCES_FILE = $env:LOADOUT_SOURCES_FILE
}

# Backend plugins directory
$global:LOADOUT_BACKENDS_DIR = Join-Path $LOADOUT_ROOT "backends"

# ---------------------------------------------------------------------------
# PS 5.1 compatibility helpers
#
# _Prop  - null-safe PSObject property access  ($obj.PSObject.Properties[key]?.Value)
# _Coal  - null-coalescing operator             ($a ?? $b)
# ---------------------------------------------------------------------------
function _Prop {
    param($Obj, $Key)
    $p = $Obj.PSObject.Properties[$Key]
    if ($null -ne $p) { $p.Value } else { $null }
}

function _Coal {
    param($a, $b)
    if ($null -ne $a) { $a } else { $b }
}

# Policies directory and default policy file resolution
$global:LOADOUT_POLICIES_DIR = Join-Path $global:LOADOUT_CONFIG_HOME "policies"
if (-not (Get-Variable -Scope Global -Name LOADOUT_POLICY_FILE -ErrorAction SilentlyContinue) -and -not $env:LOADOUT_POLICY_FILE) {
	$policyCandidate = Join-Path $global:LOADOUT_POLICIES_DIR "default.$($global:LOADOUT_PLATFORM).yaml"
	if (Test-Path $policyCandidate) {
		$global:LOADOUT_POLICY_FILE = $policyCandidate
	} else {
		$global:LOADOUT_POLICY_FILE = Join-Path $global:LOADOUT_POLICIES_DIR "default.yaml"
	}
} elseif (-not (Get-Variable -Scope Global -Name LOADOUT_POLICY_FILE -ErrorAction SilentlyContinue) -and $env:LOADOUT_POLICY_FILE) {
	$global:LOADOUT_POLICY_FILE = $env:LOADOUT_POLICY_FILE
}
