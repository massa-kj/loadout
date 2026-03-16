# Main CLI entry point

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ScriptRoot = $PSScriptRoot
$global:LOADOUT_ROOT = $ScriptRoot

# Load logger for error messages
. "$ScriptRoot\core\lib\logger.ps1"

# Usage
function Show-Usage {
    Write-Host @"
Usage: loadout.ps1 <command> [options]

A declarative environment management system.

Available commands:
  apply <profile>    Apply a loadout profile
  plan  <profile>    Show what apply would do (no changes made)
  migrate            Migrate state/profile keys to current schema

Options:
  plan -Verbose      Also list noop (already up-to-date) features

Examples:
  .\loadout.ps1 apply profiles\windows.yaml
  .\loadout.ps1 plan  profiles\windows.yaml
  .\loadout.ps1 plan  profiles\windows.yaml -Verbose
  .\loadout.ps1 migrate -DryRun

"@
    exit 1
}

# Parse command
if ($args.Count -lt 1) {
    Show-Usage
}

$Command = $args[0]
$CommandArgs = $args[1..($args.Count - 1)]

# Dispatch to command implementation
switch ($Command) {
    "apply" {
        & "$ScriptRoot\cmd\apply.ps1" @CommandArgs
        exit $LASTEXITCODE
    }
    "plan" {
        & "$ScriptRoot\cmd\plan.ps1" @CommandArgs
        exit $LASTEXITCODE
    }
    "migrate" {
        & "$ScriptRoot\cmd\migrate.ps1" @CommandArgs
        exit $LASTEXITCODE
    }
    "help" {
        Show-Usage
    }
    "--help" {
        Show-Usage
    }
    "-h" {
        Show-Usage
    }
    default {
        Log-Error "Unknown command: $Command"
        Write-Host ""
        Show-Usage
    }
}
