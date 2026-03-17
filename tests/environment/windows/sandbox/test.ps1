#Requires -Version 5.1

<#
.SYNOPSIS
    Run Windows Sandbox-based integration tests.

.DESCRIPTION
    Launches Windows Sandbox for each test scenario to verify loadout behavior
    in a clean, isolated environment.

.PARAMETER Command
    Test command to execute:
    - all: Run all test scenarios (default)
    - minimal: Basic execution test
    - idempotent: Idempotency test
    - uninstall: Uninstall safety test
    - version-install: Version specification installation
    - version-mixed: Mixed version/no-version features
    - version-upgrade: Version change reinstall
    - clean: Remove generated .wsb files

.EXAMPLE
    .\test.ps1 all
    Run all test scenarios

.EXAMPLE
    .\test.ps1 minimal
    Run minimal scenario only

.EXAMPLE
    .\test.ps1 clean
    Clean up generated files
#>

[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [ValidateSet("all", "minimal", "idempotent", "uninstall", "version-install", "version-mixed", "version-upgrade", "clean")]
    [string]$Command = "all"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Color output functions
function Write-Step {
    param([string]$Message)
    Write-Host "==>" -ForegroundColor Green -NoNewline
    Write-Host " $Message"
}

function Write-Info {
    param([string]$Message)
    Write-Host "[INFO]" -ForegroundColor Blue -NoNewline
    Write-Host " $Message"
}

function Write-Warn {
    param([string]$Message)
    Write-Host "[WARN]" -ForegroundColor Yellow -NoNewline
    Write-Host " $Message"
}

$ScriptDir = $PSScriptRoot

# Check Windows Sandbox availability
function Test-SandboxAvailable {
    try {
        $feature = Get-WindowsOptionalFeature -Online -FeatureName "Containers-DisposableClientVM" -ErrorAction Stop
        if ($feature.State -ne "Enabled") {
            Write-Host ""
            Write-Warn "Windows Sandbox is not enabled"
            Write-Info "Enable it with:"
            Write-Host "  Enable-WindowsOptionalFeature -Online -FeatureName 'Containers-DisposableClientVM' -All" -ForegroundColor Yellow
            Write-Host ""
            return $false
        }
        return $true
    } catch {
        Write-Host ""
        Write-Warn "Windows Sandbox feature not found"
        Write-Info "Ensure you are running Windows 10 Pro/Enterprise (1903+) or Windows 11"
        Write-Host ""
        return $false
    }
}

# Run a single test scenario
function Invoke-TestScenario {
    param([string]$Scenario)
    
    Write-Step "Running $Scenario scenario..."
    
    # Generate .wsb configuration
    $CreateWsbScript = Join-Path $ScriptDir "create-wsb.ps1"
    & $CreateWsbScript -Scenario $Scenario
    
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to generate .wsb configuration"
    }
    
    # Launch Sandbox
    $WsbPath = Join-Path $ScriptDir "loadout.wsb"
    
    Write-Info "Launching Windows Sandbox..."
    Write-Info "Please wait for test to complete and close Sandbox manually when done"
    Write-Host ""
    
    Start-Process -FilePath "WindowsSandbox.exe" -ArgumentList $WsbPath -Wait
    
    Write-Step "$Scenario scenario completed"
    Write-Host ""
}

# Clean generated files
function Invoke-Clean {
    Write-Step "Cleaning up..."
    
    $WsbPath = Join-Path $ScriptDir "loadout.wsb"
    if (Test-Path $WsbPath) {
        Remove-Item $WsbPath -Force
        Write-Info "Removed: loadout.wsb"
    }
    
    Write-Step "Clean complete"
}

# Main execution
Write-Host ""
Write-Host "Windows Sandbox Integration Tests" -ForegroundColor Cyan
Write-Host "==================================" -ForegroundColor Cyan
Write-Host ""

# Check prerequisites
if ($Command -ne "clean") {
    if (-not (Test-SandboxAvailable)) {
        exit 1
    }
}

# Execute command
switch ($Command) {
    "all" {
        $scenarios = @("minimal", "idempotent", "uninstall", "version-install", "version-mixed", "version-upgrade")
        
        Write-Info "Running all scenarios: $($scenarios -join ', ')"
        Write-Host ""
        
        foreach ($scenario in $scenarios) {
            Invoke-TestScenario -Scenario $scenario
        }
        
        Write-Host ""
        Write-Step "All scenarios completed!"
        Write-Host ""
    }
    "clean" {
        Invoke-Clean
    }
    default {
        Invoke-TestScenario -Scenario $Command
    }
}
