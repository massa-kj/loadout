#Requires -Version 5.1
# Dummy package backend — apply (install).
# No network access; records installation by creating a marker file under
# $env:TEMP\loadout-dummy\packages\.

$ErrorActionPreference = 'Stop'

if ($env:LOADOUT_RESOURCE_KIND -ne 'Package') {
    Write-Error "dummy-pkg backend only supports Package resources"
    exit 1
}

$MarkerDir = Join-Path $env:TEMP "loadout-dummy\packages"
$Marker    = Join-Path $MarkerDir $env:LOADOUT_PACKAGE_NAME

if (-not (Test-Path $MarkerDir)) {
    New-Item -ItemType Directory -Force -Path $MarkerDir | Out-Null
}

if (Test-Path $Marker) {
    Write-Host "dummy-pkg: '$env:LOADOUT_PACKAGE_NAME' already installed (marker present)"
    exit 0
}

New-Item -ItemType File -Force -Path $Marker | Out-Null
Write-Host "dummy-pkg: installed '$env:LOADOUT_PACKAGE_NAME'"
