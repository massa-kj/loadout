# -----------------------------------------------------------------------------
# Unit tests: declarative_executor (PowerShell)
#
# Tests that Invoke-DeclarativeExecutorRun correctly installs/uninstalls
# resources for mode:declarative features across all plan operations.
#
# Test coverage:
#   1.  create:   package resource → backend dispatched, state recorded
#   2.  create:   fs resource (link op) → symlink created, state recorded
#   3.  create:   fs resource (copy op) → file copied, state recorded
#   4.  create:   fs resource (fallback source) → basename convention applied
#   5.  destroy:  → _Executor-RemoveResources called, feature removed from state
#   6.  replace:  → remove_resources + fresh install of all spec resources
#   7.  replace_backend: same as replace
#   8.  strengthen: only add_resources installed; other spec resources skipped
#   9.  strengthen: empty add_resources → noop
#   10. create:   runtime resource → InstallRuntime dispatched, state recorded
#
# Run directly: pwsh tests/unit/test_declarative_executor.ps1
# Exit code 0 = all pass, 1 = one or more failures.
# -----------------------------------------------------------------------------

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$REPO_ROOT = (Get-Item "$PSScriptRoot/../..").FullName

. "$PSScriptRoot\helpers.ps1"

$TmpRoot = [System.IO.Path]::GetTempPath() + [System.IO.Path]::GetRandomFileName()
New-Item -ItemType Directory -Path $TmpRoot -Force | Out-Null

