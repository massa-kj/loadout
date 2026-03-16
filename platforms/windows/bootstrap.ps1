# Minimal setup for running loadout in a Windows environment

#Requires -Version 5.1

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Color output functions
function Write-Step {
    param([string]$Message)
    Write-Host "==> " -ForegroundColor Green -NoNewline
    Write-Host $Message
}

function Write-Info {
    param([string]$Message)
    Write-Host "[INFO] " -ForegroundColor Blue -NoNewline
    Write-Host $Message
}

function Write-Warn {
    param([string]$Message)
    Write-Host "[WARN] " -ForegroundColor Yellow -NoNewline
    Write-Host $Message
}

function Write-Error {
    param([string]$Message)
    Write-Host "[ERROR] " -ForegroundColor Red -NoNewline
    Write-Host $Message
}

# Detect LOADOUT_ROOT
$ScriptDir = $PSScriptRoot
$DotfilesRoot = Split-Path -Parent (Split-Path -Parent $ScriptDir)
$env:LOADOUT_ROOT = $DotfilesRoot

Write-Info "LOADOUT_ROOT: $DotfilesRoot"
Write-Info "Platform: Windows"

# Check if running on Windows
# Note: Platform property only exists in PowerShell Core (6+)
if ($PSVersionTable.PSObject.Properties.Name -contains "Platform") {
    if ($PSVersionTable.Platform -ne "Win32NT") {
        Write-Warn "Not running on Windows, but continuing..."
    }
}

# Check administrator privileges
$isAdmin = ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Write-Warn "Not running as Administrator"
    Write-Info "Some operations may require elevated privileges"
}

# Check winget availability
Write-Step "Checking package manager..."
if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
    Write-Error "winget is not available"
    Write-Info "winget is required and should be pre-installed on Windows 10 (1809+) / Windows 11"
    Write-Info "For manual installation, visit: https://aka.ms/getwinget"
    exit 1
} else {
    Write-Info "winget is available"
}

# Install required dependencies
Write-Step "Installing required dependencies..."

# Git
if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    Write-Info "Installing git..."
    try {
        winget install --id Git.Git --exact --silent --source winget --accept-package-agreements --accept-source-agreements
        # Refresh environment variables
        $env:Path = [System.Environment]::GetEnvironmentVariable("Path","Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path","User")
        Write-Info "git installed successfully"
    } catch {
        Write-Error "Failed to install git: $_"
        Write-Info "You may need to install git manually and restart the terminal"
        exit 1
    }
} else {
    Write-Info "git already installed: $(git --version)"
}

# jq
if (-not (Get-Command jq -ErrorAction SilentlyContinue)) {
    Write-Info "Installing jq..."
    try {
        winget install --id jqlang.jq --exact --silent --source winget --accept-package-agreements --accept-source-agreements
        # Refresh environment variables
        $env:Path = [System.Environment]::GetEnvironmentVariable("Path","Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path","User")
        Write-Info "jq installed successfully"
    } catch {
        Write-Error "Failed to install jq: $_"
        Write-Info "You may need to install jq manually and restart the terminal"
        exit 1
    }
} else {
    Write-Info "jq already installed"
}

# yq
if (-not (Get-Command yq -ErrorAction SilentlyContinue)) {
    Write-Info "Installing yq..."
    try {
        winget install --id MikeFarah.yq --exact --silent --source winget --accept-package-agreements --accept-source-agreements
        # Refresh environment variables
        $env:Path = [System.Environment]::GetEnvironmentVariable("Path","Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path","User")
        Write-Info "yq installed successfully"
    } catch {
        Write-Error "Failed to install yq: $_"
        Write-Info "You may need to install yq manually and restart the terminal"
        exit 1
    }
} else {
    Write-Info "yq already installed"
}

# Verify all dependencies
Write-Step "Verifying dependencies..."
$missingDeps = @()

$requiredCommands = @("git", "jq", "yq", "winget")
foreach ($cmd in $requiredCommands) {
    if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) {
        $missingDeps += $cmd
    }
}

if ($missingDeps.Count -gt 0) {
    Write-Error "Missing dependencies: $($missingDeps -join ', ')"
    Write-Info "You may need to restart your terminal to refresh PATH"
    exit 1
}

Write-Info "All dependencies verified successfully"

# Export environment variables to a file for future use
Write-Step "Setting up environment..."

$envBootstrap = @"
# Bootstrap environment variables
`$env:LOADOUT_ROOT = "$DotfilesRoot"
`$env:LOADOUT_PLATFORM = "windows"
"@

$envBootstrapPath = Join-Path $DotfilesRoot ".env.bootstrap.ps1"
$envBootstrap | Set-Content -Path $envBootstrapPath -Encoding UTF8

Write-Info "Bootstrap environment saved to .env.bootstrap.ps1"

Write-Host ""
Write-Step "Bootstrap complete!"
Write-Host ""
Write-Host "Next steps:"
Write-Host "  1. Review a profile: type profiles\windows.yaml"
Write-Host "  2. Apply a profile: .\loadout.ps1 apply profiles\windows.yaml"
Write-Host ""
Write-Host "Notes:"
Write-Host "  - If git/jq/yq commands not found, restart your terminal to refresh PATH"
Write-Host "  - Run as Administrator for symlink support (or enable Developer Mode)"
Write-Host "  - Scoop will be installed when you apply a profile that includes it"
Write-Host ""
