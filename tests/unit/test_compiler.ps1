# -----------------------------------------------------------------------------
# Unit tests: compiler (FeatureCompiler, PowerShell)
#
# Tests that Invoke-FeatureCompilerRun produces correct raw DesiredResourceGraph
# JSON for both script and declarative features, and enforces declarative
# invariants. The compiler only assigns stable resource IDs; desired_backend
# is NOT present in the output (PolicyResolver adds that in a subsequent step).
#
# Run directly: pwsh tests/unit/test_compiler.ps1
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

$env:LOADOUT_ROOT           = "$TmpRoot\repo"
$global:LOADOUT_ROOT        = $env:LOADOUT_ROOT
$env:LOADOUT_PLATFORM       = "linux"
$global:LOADOUT_PLATFORM    = "linux"
$env:LOADOUT_CONFIG_HOME    = "$TmpRoot\config\loadout"
$global:LOADOUT_CONFIG_HOME = $env:LOADOUT_CONFIG_HOME
$env:LOADOUT_DATA_HOME      = "$TmpRoot\data\loadout"
$global:LOADOUT_DATA_HOME   = $env:LOADOUT_DATA_HOME
$env:LOADOUT_SOURCES_FILE   = "$env:LOADOUT_CONFIG_HOME\sources.yaml"
$global:LOADOUT_SOURCES_FILE = $env:LOADOUT_SOURCES_FILE

foreach ($d in @(
    "$env:LOADOUT_ROOT\features", "$env:LOADOUT_ROOT\backends",
    "$env:LOADOUT_CONFIG_HOME\features", "$env:LOADOUT_CONFIG_HOME\backends",
    "$env:LOADOUT_DATA_HOME\sources"
)) { New-Item -ItemType Directory -Path $d -Force | Out-Null }

@"
sources: []
"@ | Set-Content -Path $env:LOADOUT_SOURCES_FILE -Encoding UTF8

. "$REPO_ROOT\core\lib\source_registry.ps1"

. "$REPO_ROOT\core\lib\compiler.ps1"

# ── Fixture helpers ───────────────────────────────────────────────────────────

# _MakeScriptEntry <sourceDir> → ordered hashtable for Feature Index entry
function _MakeScriptEntry {
    param([string]$SourceDir)
    return [ordered]@{
        spec_version   = 1
        mode           = "script"
        description    = "test"
        source_dir     = $SourceDir
        blocked        = $false
        blocked_reason = $null
        dep            = [ordered]@{ depends = @(); provides = @(); requires = @() }
        spec           = $null
    }
}

# _MakeDeclarativeEntry <sourceDir> <resourcesArray> → ordered hashtable
function _MakeDeclarativeEntry {
    param([string]$SourceDir, [object[]]$Resources)
    return [ordered]@{
        spec_version   = 1
        mode           = "declarative"
        description    = "test"
        source_dir     = $SourceDir
        blocked        = $false
        blocked_reason = $null
        dep            = [ordered]@{ depends = @(); provides = @(); requires = @() }
        spec           = [ordered]@{ resources = $Resources }
    }
}

# _MakeIndex <featuresOrderedHash> → Feature Index JSON string
function _MakeIndex {
    param([ordered]$FeaturesHash)
    return [ordered]@{
        schema_version = 1
        features       = $FeaturesHash
    } | ConvertTo-Json -Depth 20 -Compress
}

# ── Fixtures ──────────────────────────────────────────────────────────────────

# Script feature dir
$ScriptDir = "$TmpRoot\repo\features\scriptfeat"
New-Item -ItemType Directory -Path $ScriptDir -Force | Out-Null

# Declarative feature dir (no install.ps1/uninstall.ps1)
$DeclDir = "$TmpRoot\repo\features\declfeat"
New-Item -ItemType Directory -Path $DeclDir -Force | Out-Null

# Declarative-but-has-install.ps1 (should be rejected)
$BadDeclDir = "$TmpRoot\repo\features\baddecl"
New-Item -ItemType Directory -Path $BadDeclDir -Force | Out-Null
"" | Set-Content "$BadDeclDir\install.ps1" -Encoding UTF8

# ── Test: mode:script → empty resources ──────────────────────────────────────

Write-Host "Invoke-FeatureCompilerRun: mode:script produces empty resources"

$scriptFeats = [ordered]@{ "core/scriptfeat" = (_MakeScriptEntry $ScriptDir) }
$scriptIndex = _MakeIndex $scriptFeats
$drgScript   = Invoke-FeatureCompilerRun -FeatureIndexJson $scriptIndex `
                   -SortedFeatures @("core/scriptfeat") 2>$null

$drgScriptObj = $drgScript | ConvertFrom-Json
Assert-Equal "DRG schema_version is 1" "1" "$($drgScriptObj.schema_version)"

$scriptFeatEntry = (_Prop $drgScriptObj.features "core/scriptfeat")
Assert-NotNull "core/scriptfeat present in DRG" $scriptFeatEntry
$resCount = if ($scriptFeatEntry.resources) { $scriptFeatEntry.resources.Count } else { 0 }
Assert-Equal "script feature has 0 resources" "0" "$resCount"

# ── Test: mode:declarative → resources expanded with stable id (no desired_backend) ──

Write-Host "Invoke-FeatureCompilerRun: mode:declarative expands resources with stable id"

$declResources = @(
    [ordered]@{ kind = "package"; name = "ripgrep" }
)
$declFeats = [ordered]@{ "core/declfeat" = (_MakeDeclarativeEntry $DeclDir $declResources) }
$declIndex = _MakeIndex $declFeats
$drgDecl   = Invoke-FeatureCompilerRun -FeatureIndexJson $declIndex `
                -SortedFeatures @("core/declfeat") 2>$null

