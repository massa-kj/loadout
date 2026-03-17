# -----------------------------------------------------------------------------
# Unit tests: feature_index (PowerShell)
#
# Tests that Invoke-FeatureIndexBuild scans sources and produces valid Feature
# Index JSON, and that Invoke-FeatureIndexFilter separates valid from blocked.
#
# Run directly: pwsh tests/unit/test_feature_index.ps1
# Exit code 0 = all pass, 1 = one or more failures.
# -----------------------------------------------------------------------------

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$REPO_ROOT = (Get-Item "$PSScriptRoot/../..").FullName

. "$PSScriptRoot\helpers.ps1"

# ── Temp environment ──────────────────────────────────────────────────────────

$TmpRoot = [System.IO.Path]::GetTempPath() + [System.IO.Path]::GetRandomFileName()
New-Item -ItemType Directory -Path $TmpRoot -Force | Out-Null

try {

$env:LOADOUT_ROOT                  = "$TmpRoot\repo"
$global:LOADOUT_ROOT               = $env:LOADOUT_ROOT
$env:LOADOUT_PLATFORM              = "linux"
$global:LOADOUT_PLATFORM           = "linux"
$env:LOADOUT_CONFIG_HOME           = "$TmpRoot\config\loadout"
$global:LOADOUT_CONFIG_HOME        = $env:LOADOUT_CONFIG_HOME
$env:LOADOUT_DATA_HOME             = "$TmpRoot\data\loadout"
$global:LOADOUT_DATA_HOME          = $env:LOADOUT_DATA_HOME
$env:LOADOUT_SOURCES_FILE          = "$env:LOADOUT_CONFIG_HOME\sources.yaml"
$global:LOADOUT_SOURCES_FILE       = $env:LOADOUT_SOURCES_FILE
$global:SUPPORTED_FEATURE_SPEC_VERSION = "1"

$FeatureRoot = "$env:LOADOUT_ROOT\features"
$BackendRoot = "$env:LOADOUT_ROOT\backends"

foreach ($d in @(
    $FeatureRoot, $BackendRoot,
    "$env:LOADOUT_CONFIG_HOME\features", "$env:LOADOUT_CONFIG_HOME\backends",
    "$env:LOADOUT_DATA_HOME\sources"
)) { New-Item -ItemType Directory -Path $d -Force | Out-Null }

@"
sources: []
"@ | Set-Content -Path $env:LOADOUT_SOURCES_FILE -Encoding UTF8

. "$REPO_ROOT\core\lib\source_registry.ps1"
. "$REPO_ROOT\core\lib\feature_index.ps1"

# ── Fixtures ──────────────────────────────────────────────────────────────────

# alpha — no deps
New-Item -ItemType Directory -Path "$FeatureRoot\alpha" -Force | Out-Null
@"
spec_version: 1
mode: script
description: alpha feature
depends: []
"@ | Set-Content "$FeatureRoot\alpha\feature.yaml" -Encoding UTF8

# beta — depends on bare name "alpha" (should normalize to core/alpha)
New-Item -ItemType Directory -Path "$FeatureRoot\beta" -Force | Out-Null
@"
spec_version: 1
mode: script
description: beta feature
depends:
  - alpha
"@ | Set-Content "$FeatureRoot\beta\feature.yaml" -Encoding UTF8

# future — unsupported spec_version (should be blocked)
New-Item -ItemType Directory -Path "$FeatureRoot\future" -Force | Out-Null
@"
spec_version: 999
mode: script
description: future feature with unknown spec
depends: []
"@ | Set-Content "$FeatureRoot\future\feature.yaml" -Encoding UTF8

# nodeps — minimal valid feature, no provides/requires
New-Item -ItemType Directory -Path "$FeatureRoot\nodeps" -Force | Out-Null
@"
spec_version: 1
mode: script
description: minimal feature
"@ | Set-Content "$FeatureRoot\nodeps\feature.yaml" -Encoding UTF8

# withcap — provides and requires capabilities
New-Item -ItemType Directory -Path "$FeatureRoot\withcap" -Force | Out-Null
@"
spec_version: 1
mode: script
description: feature with capabilities
depends: []
provides:
  - name: my_cap
requires:
  - name: other_cap
"@ | Set-Content "$FeatureRoot\withcap\feature.yaml" -Encoding UTF8

# nofile — a directory without feature.yaml (should be skipped)
New-Item -ItemType Directory -Path "$FeatureRoot\nofile" -Force | Out-Null

# ── Tests: Invoke-FeatureIndexBuild ───────────────────────────────────────────

Write-Host "Invoke-FeatureIndexBuild: schema_version field"

$fi = Invoke-FeatureIndexBuild 2>$null
$sv = ($fi | ConvertFrom-Json).schema_version
Assert-Equal "schema_version is 1" "1" "$sv"

# ── Test: well-formed entry for core/alpha ────────────────────────────────────

Write-Host "Invoke-FeatureIndexBuild: core/alpha entry"

$idx = $fi | ConvertFrom-Json
$alphaEntry = (_Prop $idx.features "core/alpha")
Assert-NotNull "core/alpha entry is present" $alphaEntry
Assert-Equal "core/alpha mode is script" "script" "$($alphaEntry.mode)"
Assert-Equal "core/alpha is not blocked" "False" "$($alphaEntry.blocked)"
Assert-Equal "core/alpha description" "alpha feature" "$($alphaEntry.description)"

# ── Test: bare dep "alpha" normalized to "core/alpha" ─────────────────────────

Write-Host "Invoke-FeatureIndexBuild: bare dep normalization"

$betaEntry = (_Prop $idx.features "core/beta")
$dep0 = if ($betaEntry.dep.depends.Count -gt 0) { $betaEntry.dep.depends[0] } else { "null" }
Assert-Equal "beta dep 'alpha' normalized to core/alpha" "core/alpha" "$dep0"

# ── Test: unsupported spec_version is blocked ─────────────────────────────────

Write-Host "Invoke-FeatureIndexBuild: unsupported spec_version blocked"

$futureEntry = (_Prop $idx.features "core/future")
Assert-NotNull "core/future entry present" $futureEntry
Assert-True "core/future is blocked" ([bool]$futureEntry.blocked)

$reason = "$($futureEntry.blocked_reason)"
if ($reason -like "*unsupported spec_version*") {
    Write-Host "  PASS  blocked_reason mentions unsupported spec_version"
    $script:PassCount++
} else {
    Write-Host "  FAIL  blocked_reason should mention 'unsupported spec_version'; got: '$reason'"
    $script:FailCount++
}

# ── Test: directory without feature.yaml is skipped ──────────────────────────

Write-Host "Invoke-FeatureIndexBuild: skips dirs without feature.yaml"

$nofileProp = $idx.features.PSObject.Properties["core/nofile"]
Assert-Null "core/nofile absent from index" $nofileProp

# ── Test: source_dir field is set correctly ───────────────────────────────────

Write-Host "Invoke-FeatureIndexBuild: source_dir field"

$alphaDir = "$($alphaEntry.source_dir)"
Assert-Equal "core/alpha source_dir" "$FeatureRoot\alpha" $alphaDir

# ── Test: provides/requires arrays are populated ──────────────────────────────

Write-Host "Invoke-FeatureIndexBuild: provides and requires"

$capEntry = (_Prop $idx.features "core/withcap")
Assert-NotNull "core/withcap entry present" $capEntry
$providesName = $capEntry.dep.provides[0].name
Assert-Equal "withcap provides my_cap" "my_cap" "$providesName"
$requiresName = $capEntry.dep.requires[0].name
Assert-Equal "withcap requires other_cap" "other_cap" "$requiresName"

# ── Tests: Invoke-FeatureIndexFilter ─────────────────────────────────────────

Write-Host "Invoke-FeatureIndexFilter: separates valid from blocked"

$filterResult = Invoke-FeatureIndexFilter `
    -FeatureIndexJson $fi `
    -DesiredFeatures @("core/alpha", "core/future") `
    2>$null

Assert-NotNull "filter result not null" $filterResult
Assert-Contains "valid array contains core/alpha" "core/alpha" ($filterResult.Valid -join " ")

if ($filterResult.Valid -contains "core/future") {
    Write-Host "  FAIL  core/future should not be in valid list"
    $script:FailCount++
} else {
    Write-Host "  PASS  core/future absent from valid list"
    $script:PassCount++
}

$blockedObj = $filterResult.BlockedJson | ConvertFrom-Json
$blockedFeat = if ($blockedObj.Count -gt 0) { $blockedObj[0].feature } else { "null" }
Assert-Equal "blocked array contains core/future" "core/future" "$blockedFeat"

# ── Test: Invoke-FeatureIndexFilter errors on unknown feature ─────────────────

Write-Host "Invoke-FeatureIndexFilter: error on unknown feature"

$errResult = $null
try {
    $errResult = Invoke-FeatureIndexFilter `
        -FeatureIndexJson $fi `
        -DesiredFeatures @("core/doesnotexist") `
        2>$null
} catch { $errResult = $null }

