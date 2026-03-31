#Requires -Version 5.1
# Dummy runtime backend — remove.
# Removes the marker file if it exists.

$ErrorActionPreference = 'Stop'

if ($env:LOADOUT_RESOURCE_KIND -ne 'Runtime') {
    Write-Error "dummy-rt backend only supports Runtime resources"
    exit 1
}

$Marker = Join-Path $env:TEMP "loadout-dummy\runtimes\$env:LOADOUT_RUNTIME_NAME\$env:LOADOUT_RUNTIME_VERSION"

if (Test-Path $Marker) {
    Remove-Item $Marker -Force
    Write-Host "dummy-rt: removed '$env:LOADOUT_RUNTIME_NAME@$env:LOADOUT_RUNTIME_VERSION'"
} else {
    Write-Host "dummy-rt: '$env:LOADOUT_RUNTIME_NAME@$env:LOADOUT_RUNTIME_VERSION' not installed"
}