$drgDeclObj  = $drgDecl | ConvertFrom-Json
$declFeat    = (_Prop $drgDeclObj.features "core/declfeat")
Assert-NotNull "core/declfeat in DRG" $declFeat
$resCountDecl = $declFeat.resources.Count
Assert-Equal "declarative feature has 1 resource" "1" "$resCountDecl"
$resId      = $declFeat.resources[0].id
Assert-Equal "resource id is package:ripgrep" "package:ripgrep" "$resId"

# Compiler must NOT embed desired_backend (that is PolicyResolver's job)
$resBackendProp = $declFeat.resources[0].PSObject.Properties["desired_backend"]
Assert-Null "compiler does not embed desired_backend" $resBackendProp

# ── Test: mode:declarative with no resources → error ($null) ──────────────────

Write-Host "Invoke-FeatureCompilerRun: declarative with no resources returns null"

$emptyDeclFeats = [ordered]@{
    "core/emptydecl" = (_MakeDeclarativeEntry $DeclDir @())
}
$emptyDeclIndex = _MakeIndex $emptyDeclFeats

$drgEmpty = $null
try {
    $drgEmpty = Invoke-FeatureCompilerRun `
        -FeatureIndexJson $emptyDeclIndex `
        -SortedFeatures @("core/emptydecl") 2>$null
} catch { $drgEmpty = $null }

Assert-Null "declarative with no resources returns null" $drgEmpty

# ── Test: mode:declarative with install.ps1 present → error ($null) ───────────

Write-Host "Invoke-FeatureCompilerRun: declarative with install.ps1 returns null"

$badResources = @( [ordered]@{ kind = "package"; name = "something" } )
$badDeclFeats = [ordered]@{
    "core/baddecl" = (_MakeDeclarativeEntry $BadDeclDir $badResources)
}
$badDeclIndex = _MakeIndex $badDeclFeats

$drgBad = $null
try {
    $drgBad = Invoke-FeatureCompilerRun `
        -FeatureIndexJson $badDeclIndex `
        -SortedFeatures @("core/baddecl") 2>$null
} catch { $drgBad = $null }

Assert-Null "declarative with install.ps1 returns null" $drgBad

# ── Test: multiple features in DRG ───────────────────────────────────────────

Write-Host "Invoke-FeatureCompilerRun: multiple features in DRG"

$multiFeats = [ordered]@{
    "core/scriptfeat" = (_MakeScriptEntry $ScriptDir)
    "core/declfeat"   = (_MakeDeclarativeEntry $DeclDir $declResources)
}
$multiIndex = _MakeIndex $multiFeats
$drgMulti   = Invoke-FeatureCompilerRun `
    -FeatureIndexJson $multiIndex `
    -SortedFeatures @("core/scriptfeat", "core/declfeat") 2>$null

$drgMultiObj = $drgMulti | ConvertFrom-Json
$keyCount    = $drgMultiObj.features.PSObject.Properties.Count
Assert-Equal "DRG has 2 feature entries" "2" "$keyCount"

# ── Test: fs resource gets id but no desired_backend ─────────────────────────

Write-Host "Invoke-FeatureCompilerRun: fs resource gets id only"

$fsResources = @( [ordered]@{ kind = "fs"; path = "/etc/myapp/config" } )
$fsFeat = [ordered]@{
    "core/fsfeature" = (_MakeDeclarativeEntry $DeclDir $fsResources)
}
$fsIndex = _MakeIndex $fsFeat
$drgFs   = Invoke-FeatureCompilerRun `
    -FeatureIndexJson $fsIndex `
    -SortedFeatures @("core/fsfeature") 2>$null

$drgFsObj  = $drgFs | ConvertFrom-Json
$fsFeatOut = (_Prop $drgFsObj.features "core/fsfeature")
Assert-NotNull "core/fsfeature in DRG" $fsFeatOut

$fsId      = $fsFeatOut.resources[0].id
Assert-Equal "fs resource id is fs:config" "fs:config" "$fsId"

$fsBackendProp = $fsFeatOut.resources[0].PSObject.Properties["desired_backend"]
Assert-Null "fs resource has no desired_backend property" $fsBackendProp

} finally {
    Remove-Item -Recurse -Force $TmpRoot -ErrorAction SilentlyContinue
}

# ── Summary ───────────────────────────────────────────────────────────────────

Show-TestSummary