if ($null -eq $errResult) {
    Write-Host "  PASS  unknown feature in filter causes error"
    $script:PassCount++
} else {
    Write-Host "  FAIL  expected error for unknown feature"
    $script:FailCount++
}

# ── Test: Invoke-FeatureIndexBuild with real repo features ────────────────────

Write-Host "Invoke-FeatureIndexBuild: real repo smoke test"

# Copy real features into temp repo
if (Test-Path "$REPO_ROOT\features") {
    Copy-Item -Recurse -Force "$REPO_ROOT\features\*" "$FeatureRoot\" 2>$null
}

$fiReal = Invoke-FeatureIndexBuild 2>$null
$idxReal = $fiReal | ConvertFrom-Json

$gitEntry  = (_Prop $idxReal.features "core/git")
$bashEntry = (_Prop $idxReal.features "core/bash")

Assert-NotNull "real core/git is in index" $gitEntry
Assert-NotNull "real core/bash is in index" $bashEntry
Assert-Equal "real core/git mode" "script" "$($gitEntry.mode)"
Assert-Equal "real core/bash mode" "script" "$($bashEntry.mode)"
Assert-False "real core/git is not blocked" ([bool]$gitEntry.blocked)
Assert-False "real core/bash is not blocked" ([bool]$bashEntry.blocked)

} finally {
    Remove-Item -Recurse -Force $TmpRoot -ErrorAction SilentlyContinue
}

# ── Summary ───────────────────────────────────────────────────────────────────

Show-TestSummary
