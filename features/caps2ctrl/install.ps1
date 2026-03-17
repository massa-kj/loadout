# Caps Lock to Ctrl remapping installation script for Windows

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Load core libraries
$ScriptDir = $PSScriptRoot
$DotfilesRoot = Split-Path -Parent (Split-Path -Parent $ScriptDir)

. "$DotfilesRoot\core\lib\env.ps1"
. "$DotfilesRoot\core\lib\logger.ps1"
. "$DotfilesRoot\core\lib\runner.ps1"

$FeatureName = "caps2ctrl"

Log-Task "Installing feature: $FeatureName"

# Check if running as Administrator
if (-not (Test-Administrator)) {
    Log-Error "This feature requires Administrator privileges"
    Log-Info "Please restart PowerShell as Administrator"
    exit 1
}

# Registry key path
$registryPath = "HKLM:\SYSTEM\CurrentControlSet\Control\Keyboard Layout"
$registryName = "Scancode Map"

# Check if already configured (idempotency)
$currentValue = $null
try {
    $currentValue = Get-ItemProperty -Path $registryPath -Name $registryName -ErrorAction SilentlyContinue
} catch {
    # Key doesn't exist, that's fine
}

# Expected scancode map value (Caps Lock -> Ctrl)
$expectedValue = @(
    0x00, 0x00, 0x00, 0x00,  # Header: Version
    0x00, 0x00, 0x00, 0x00,  # Header: Flags
    0x02, 0x00, 0x00, 0x00,  # Entry count (2 entries including null terminator)
    0x1D, 0x00, 0x3A, 0x00,  # Map Caps Lock (0x3A) to Left Ctrl (0x1D)
    0x00, 0x00, 0x00, 0x00   # Null terminator
)

if ($currentValue -and $currentValue.$registryName) {
    # Compare byte arrays
    $isSame = $true
    if ($currentValue.$registryName.Length -eq $expectedValue.Length) {
        for ($i = 0; $i -lt $expectedValue.Length; $i++) {
            if ($currentValue.$registryName[$i] -ne $expectedValue[$i]) {
                $isSame = $false
                break
            }
        }
    } else {
        $isSame = $false
    }
    
    if ($isSame) {
        Log-Info "Caps Lock to Ctrl mapping is already configured"
        Log-Success "Feature $FeatureName installed successfully (already configured)"
        exit 0
    } else {
        Log-Warn "Different keyboard mapping detected, will update to Caps Lock -> Ctrl"
    }
}

# Apply registry settings
Log-Info "Applying Caps Lock to Ctrl mapping..."

$regFilePath = Join-Path (Join-Path $ScriptDir "files") "Caps2Ctrl.reg"

if (-not (Test-Path $regFilePath)) {
    Log-Error "Registry file not found: $regFilePath"
    exit 1
}

try {
    # Import registry file
    Log-Info "Importing registry settings..."
    $process = Start-Process -FilePath "regedit.exe" -ArgumentList @("/s", $regFilePath) -PassThru -Wait
    
    if ($process.ExitCode -ne 0) {
        throw "regedit failed with exit code $($process.ExitCode)"
    }
    
    Log-Success "Registry settings applied successfully"
    Log-Warn "You need to restart your computer for the changes to take effect"
    
} catch {
    Log-Error "Failed to apply registry settings: $_"
    exit 1
}

Log-Success "Feature $FeatureName installed successfully"
Write-Host ""
Write-Host "IMPORTANT: Restart your computer to apply the keyboard mapping" -ForegroundColor Yellow
Write-Host ""
