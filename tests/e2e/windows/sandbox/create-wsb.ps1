#Requires -Version 5.1

<#
.SYNOPSIS
    Generate a .wsb (Windows Sandbox) configuration file for testing.

.DESCRIPTION
    Creates a Windows Sandbox configuration from a template with
    repository-specific paths substituted.

    The sandbox uses pre-built host binaries (loadout.exe, loadout-e2e.exe)
    and dummy backends — no WinGet, no network access required.

.PARAMETER Scenario
    Test scenario to run (minimal, idempotent, lifecycle, uninstall, etc.)
    If not specified, creates a manual testing environment.

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
    [ValidateSet("minimal", "idempotent", "lifecycle", "uninstall",
                 "version-install", "version-mixed", "version-upgrade")]
    [string]$Scenario
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Resolve repository root (this script is at tests/e2e/windows/sandbox/create-wsb.ps1)
$ScriptDir = $PSScriptRoot
$RepoRoot  = (Resolve-Path (Join-Path $ScriptDir "..\..\..\..\")).Path
$LogsRoot  = Join-Path $RepoRoot "tests\e2e\windows\logs"

# Ensure logs directory exists
if (-not (Test-Path $LogsRoot)) {
    New-Item -ItemType Directory -Force -Path $LogsRoot | Out-Null
}

# Read template
$TemplatePath = Join-Path $ScriptDir "loadout.wsb.template"
$OutputPath   = Join-Path $ScriptDir "loadout.wsb"

if (-not (Test-Path $TemplatePath)) {
    throw "Template not found: $TemplatePath"
}

$Content = Get-Content $TemplatePath -Raw

# Substitute path placeholders
$Content = $Content.Replace("__LOADOUT_ROOT__", $RepoRoot)
$Content = $Content.Replace("__LOGS_ROOT__",    $LogsRoot)

# Copy-repo command: called first in all modes.
# The sandbox mounts the repo read-only at C:\host-loadout; copy it to a
# writable location before running any scripts.
$CopyRepoCmd = @"
Write-Host 'Copying repository...' -ForegroundColor Cyan;
`$w = 'C:\loadout';
if (Test-Path `$w) { Remove-Item `$w -Recurse -Force };
Copy-Item 'C:\host-loadout' `$w -Recurse;
cd `$w;
Write-Host 'Working directory: `$w' -ForegroundColor Gray;
"@

# Generate the LogonCommand payload depending on mode.
if ($Scenario) {
    # Automated test mode: set SCENARIO and run the test script.
    $InnerCommand = $CopyRepoCmd + "`$env:SCENARIO='$Scenario'; .\tests\e2e\windows\sandbox\run-in-sandbox.ps1"
} else {
    # Manual mode: copy repo, set up config directory, and show instructions.
    $SetupConfigCmd = @"
Write-Host 'Setting up loadout config...' -ForegroundColor Cyan;
`$LoadoutRoot = Join-Path `$env:APPDATA 'loadout';
`$FeaturesDir = Join-Path `$LoadoutRoot 'features';
`$BackendsDir = Join-Path `$LoadoutRoot 'backends';
`$ConfigsDir  = Join-Path `$LoadoutRoot 'configs';
New-Item -ItemType Directory -Force -Path `$FeaturesDir, `$BackendsDir, `$ConfigsDir | Out-Null;
Copy-Item 'features\*'                   `$FeaturesDir -Recurse -Force;
Copy-Item 'backends\*'                   `$BackendsDir -Recurse -Force;
Copy-Item 'tests\fixtures\backends\*'   `$BackendsDir -Recurse -Force;
Copy-Item 'tests\fixtures\features\*'   `$FeaturesDir -Recurse -Force;
Copy-Item 'tests\fixtures\configs\*'    `$ConfigsDir  -Force;
`$env:XDG_CONFIG_HOME = `$env:APPDATA;
`$env:XDG_STATE_HOME  = `$env:APPDATA;
Write-Host 'Config root: ' -NoNewline -ForegroundColor Gray; Write-Host `$LoadoutRoot;
"@
    $InnerCommand = $CopyRepoCmd + $SetupConfigCmd + @"
Write-Host 'Ready for manual testing!' -ForegroundColor Green;
Write-Host 'Binaries are at: target\release\' -ForegroundColor Yellow;
Write-Host 'Config names (e.g. -c config-base) resolve from: ' -NoNewline -ForegroundColor Yellow; Write-Host (Join-Path `$env:APPDATA 'loadout\configs');
Write-Host 'To run a scenario: `$env:SCENARIO=''minimal''; .\tests\e2e\windows\sandbox\run-in-sandbox.ps1' -ForegroundColor Yellow;
"@
}

# Escape single quotes for embedding into the PowerShell -Command argument.
$InnerCommand = $InnerCommand -replace "'", "''"

# Wrap in Start-Process so the script runs in a visible, interactive window.
$LogonCommand = "powershell -ExecutionPolicy Bypass -Command `"Start-Process powershell -ArgumentList '-ExecutionPolicy', 'Bypass', '-NoExit', '-NoLogo', '-Command', '$InnerCommand'`""
$Content = $Content.Replace("__LOGON_COMMAND__", $LogonCommand)

Set-Content -Path $OutputPath -Value $Content -Encoding UTF8 -NoNewline

Write-Host "Generated: $OutputPath" -ForegroundColor Green
if ($Scenario) {
    Write-Host "Mode    : Automated test" -ForegroundColor Cyan
    Write-Host "Scenario: $Scenario"      -ForegroundColor Cyan
} else {
    Write-Host "Mode: Manual testing" -ForegroundColor Cyan
    Write-Host "No scenario will run automatically" -ForegroundColor Yellow
}
Write-Host ""
Write-Host "To launch Sandbox:"
Write-Host "  .\loadout.wsb" -ForegroundColor Yellow
