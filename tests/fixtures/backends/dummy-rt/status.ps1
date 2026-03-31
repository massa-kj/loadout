#Requires -Version 5.1
# Dummy runtime backend — status.
# Outputs "installed" or "not_installed" based on marker file presence.

$ErrorActionPreference = 'Stop'

if ($env:LOADOUT_RESOURCE_KIND -ne 'Runtime') {
    Write-Error "dummy-rt backend only supports Runtime resources"
    exit 1
}

$Marker = Join-Path $env:TEMP "loadout-dummy\runtimes\$env:LOADOUT_RUNTIME_NAME\$env:LOADOUT_RUNTIME_VERSION"

if (Test-Path $Marker) {
    Write-Output "installed"
} else {
    Write-Output "not_installed"
}
