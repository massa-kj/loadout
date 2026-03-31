#Requires -Version 5.1
# Dummy package backend — remove.
# Removes the marker file if it exists.

$ErrorActionPreference = 'Stop'

if ($env:LOADOUT_RESOURCE_KIND -ne 'Package') {
    Write-Error "dummy-pkg backend only supports Package resources"
    exit 1
}

$Marker = Join-Path $env:TEMP "loadout-dummy\packages\$env:LOADOUT_PACKAGE_NAME"

if (Test-Path $Marker) {
    Remove-Item $Marker -Force
    Write-Host "dummy-pkg: removed '$env:LOADOUT_PACKAGE_NAME'"
} else {
    Write-Host "dummy-pkg: '$env:LOADOUT_PACKAGE_NAME' not installed"
}
