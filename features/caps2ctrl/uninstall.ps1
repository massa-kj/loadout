# Caps Lock to Ctrl remapping uninstallation script for Windows

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Load core libraries
$ScriptDir = $PSScriptRoot
$DotfilesRoot = Split-Path -Parent (Split-Path -Parent $ScriptDir)

. "$DotfilesRoot\core\lib\env.ps1"
. "$DotfilesRoot\core\lib\logger.ps1"
. "$DotfilesRoot\core\lib\runner.ps1"

$FeatureName = "caps2ctrl"

Log-Task "Uninstalling feature: $FeatureName"

# Check if running as Administrator
if (-not (Test-Administrator)) {
    Log-Error "This feature requires Administrator privileges"
    Log-Info "Please restart PowerShell as Administrator"
    exit 1
}

# Registry key path
$registryPath = "HKLM:\SYSTEM\CurrentControlSet\Control\Keyboard Layout"
$registryName = "Scancode Map"

# Remove the registry value
Log-Info "Removing Caps Lock to Ctrl mapping..."

try {
    $exists = Get-ItemProperty -Path $registryPath -Name $registryName -ErrorAction SilentlyContinue
    
    if ($exists) {
        Remove-ItemProperty -Path $registryPath -Name $registryName -Force
        Log-Success "Registry value removed successfully"
        Log-Warn "You need to restart your computer for the changes to take effect"
    } else {
        Log-Info "Registry value not found (already removed)"
    }
} catch {
    Log-Error "Failed to remove registry value: $_"
    # Don't exit with error, continue to remove from state
}

Log-Success "Feature $FeatureName uninstalled successfully"
Write-Host ""
Write-Host "IMPORTANT: Restart your computer to restore default keyboard mapping" -ForegroundColor Yellow
Write-Host ""
