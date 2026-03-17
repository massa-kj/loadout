#Requires -Version 5.1

<#
.SYNOPSIS
    Generate a .wsb (Windows Sandbox) configuration file for testing.

.DESCRIPTION
    Creates a Windows Sandbox configuration from a template with
    repository-specific paths substituted.

.PARAMETER Scenario
    Test scenario to run (minimal, idempotent, uninstall, etc.)
    If not specified, creates a manual testing environment without running a scenario.

.EXAMPLE
    .\create-wsb.ps1 -Scenario minimal
    .\loadout.wsb  # Launch Sandbox with minimal scenario

.EXAMPLE
    .\create-wsb.ps1
    .\loadout.wsb  # Launch Sandbox for manual testing
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory=$false)]
    [ValidateSet("minimal", "idempotent", "uninstall", "version-install", "version-mixed", "version-upgrade")]
    [string]$Scenario
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Resolve repository root (this script is at tests/environment/windows/sandbox/create-wsb.ps1)
$ScriptDir = $PSScriptRoot
$RepoRoot = (Resolve-Path (Join-Path $ScriptDir "..\..\..\..\")).Path
$LogsRoot = Join-Path $RepoRoot "tests\environment\windows\logs"

# Ensure logs directory exists
if (-not (Test-Path $LogsRoot)) {
    New-Item -ItemType Directory -Force -Path $LogsRoot | Out-Null
}

# Read template
$TemplatePath = Join-Path $ScriptDir "loadout.wsb.template"
$OutputPath = Join-Path $ScriptDir "loadout.wsb"

if (-not (Test-Path $TemplatePath)) {
    throw "Template not found: $TemplatePath"
}

$Content = Get-Content $TemplatePath -Raw

# Substitute placeholders
# Note: XML paths use backslashes, so we escape them for literal replacement
$Content = $Content.Replace("__LOADOUT_ROOT__", $RepoRoot)
$Content = $Content.Replace("__LOGS_ROOT__", $LogsRoot)

# Generate appropriate LogonCommand based on scenario
# WinGet installation command (common for both modes) - simplified without jobs
$WinGetInstallCmd = @"
Write-Host 'Installing WinGet...' -ForegroundColor Cyan;
cd `$env:USERPROFILE\Downloads;
Write-Host 'Downloading dependencies...' -ForegroundColor Gray;
curl.exe -LO https://www.nuget.org/api/v2/package/Microsoft.UI.Xaml/2.7.0;
curl.exe -LO https://aka.ms/Microsoft.VCLibs.x64.14.00.Desktop.appx;
curl.exe -LO https://github.com/microsoft/winget-cli/releases/download/v1.6.3482/Microsoft.DesktopAppInstaller_8wekyb3d8bbwe.msixbundle;
Write-Host 'Extracting and installing...' -ForegroundColor Gray;
Rename-Item .\2.7.0 -NewName Microsoft.UI.Xaml.2.7.0.zip;
Expand-Archive -LiteralPath .\Microsoft.UI.Xaml.2.7.0.zip;
Add-AppxPackage -Path .\Microsoft.UI.Xaml.2.7.0\tools\AppX\x64\Release\Microsoft.UI.Xaml.2.7.appx -ErrorAction SilentlyContinue;
Add-AppxPackage -Path .\Microsoft.VCLibs.x64.14.00.Desktop.appx;
Add-AppxPackage -Path .\Microsoft.DesktopAppInstaller_8wekyb3d8bbwe.msixbundle;
Write-Host 'WinGet installation complete!' -ForegroundColor Green;
"@

# Repository copy command (common for both modes)
$CopyRepoCmd = @"
Write-Host 'Copying repository...' -ForegroundColor Cyan;
`$WorkDir = 'C:\loadout';
if (Test-Path `$WorkDir) { Remove-Item `$WorkDir -Recurse -Force };
Copy-Item 'C:\host-loadout' `$WorkDir -Recurse;
cd `$WorkDir;
Write-Host 'Working directory: `$WorkDir' -ForegroundColor Gray;
"@

if ($Scenario) {
    # Automated test mode: Install WinGet, copy repo, set SCENARIO env var, then run script
    $InnerCommand = $WinGetInstallCmd + $CopyRepoCmd + "`$env:SCENARIO='$Scenario'; .\tests\environment\windows\sandbox\run-in-sandbox.ps1"
} else {
    # Manual mode: Install WinGet, copy repo, then ready for manual testing
    $InnerCommand = $WinGetInstallCmd + $CopyRepoCmd + "Write-Host 'Ready for manual testing!' -ForegroundColor Green; Write-Host 'Run bootstrap: .\platforms\windows\bootstrap.ps1' -ForegroundColor Yellow"
}

# Escape single quotes for PowerShell string
$InnerCommand = $InnerCommand -replace "'", "''"

# Use Start-Process to launch PowerShell in a new visible window
$LogonCommand = "powershell -ExecutionPolicy Bypass -Command `"Start-Process powershell -ArgumentList '-ExecutionPolicy', 'Bypass', '-NoExit', '-NoLogo', '-Command', '$InnerCommand'`""
$Content = $Content.Replace("__LOGON_COMMAND__", $LogonCommand)

# Write output
Set-Content -Path $OutputPath -Value $Content -Encoding UTF8 -NoNewline

Write-Host "Generated: $OutputPath" -ForegroundColor Green
if ($Scenario) {
    Write-Host "Mode: Automated test" -ForegroundColor Cyan
    Write-Host "Scenario: $Scenario" -ForegroundColor Cyan
} else {
    Write-Host "Mode: Manual testing" -ForegroundColor Cyan
    Write-Host "No scenario will run automatically" -ForegroundColor Yellow
}
Write-Host ""
Write-Host "To launch Sandbox:"
Write-Host "  .\loadout.wsb" -ForegroundColor Yellow
