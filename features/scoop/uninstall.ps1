# Scoop package manager uninstallation script for Windows

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Load core libraries
$ScriptDir = $PSScriptRoot
$DotfilesRoot = Split-Path -Parent (Split-Path -Parent $ScriptDir)

. "$DotfilesRoot\core\lib\env.ps1"
. "$DotfilesRoot\core\lib\logger.ps1"
. "$DotfilesRoot\core\lib\runner.ps1"

$FeatureName = "scoop"

Log-Task "Uninstalling feature: $FeatureName"

# Check if Scoop is installed
if (-not (Test-Command "scoop")) {
    Log-Warn "Scoop command not found, skipping secondary cleanup"
    exit 0
}

# Warning: Uninstalling Scoop will remove all installed packages
Write-Host ""
Write-Host "WARNING: Uninstalling Scoop will remove ALL packages installed via Scoop!" -ForegroundColor Red
Write-Host ""

$confirmation = Get-UserConfirmation -Message "Are you sure you want to uninstall Scoop?"
if (-not $confirmation) {
    Log-Info "Scoop uninstallation cancelled by user"
    exit 0
}

# Uninstall Scoop
Log-Info "Uninstalling Scoop..."

try {
    # Scoop uninstall command
    scoop uninstall scoop 2>&1 | Out-Null
    
    # Remove Scoop directory if still exists
    $scoopPath = Join-Path $env:USERPROFILE "scoop"
    if (Test-Path $scoopPath) {
        Log-Info "Removing Scoop directory: $scoopPath"
        Remove-Item -Path $scoopPath -Recurse -Force -ErrorAction SilentlyContinue
    }
    
    Log-Success "Scoop uninstalled successfully"
} catch {
    Log-Error "Failed to uninstall Scoop: $_"
    Log-Info "Manual cleanup may be required"
}

Log-Success "Feature $FeatureName uninstalled successfully"
Write-Host ""
Write-Host "Scoop has been removed from your system." -ForegroundColor Green
Write-Host "You may need to restart your PowerShell session." -ForegroundColor Yellow
Write-Host ""
