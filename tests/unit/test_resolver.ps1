# -----------------------------------------------------------------------------
# Unit tests: resolver (PowerShell, Phase 2)
#
# Tests that Resolve-Dependencies outputs canonical IDs, normalizes bare deps,
# and correctly orders features topologically.
#
# Run directly: pwsh tests/unit/test_resolver.ps1
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

$env:LOADOUT_ROOT                     = "$TmpRoot\repo"
$global:LOADOUT_ROOT                  = $env:LOADOUT_ROOT
$env:LOADOUT_PLATFORM                 = "linux"
$global:LOADOUT_PLATFORM              = "linux"
$env:LOADOUT_CONFIG_HOME              = "$TmpRoot\config\loadout"
$global:LOADOUT_CONFIG_HOME           = $env:LOADOUT_CONFIG_HOME
$env:LOADOUT_DATA_HOME                = "$TmpRoot\data\loadout"
$global:LOADOUT_DATA_HOME             = $env:LOADOUT_DATA_HOME
$env:LOADOUT_SOURCES_FILE             = "$env:LOADOUT_CONFIG_HOME\sources.yaml"
$global:LOADOUT_SOURCES_FILE          = $env:LOADOUT_SOURCES_FILE
$global:SUPPORTED_FEATURE_SPEC_VERSION = "1"

$FeatureRoot = "$env:LOADOUT_ROOT\features"
foreach ($d in @(
    $FeatureRoot, "$env:LOADOUT_ROOT\backends",
    "$env:LOADOUT_CONFIG_HOME\features", "$env:LOADOUT_CONFIG_HOME\backends",
    "$env:LOADOUT_DATA_HOME\sources"
)) { New-Item -ItemType Directory -Path $d -Force | Out-Null }

@"
sources: []
"@ | Set-Content $env:LOADOUT_SOURCES_FILE -Encoding UTF8

. "$REPO_ROOT\core\lib\source_registry.ps1"
. "$REPO_ROOT\core\lib\feature_index.ps1"
. "$REPO_ROOT\core\lib\resolver.ps1"

# ── Fixtures ──────────────────────────────────────────────────────────────────

# alpha — no deps
New-Item -ItemType Directory "$FeatureRoot\alpha" -Force | Out-Null
@"
spec_version: 1
mode: script
description: alpha feature
depends: []
"@ | Set-Content "$FeatureRoot\alpha\feature.yaml" -Encoding UTF8

# beta — depends on bare name "alpha" (→ core/alpha)
New-Item -ItemType Directory "$FeatureRoot\beta" -Force | Out-Null
@"
spec_version: 1
mode: script
description: beta feature
depends:
  - alpha
"@ | Set-Content "$FeatureRoot\beta\feature.yaml" -Encoding UTF8

# gamma — depends on bare name "beta" (→ core/beta)
New-Item -ItemType Directory "$FeatureRoot\gamma" -Force | Out-Null
@"
spec_version: 1
mode: script
description: gamma feature
depends:
  - beta
"@ | Set-Content "$FeatureRoot\gamma\feature.yaml" -Encoding UTF8

# provider — provides a capability
New-Item -ItemType Directory "$FeatureRoot\provider" -Force | Out-Null
@"
spec_version: 1
mode: script
description: provides a capability
depends: []
provides:
  - name: my_capability
"@ | Set-Content "$FeatureRoot\provider\feature.yaml" -Encoding UTF8

# consumer — requires the capability
New-Item -ItemType Directory "$FeatureRoot\consumer" -Force | Out-Null
@"
spec_version: 1
mode: script
description: requires a capability
depends: []
requires:
  - name: my_capability
"@ | Set-Content "$FeatureRoot\consumer\feature.yaml" -Encoding UTF8

# orphan — depends on a non-existent feature
New-Item -ItemType Directory "$FeatureRoot\orphan" -Force | Out-Null
@"
spec_version: 1
mode: script
description: depends on non-existent feature
depends:
  - nonexistent
"@ | Set-Content "$FeatureRoot\orphan\feature.yaml" -Encoding UTF8

# ── Helper: build index once for tests that share fixtures ────────────────────

$Idx1 = Invoke-FeatureIndexBuild 2>$null
Assert-NotNull "feature index built successfully" $Idx1

# ── Test: Resolve-Dependencies outputs canonical IDs ─────────────────────────

Write-Host "Resolve-Dependencies: canonical ID output"

$features1 = @("core/alpha", "core/beta", "core/gamma")
Read-FeatureMetadata -FeatureIndexJson $Idx1 -Features $features1 2>$null | Out-Null
$sorted1 = Resolve-Dependencies -DesiredFeatures $features1 2>$null

Assert-NotNull "sorted output not null" $sorted1
Assert-Contains "output contains core/alpha" "core/alpha" ($sorted1 -join " ")
Assert-Contains "output contains core/beta" "core/beta" ($sorted1 -join " ")
Assert-Contains "output contains core/gamma" "core/gamma" ($sorted1 -join " ")

# ── Test: dependency ordering is correct ─────────────────────────────────────

Write-Host "Resolve-Dependencies: ordering"

$idxAlpha = [Array]::IndexOf($sorted1, "core/alpha")
$idxBeta  = [Array]::IndexOf($sorted1, "core/beta")
$idxGamma = [Array]::IndexOf($sorted1, "core/gamma")

