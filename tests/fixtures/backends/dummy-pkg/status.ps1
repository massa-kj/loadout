#Requires -Version 5.1
# Dummy package backend — status.
# Outputs "installed" or "not_installed" based on marker file presence.

$ErrorActionPreference = 'Stop'

if ($env:LOADOUT_RESOURCE_KIND -ne 'Package') {
    Write-Error "dummy-pkg backend only supports Package resources"
    exit 1
}

$Marker = Join-Path $env:TEMP "loadout-dummy\packages\$env:LOADOUT_PACKAGE_NAME"

if (Test-Path $Marker) {
    Write-Output "installed"
} else {
    Write-Output "not_installed"
}
