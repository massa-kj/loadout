# Visual C++ Redistributable installation script for Windows

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Load core libraries
$ScriptDir = $PSScriptRoot
$DotfilesRoot = Split-Path -Parent (Split-Path -Parent $ScriptDir)

. "$DotfilesRoot\core\lib\env.ps1"
. "$DotfilesRoot\core\lib\logger.ps1"

$FeatureName = "vcredist"

Log-Task "Installing feature: $FeatureName"

# Package installation is handled by executor (declared in feature.yaml).

Log-Success "Feature $FeatureName installed successfully"