Assert-True "core/alpha before core/beta" ($idxAlpha -lt $idxBeta)
Assert-True "core/beta before core/gamma" ($idxBeta  -lt $idxGamma)

# ── Test: Read-FeatureMetadata normalizes bare deps ───────────────────────────

Write-Host "Read-FeatureMetadata: bare dep normalization"

$single = @("core/beta")
Read-FeatureMetadata -FeatureIndexJson $Idx1 -Features $single 2>$null | Out-Null

$betaDeps = $script:FeatureDeps["core/beta"]
$betaDep0 = if ($betaDeps -and $betaDeps.Count -gt 0) { $betaDeps[0] } else { "null" }
Assert-Equal "beta's dep normalized to core/alpha" "core/alpha" "$betaDep0"

# ── Test: capability-based deps use canonical IDs ────────────────────────────

Write-Host "Read-FeatureMetadata: canonical IDs in provides/requires"

$capFeatures = @("core/provider", "core/consumer")
Read-FeatureMetadata -FeatureIndexJson $Idx1 -Features $capFeatures 2>$null | Out-Null

$providerList = $script:Provides["my_capability"]
$providerVal  = if ($providerList -and $providerList.Count -gt 0) { $providerList[0] } else { "null" }
Assert-Equal "provider stored as canonical ID in Provides" "core/provider" "$providerVal"

$capSorted = Resolve-Dependencies -DesiredFeatures $capFeatures 2>$null
Assert-NotNull "capability resolved successfully" $capSorted

$idxProvider = [Array]::IndexOf($capSorted, "core/provider")
$idxConsumer = [Array]::IndexOf($capSorted, "core/consumer")
Assert-True "core/provider before core/consumer (capability dep)" ($idxProvider -lt $idxConsumer)

# ── Test: single feature (no deps) outputs canonical ID ──────────────────────

Write-Host "Resolve-Dependencies: single feature"

$onlyAlpha = @("core/alpha")
Read-FeatureMetadata -FeatureIndexJson $Idx1 -Features $onlyAlpha 2>$null | Out-Null
$singleSorted = Resolve-Dependencies -DesiredFeatures $onlyAlpha 2>$null

Assert-NotNull "single feature sorted not null" $singleSorted
Assert-Equal "single feature output is canonical" "core/alpha" "$($singleSorted[0])"

# ── Test: missing dependency causes error ($null) ─────────────────────────────

Write-Host "Resolve-Dependencies: missing dep -> null"

$orphanFeatures = @("core/orphan")
Read-FeatureMetadata -FeatureIndexJson $Idx1 -Features $orphanFeatures 2>$null | Out-Null

# core/nonexistent is not present in the desired set → resolve should return $null
$orphanResult = $null
try {
    $orphanResult = Resolve-Dependencies -DesiredFeatures $orphanFeatures 2>$null
} catch { $orphanResult = $null }

Assert-Null "missing dep causes Resolve-Dependencies to return null" $orphanResult

# ── Test: disallowed external dependency (not in desired set) causes error ────

Write-Host "Resolve-Dependencies: undeclared dep not in desired set → null"

New-Item -ItemType Directory "$FeatureRoot\extparent" -Force | Out-Null
@"
spec_version: 1
mode: script
description: depends on a feature not in the desired set
depends:
  - core/git
"@ | Set-Content "$FeatureRoot\extparent\feature.yaml" -Encoding UTF8

$idx2 = Invoke-FeatureIndexBuild 2>$null
$extParent = @("core/extparent")
Read-FeatureMetadata -FeatureIndexJson $idx2 -Features $extParent 2>$null | Out-Null

$extResult = $null
try {
    $extResult = Resolve-Dependencies -DesiredFeatures $extParent 2>$null
} catch { $extResult = $null }

Assert-Null "dep outside desired set causes null" $extResult

# ── Test: real repo features produce canonical IDs ────────────────────────────

Write-Host "Resolve-Dependencies: real repo smoke test"

if (Test-Path "$REPO_ROOT\features") {
    Copy-Item -Recurse -Force "$REPO_ROOT\features\*" "$FeatureRoot\" 2>$null
}

$idxReal    = Invoke-FeatureIndexBuild 2>$null
$realFeats  = @("core/git", "core/bash")
Read-FeatureMetadata -FeatureIndexJson $idxReal -Features $realFeats 2>$null | Out-Null
$realSorted = Resolve-Dependencies -DesiredFeatures $realFeats 2>$null

Assert-NotNull "real sorted output not null" $realSorted
Assert-Contains "real output contains core/git"  "core/git"  ($realSorted -join " ")
Assert-Contains "real output contains core/bash" "core/bash" ($realSorted -join " ")

# Verify no bare names leaked into output
$bareLeak = $false
foreach ($id in $realSorted) {
    if ($id -notlike "*/*") {
        Write-Host "  FAIL  bare name leaked into output: '$id'"
        $script:FailCount++
        $bareLeak = $true
    }
}
if (-not $bareLeak) {
    Write-Host "  PASS  no bare names in output ($($realSorted.Count) features checked)"
    $script:PassCount++
}

} finally {
    Remove-Item -Recurse -Force $TmpRoot -ErrorAction SilentlyContinue
}

# ── Summary ───────────────────────────────────────────────────────────────────

Show-TestSummary
