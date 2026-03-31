#Requires -Version 5.1
# Dummy runtime backend — apply (install).
# No network access; records installation by creating a marker file under
# $env:TEMP\loadout-dummy\runtimes\<name>\<version>.

$ErrorActionPreference = 'Stop'

if ($env:LOADOUT_RESOURCE_KIND -ne 'Runtime') {
    Write-Error "dummy-rt backend only supports Runtime resources"
    exit 1
}

$MarkerDir = Join-Path $env:TEMP "loadout-dummy\runtimes\$env:LOADOUT_RUNTIME_NAME"
$Marker    = Join-Path $MarkerDir $env:LOADOUT_RUNTIME_VERSION

if (-not (Test-Path $MarkerDir)) {
    New-Item -ItemType Directory -Force -Path $MarkerDir | Out-Null
}

if (Test-Path $Marker) {
    Write-Host "dummy-rt: '$env:LOADOUT_RUNTIME_NAME@$env:LOADOUT_RUNTIME_VERSION' already installed (marker present)"
    exit 0
}

New-Item -ItemType File -Force -Path $Marker | Out-Null
Write-Host "dummy-rt: installed '$env:LOADOUT_RUNTIME_NAME@$env:LOADOUT_RUNTIME_VERSION'"