try {

# ── Stub: _Executor-* helpers ─────────────────────────────────────────────────

$script:TestFeatureDir = ""

function _Executor-GetFeatureDir {
    param([string]$Feature)
    return $script:TestFeatureDir
}
function _Executor-ResolveFeature {
    param([string]$Feature)
    return (Join-Path $script:TestFeatureDir "feature.yaml")
}
function _Executor-ResolvePlatformFeature {
    param([string]$Feature)
    return $null
}

function _Executor-ExpandPath {
    param([string]$Path)
    $expanded = $Path -replace '^~', $HOME
    return [System.IO.Path]::GetFullPath($expanded)
}

function _Executor-TrySymlink {
    param([string]$Src, [string]$Dst)
    try {
        New-Item -ItemType SymbolicLink -Path $Dst -Target $Src -Force -ErrorAction Stop | Out-Null
        return $true
    } catch { return $false }
}

function _Executor-TryJunction {
    param([string]$Src, [string]$Dst)
    return $false  # Not needed for test scenarios
}

$script:RemovedResources = [System.Collections.Generic.List[string]]::new()

function _Executor-RemoveResources {
    param([string]$Feature)
    $script:RemovedResources.Add($Feature)
    return $true
}

# ── Stub: backend functions ───────────────────────────────────────────────────

$script:BackendInstalls   = [System.Collections.Generic.List[string]]::new()
$script:BackendUninstalls = [System.Collections.Generic.List[string]]::new()

function Resolve-BackendFor { param([string]$Kind, [string]$Name); return "stub_backend" }
function Load-Backend       { param([string]$BackendId); return $true }

function Backend-Call {
    param([string]$Operation)
    $args2 = $args  # remaining positional args
    switch ($Operation) {
        "PackageExists"    { return $false }
        "InstallPackage"   {
            $script:BackendInstalls.Add("pkg:$($args2[0])")
            $global:LASTEXITCODE = 0
            return $null
        }
        "UninstallPackage" {
            $script:BackendUninstalls.Add("pkg:$($args2[0])")
            $global:LASTEXITCODE = 0
            return $null
        }
        "RuntimeExists"    { return $false }
        "InstallRuntime"   {
            $script:BackendInstalls.Add("rt:$($args2[0])@$($args2[1])")
            $global:LASTEXITCODE = 0
            return $args2[1]  # echo version back
        }
        "UninstallRuntime" {
            $script:BackendUninstalls.Add("rt:$($args2[0])@$($args2[1])")
            $global:LASTEXITCODE = 0
            return $null
        }
    }
}

# ── Stub: state functions ─────────────────────────────────────────────────────

$script:PatchBeginCount    = 0
$script:PatchFinalizeCount = 0
$script:PatchResources     = [System.Collections.Generic.List[string]]::new()
$script:PatchRemoved       = [System.Collections.Generic.List[string]]::new()

function State-PatchBegin          { $script:PatchBeginCount++ }
function State-PatchFinalize       { $script:PatchFinalizeCount++ }
function State-PatchAddResource    {
    param([string]$Feature, [object]$ResourceObject)
    $rid = if ($ResourceObject.PSObject.Properties['id']) { $ResourceObject.id } else { "?" }
    $script:PatchResources.Add("${Feature}:${rid}")
}
function State-PatchRemoveFeature  { param([string]$Feature); $script:PatchRemoved.Add($Feature) }
function State-HasFile             { param([string]$File); return $false }
function State-HasFeature          { param([string]$Feature); return $false }

# ── Source module under test ──────────────────────────────────────────────────

. "$REPO_ROOT\core\lib\declarative_executor.ps1"

# ── Reset helper ──────────────────────────────────────────────────────────────

function Reset-State {
    $script:BackendInstalls.Clear()
    $script:BackendUninstalls.Clear()
    $script:RemovedResources.Clear()
    $script:PatchBeginCount    = 0
    $script:PatchFinalizeCount = 0
    $script:PatchResources.Clear()
    $script:PatchRemoved.Clear()
}

# _Make-FeatureYaml <Dir> <ResourcesYaml>
function _Make-FeatureYaml {
    param([string]$Dir, [string]$ResourcesYaml)
    $yaml = @"
spec_version: 1
mode: declarative
description: test feature
depends: []

resources:
$ResourcesYaml
"@
    Set-Content -Path (Join-Path $Dir "feature.yaml") -Value $yaml -NoNewline:$false
}

# Assert-ContainsItem <TestName> <Expected> <List>
function Assert-ContainsItem {
    param([string]$TestName, [string]$Expected, [object]$List)
    $found = $List -contains $Expected
    if ($found) {
        Write-Host "  PASS  $TestName"
        $script:PassCount++
    } else {
        Write-Host "  FAIL  $TestName"
        Write-Host "        expected item: '$Expected'"
        Write-Host "        list: $($List -join ', ')"
        $script:FailCount++
    }
}

# Assert-NotContainsItem <TestName> <NotExpected> <List>
function Assert-NotContainsItem {
    param([string]$TestName, [string]$NotExpected, [object]$List)
    $found = $List -contains $NotExpected
    if (-not $found) {
        Write-Host "  PASS  $TestName"
        $script:PassCount++
    } else {
        Write-Host "  FAIL  $TestName"
        Write-Host "        unexpected item '$NotExpected' found in list"
        $script:FailCount++
    }
}

# ── Test 1: create – package resource ─────────────────────────────────────────

Write-Host ""
Write-Host "── create: package resource ─────────────────────────────────────────────"

$dir1 = Join-Path $TmpRoot "feat_pkg"
New-Item -ItemType Directory -Path $dir1 -Force | Out-Null
$script:TestFeatureDir = $dir1
_Make-FeatureYaml -Dir $dir1 -ResourcesYaml "  - kind: package`n    id: package:jq`n    name: jq"

Reset-State
Invoke-DeclarativeExecutorRun -Feature "core/test" -Operation "create"

Assert-Equal          "create:pkg: patch began once"            "1"                  "$($script:PatchBeginCount)"
Assert-Equal          "create:pkg: patch finalized once"        "1"                  "$($script:PatchFinalizeCount)"
Assert-ContainsItem   "create:pkg: install_package jq"          "pkg:jq"             $script:BackendInstalls
Assert-ContainsItem   "create:pkg: state recorded jq"           "core/test:package:jq" $script:PatchResources

# ── Test 2: create – fs resource (link op) ────────────────────────────────────

Write-Host ""
Write-Host "── create: fs resource (link) ────────────────────────────────────────────"

$dir2     = Join-Path $TmpRoot "feat_fs"
$fsFiles  = Join-Path $dir2 "files"
New-Item -ItemType Directory -Path $fsFiles -Force | Out-Null
Set-Content -Path (Join-Path $fsFiles "marker") -Value "test content"
$fsTarget = Join-Path $TmpRoot "home" ".config" "loadout" "marker"
$script:TestFeatureDir = $dir2
_Make-FeatureYaml -Dir $dir2 -ResourcesYaml @"
  - kind: fs
    id: fs:test-marker
    source: files/marker
    path: $($fsTarget -replace '\\', '/')
    entry_type: file
    op: link
"@

Reset-State
Invoke-DeclarativeExecutorRun -Feature "core/testfs" -Operation "create"

Assert-Equal          "create:fs: patch began once"              "1"                           "$($script:PatchBeginCount)"
Assert-Equal          "create:fs: patch finalized once"          "1"                           "$($script:PatchFinalizeCount)"
Assert-ContainsItem   "create:fs: state recorded fs:test-marker" "core/testfs:fs:test-marker"  $script:PatchResources
Assert-Equal          "create:fs: link/copy created"             "True"                        "$(Test-Path $fsTarget)"

# ── Test 3: create – fs resource (copy op) ────────────────────────────────────

Write-Host ""
Write-Host "── create: fs resource (copy op) ────────────────────────────────────────"

$dir3      = Join-Path $TmpRoot "feat_fs_copy"
$fsFiles3  = Join-Path $dir3 "files"
New-Item -ItemType Directory -Path $fsFiles3 -Force | Out-Null
Set-Content -Path (Join-Path $fsFiles3 "copyfile") -Value "copy content"
$copyTarget = Join-Path $TmpRoot "home" ".config" "loadout" "copyfile"
$script:TestFeatureDir = $dir3
_Make-FeatureYaml -Dir $dir3 -ResourcesYaml @"
  - kind: fs
    id: fs:test-copy
    source: files/copyfile
    path: $($copyTarget -replace '\\', '/')
    entry_type: file
    op: copy
"@

Reset-State
Invoke-DeclarativeExecutorRun -Feature "core/testfscopy" -Operation "create"

Assert-Equal          "create:fs(copy): file created"            "True"                            "$(Test-Path $copyTarget)"
Assert-ContainsItem   "create:fs(copy): state recorded"          "core/testfscopy:fs:test-copy"    $script:PatchResources

# ── Test 4: create – fs resource (fallback source by convention) ──────────────

Write-Host ""
Write-Host "── create: fs resource (source fallback: files/<basename>) ──────────────"

$dir4      = Join-Path $TmpRoot "feat_fallback"
$fsFiles4  = Join-Path $dir4 "files"
New-Item -ItemType Directory -Path $fsFiles4 -Force | Out-Null
Set-Content -Path (Join-Path $fsFiles4 "marker2") -Value "fallback content"
$fallbackTarget = Join-Path $TmpRoot "home" ".config" "loadout" "marker2"
$script:TestFeatureDir = $dir4
_Make-FeatureYaml -Dir $dir4 -ResourcesYaml @"
  - kind: fs
    id: fs:fallback
    path: $($fallbackTarget -replace '\\', '/')
    entry_type: file
    op: link
"@

Reset-State
Invoke-DeclarativeExecutorRun -Feature "core/testfallback" -Operation "create"

Assert-Equal          "create:fs(fallback): path created"        "True"                              "$(Test-Path $fallbackTarget)"
Assert-ContainsItem   "create:fs(fallback): state recorded"      "core/testfallback:fs:fallback"     $script:PatchResources

# ── Test 5: destroy ────────────────────────────────────────────────────────────

Write-Host ""
Write-Host "── destroy ──────────────────────────────────────────────────────────────"

$script:TestFeatureDir = $dir1

Reset-State
Invoke-DeclarativeExecutorRun -Feature "core/test" -Operation "destroy"

Assert-Equal          "destroy: patch began once"                   "1"          "$($script:PatchBeginCount)"
Assert-Equal          "destroy: patch finalized once"               "1"          "$($script:PatchFinalizeCount)"
Assert-ContainsItem   "destroy: _Executor-RemoveResources called"   "core/test"  $script:RemovedResources
Assert-ContainsItem   "destroy: State-PatchRemoveFeature called"    "core/test"  $script:PatchRemoved

# ── Test 6: replace ────────────────────────────────────────────────────────────

Write-Host ""
Write-Host "── replace ──────────────────────────────────────────────────────────────"

$script:TestFeatureDir = $dir1

Reset-State
Invoke-DeclarativeExecutorRun -Feature "core/test" -Operation "replace"

Assert-Equal          "replace: patch began twice"                  "2"          "$($script:PatchBeginCount)"
Assert-Equal          "replace: patch finalized twice"              "2"          "$($script:PatchFinalizeCount)"
Assert-ContainsItem   "replace: remove_resources called"            "core/test"  $script:RemovedResources
Assert-ContainsItem   "replace: State-PatchRemoveFeature called"    "core/test"  $script:PatchRemoved
Assert-ContainsItem   "replace: install_package jq re-run"          "pkg:jq"     $script:BackendInstalls

# ── Test 7: replace_backend ────────────────────────────────────────────────────

Write-Host ""
Write-Host "── replace_backend ──────────────────────────────────────────────────────"

$script:TestFeatureDir = $dir1

Reset-State
Invoke-DeclarativeExecutorRun -Feature "core/test" -Operation "replace_backend"

Assert-ContainsItem   "replace_backend: remove_resources called"    "core/test"  $script:RemovedResources
Assert-ContainsItem   "replace_backend: install_package jq"         "pkg:jq"     $script:BackendInstalls

# ── Test 8: strengthen – only add_resources installed ─────────────────────────

Write-Host ""
Write-Host "── strengthen: only add_resources installed ─────────────────────────────"

$dir8 = Join-Path $TmpRoot "feat_strengthen"
New-Item -ItemType Directory -Path $dir8 -Force | Out-Null
$script:TestFeatureDir = $dir8
_Make-FeatureYaml -Dir $dir8 -ResourcesYaml @"
  - kind: package
    id: package:git
    name: git
  - kind: package
    id: package:curl
    name: curl
"@

$strengthenDetails = [PSCustomObject]@{
    add_resources = @([PSCustomObject]@{ kind = "package"; id = "package:curl" })
}

Reset-State
Invoke-DeclarativeExecutorRun -Feature "core/teststrengthen" -Operation "strengthen" -Details $strengthenDetails

Assert-Equal          "strengthen: patch began once"               "1"      "$($script:PatchBeginCount)"
Assert-Equal          "strengthen: patch finalized once"           "1"      "$($script:PatchFinalizeCount)"
Assert-ContainsItem   "strengthen: curl installed"                 "pkg:curl" $script:BackendInstalls
Assert-NotContainsItem "strengthen: git NOT installed"             "pkg:git"  $script:BackendInstalls

# ── Test 9: strengthen – empty add_resources is noop ─────────────────────────

Write-Host ""
Write-Host "── strengthen: empty add_resources → noop ───────────────────────────────"

$script:TestFeatureDir = $dir8

$strengthenEmpty = [PSCustomObject]@{ add_resources = @() }

Reset-State
Invoke-DeclarativeExecutorRun -Feature "core/teststrengthen" -Operation "strengthen" -Details $strengthenEmpty

Assert-Equal "strengthen(empty): no install calls"   "0"  "$($script:BackendInstalls.Count)"
Assert-Equal "strengthen(empty): patch not begun"    "0"  "$($script:PatchBeginCount)"

# ── Test 10: create – runtime resource ────────────────────────────────────────

Write-Host ""
Write-Host "── create: runtime resource ─────────────────────────────────────────────"

$dir10 = Join-Path $TmpRoot "feat_runtime"
New-Item -ItemType Directory -Path $dir10 -Force | Out-Null
$script:TestFeatureDir = $dir10
_Make-FeatureYaml -Dir $dir10 -ResourcesYaml @"
  - kind: runtime
    id: runtime:node
    name: node
    version: "20.0.0"
"@

Reset-State
Invoke-DeclarativeExecutorRun -Feature "core/testrt" -Operation "create"

Assert-ContainsItem "create:rt: install_runtime called"    "rt:node@20.0.0"           $script:BackendInstalls
Assert-ContainsItem "create:rt: state recorded"            "core/testrt:runtime:node" $script:PatchResources

# ── Summary ────────────────────────────────────────────────────────────────────

Write-Host ""
Show-TestSummary

} finally {
    Remove-Item -Path $TmpRoot -Recurse -Force -ErrorAction SilentlyContinue
}
