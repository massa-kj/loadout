# Scoop package manager installation script for Windows

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Load core libraries
$ScriptDir = $PSScriptRoot
$DotfilesRoot = Split-Path -Parent (Split-Path -Parent $ScriptDir)

. "$DotfilesRoot\core\lib\env.ps1"
. "$DotfilesRoot\core\lib\logger.ps1"
. "$DotfilesRoot\core\lib\state.ps1"
. "$DotfilesRoot\core\lib\runner.ps1"

$FeatureName = "scoop"

Log-Task "Installing feature: $FeatureName"

function Register-ScoopState {
    $resource = [PSCustomObject]@{
        kind    = "package"
        id      = "pkg:scoop"
        backend = "unknown"
        package = [PSCustomObject]@{ name = "scoop"; version = $null }
    }

    State-PatchBegin
    State-PatchAddResource -Feature $FeatureName -ResourceObject $resource
    State-PatchFinalize | Out-Null
}

# Check if Scoop is already installed (idempotency)
if (Test-Command "scoop") {
    Log-Info "Scoop is already installed"
    $scoopVersion = scoop --version 2>&1 | Select-Object -First 1
    Log-Info "Current version: $scoopVersion"
    
    # Ensure buckets are added
    $bucketsToAdd = @("main", "extras")
    $existingBuckets = @()
    
    try {
        $bucketList = scoop bucket list 2>&1
        if ($LASTEXITCODE -eq 0) {
            $existingBuckets = $bucketList | Where-Object { $_ -match "^\s*\w+" } | ForEach-Object { 
                ($_ -split '\s+')[0].Trim() 
            }
        }
    } catch {
        Log-Warn "Could not retrieve bucket list: $_"
    }
    
    foreach ($bucket in $bucketsToAdd) {
        if ($existingBuckets -contains $bucket) {
            Log-Info "Bucket '$bucket' is already added"
        } else {
            Log-Info "Adding bucket '$bucket'..."
            try {
                scoop bucket add $bucket 2>&1 | Out-Null
                if ($LASTEXITCODE -eq 0) {
                    Log-Success "Bucket '$bucket' added successfully"
                } else {
                    Log-Warn "Failed to add bucket '$bucket' (exit code: $LASTEXITCODE)"
                }
            } catch {
                Log-Warn "Failed to add bucket '$bucket': $_"
            }
        }
    }
    
    Register-ScoopState
    
    Log-Success "Feature $FeatureName is already configured"
    exit 0
}

# Install Scoop
Log-Info "Installing Scoop package manager..."

try {
    # Download and execute Scoop installer
    Log-Info "Downloading Scoop installer..."
    $installScript = Invoke-RestMethod -Uri "https://get.scoop.sh" -UseBasicParsing
    
    if ([string]::IsNullOrWhiteSpace($installScript)) {
        Log-Error "Failed to download Scoop installer script"
        exit 1
    }
    
    Log-Info "Executing Scoop installer..."
    Invoke-Expression $installScript
    
    if ($LASTEXITCODE -ne 0) {
        Log-Error "Scoop installation failed with exit code: $LASTEXITCODE"
        exit 1
    }
    
} catch {
    Log-Error "Failed to install Scoop: $_"
    exit 1
}

# Verify installation
if (-not (Test-Command "scoop")) {
    Log-Error "Scoop command not found after installation"
    Log-Info "You may need to restart your PowerShell session"
    exit 1
}

$scoopVersion = scoop --version 2>&1 | Select-Object -First 1
Log-Success "Scoop installed successfully: $scoopVersion"

# Add main and extras buckets
$bucketsToAdd = @("main", "extras")

foreach ($bucket in $bucketsToAdd) {
    Log-Info "Adding bucket '$bucket'..."
    try {
        scoop bucket add $bucket 2>&1 | Out-Null
        if ($LASTEXITCODE -eq 0) {
            Log-Success "Bucket '$bucket' added successfully"
        } else {
            Log-Warn "Failed to add bucket '$bucket' (exit code: $LASTEXITCODE)"
        }
    } catch {
        Log-Error "Failed to add bucket '$bucket': $_"
    }
}

# Add to state
Register-ScoopState

Log-Success "Feature $FeatureName installed successfully"
Write-Host ""
Write-Host "Scoop package manager has been installed with main and extras buckets." -ForegroundColor Green
Write-Host "You can now install packages using: scoop install <package-name>" -ForegroundColor Cyan
Write-Host ""
