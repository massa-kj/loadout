# python installation script for Windows

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Load core libraries
$ScriptDir = $PSScriptRoot
$DotfilesRoot = Split-Path -Parent (Split-Path -Parent $ScriptDir)

. "$DotfilesRoot\core\lib\env.ps1"
. "$DotfilesRoot\core\lib\logger.ps1"

$FeatureName = "python"

Log-Task "Installing feature: $FeatureName"

# Read version from profile (default: latest)
$Version = if ($env:LOADOUT_FEATURE_CONFIG_VERSION) { $env:LOADOUT_FEATURE_CONFIG_VERSION } else { "latest" }

# Runtime installation is handled by executor (declared in feature.yaml).
# uv packages are managed by the separate `uv` feature.

Log-Success "Feature $FeatureName installed successfully"
