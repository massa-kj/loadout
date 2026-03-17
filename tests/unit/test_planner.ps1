# -----------------------------------------------------------------------------
# Unit tests: planner (Planner, PowerShell)
#
# Tests that Invoke-PlannerRun produces the correct plan for every
# classification case: create, destroy, noop (script), noop (identical),
# replace, replace_backend, strengthen, blocked, runtime version cases, mixed.
#
# DRG fixtures use the RRG format (desired_backend present in resources).
# Version comparison uses a profile YAML file passed to Invoke-PlannerRun.
#
# Run directly: pwsh tests/unit/test_planner.ps1
# Exit code 0 = all pass, 1 = one or more failures.
# -----------------------------------------------------------------------------

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$REPO_ROOT = (Get-Item "$PSScriptRoot/../..").FullName

. "$PSScriptRoot\helpers.ps1"

# ── Setup: temp directory for profile YAML files ───────────────────────────────

$TmpPlannerDir = [System.IO.Path]::GetTempPath() + [System.IO.Path]::GetRandomFileName()
New-Item -ItemType Directory -Path $TmpPlannerDir -Force | Out-Null
try {

# ── State stubs ───────────────────────────────────────────────────────────────
#
# Tests populate $script:TestStateJson (a JSON string) and these stubs
# parse it on each call to simulate the real State-* API.

$script:TestStateJson = '{"version":3,"features":{}}'

function State-HasFeature {
    param([string]$Feature)
    $s = $script:TestStateJson | ConvertFrom-Json
    return $s.features.PSObject.Properties[$Feature] -ne $null
}

function State-ListFeatures {
    $s = $script:TestStateJson | ConvertFrom-Json
    return @($s.features.PSObject.Properties | Select-Object -ExpandProperty Name)
}

function State-QueryResources {
    param([string]$Feature)
    $s = $script:TestStateJson | ConvertFrom-Json
    $feat = (_Prop $s.features $Feature)
    if ($feat -and (_Prop $feat 'resources')) {
        return @($feat.resources)
    }
    return @()
}

. "$REPO_ROOT\core\lib\planner.ps1"

# ── Test helpers ──────────────────────────────────────────────────────────────

# Assert-Operation <TestName> <Feature> <ExpectedOp> <PlanJson>
function Assert-Operation {
    param([string]$TestName, [string]$Feature, [string]$ExpectedOp, [string]$PlanJson)
    $plan   = $PlanJson | ConvertFrom-Json
    $action = @($plan.actions) | Where-Object { $_.feature -eq $Feature } | Select-Object -First 1
    $actual = if ($action) { $action.operation } else { "" }
    Assert-Equal $TestName $ExpectedOp $actual
}

# Assert-Noop <TestName> <Feature> <PlanJson>
function Assert-Noop {
    param([string]$TestName, [string]$Feature, [string]$PlanJson)
    $plan    = $PlanJson | ConvertFrom-Json
    $inNoops = @($plan.noops | Where-Object { $_.feature -eq $Feature }).Count
    Assert-Equal $TestName "1" "$inNoops"
}

# Assert-Blocked <TestName> <Feature> <PlanJson>
function Assert-Blocked {
    param([string]$TestName, [string]$Feature, [string]$PlanJson)
    $plan       = $PlanJson | ConvertFrom-Json
    $inBlocked  = @($plan.blocked | Where-Object { $_.feature -eq $Feature }).Count
    Assert-Equal $TestName "1" "$inBlocked"
}

# Assert-Summary <TestName> <Field> <Expected> <PlanJson>
function Assert-Summary {
    param([string]$TestName, [string]$Field, [int]$Expected, [string]$PlanJson)
    $plan   = $PlanJson | ConvertFrom-Json
    $actual = if ((_Prop $plan.summary $Field) -ne $null) {
        [int]$plan.summary.PSObject.Properties[$Field].Value
    } else { 0 }
    Assert-Equal $TestName "$Expected" "$actual"
}

# ── Test cases ────────────────────────────────────────────────────────────────

# ---------------------------------------------------------------------------
# 1. create: feature in DRG, not in state
# ---------------------------------------------------------------------------
Write-Host ""
Write-Host "── create ─────────────────────────────────────────────────────"

$script:TestStateJson = '{"version":3,"features":{}}'
$drg = '{"schema_version":1,"features":{"core/git":{"resources":[{"kind":"package","name":"git","id":"pkg:git","desired_backend":"brew"}]}}}'
$plan = Invoke-PlannerRun -DrgJson $drg -SortedFeatures @("core/git")

Assert-Operation "create: action=create"           "core/git" "create" $plan
Assert-Summary   "create: summary.create=1"        "create"   1        $plan
Assert-Summary   "create: summary.destroy=0"       "destroy"  0        $plan

# ---------------------------------------------------------------------------
# 2. destroy: feature in state, not in DRG desired
# ---------------------------------------------------------------------------
Write-Host ""
Write-Host "── destroy ────────────────────────────────────────────────────"

$script:TestStateJson = '{"version":3,"features":{"core/old":{"resources":[{"kind":"package","id":"pkg:old","backend":"brew","package":{"name":"old","version":null}}]}}}'
$drg = '{"schema_version":1,"features":{}}'
$plan = Invoke-PlannerRun -DrgJson $drg -SortedFeatures @()

Assert-Operation "destroy: action=destroy"         "core/old" "destroy" $plan
Assert-Summary   "destroy: summary.destroy=1"      "destroy"  1         $plan

# ---------------------------------------------------------------------------
# 3. noop (script): feature in both, empty DRG resources
# ---------------------------------------------------------------------------
Write-Host ""
Write-Host "── noop (script feature) ──────────────────────────────────────"

$script:TestStateJson = '{"version":3,"features":{"core/bash":{"resources":[]}}}'
$drg = '{"schema_version":1,"features":{"core/bash":{"resources":[]}}}'
$plan = Invoke-PlannerRun -DrgJson $drg -SortedFeatures @("core/bash")

Assert-Noop    "noop(script): shows as noop"       "core/bash" $plan
Assert-Summary "noop(script): summary.noop=1"      "noop"      1       $plan

# ---------------------------------------------------------------------------
# 4. noop (declarative identical): same package resources in desired and state
# ---------------------------------------------------------------------------
Write-Host ""
Write-Host "── noop (declarative identical) ───────────────────────────────"

$script:TestStateJson = '{"version":3,"features":{"core/git":{"resources":[{"kind":"package","id":"pkg:git","backend":"brew","package":{"name":"git","version":null}}]}}}'
$drg = '{"schema_version":1,"features":{"core/git":{"resources":[{"kind":"package","name":"git","id":"pkg:git","desired_backend":"brew"}]}}}'
$plan = Invoke-PlannerRun -DrgJson $drg -SortedFeatures @("core/git")

Assert-Noop    "noop(decl): shows as noop"         "core/git"  $plan
Assert-Summary "noop(decl): summary.noop=1"        "noop"      1       $plan

# ---------------------------------------------------------------------------
# 5. replace: incompatible fs resource (target path change)
# ---------------------------------------------------------------------------
Write-Host ""
Write-Host "── replace (incompatible fs) ──────────────────────────────────"

$script:TestStateJson = '{"version":3,"features":{"user/git":{"resources":[{"kind":"fs","id":"fs:gitconfig","backend":"fs","fs":{"path":"/home/user/.gitconfig","entry_type":"file","op":"link"}}]}}}'
$drg = '{"schema_version":1,"features":{"user/git":{"resources":[{"kind":"fs","name":"gitconfig","id":"fs:gitconfig","target":"/home/user/.config/git/config","entry_type":"file","op":"link","desired_backend":"fs"}]}}}'
$plan = Invoke-PlannerRun -DrgJson $drg -SortedFeatures @("user/git")

Assert-Operation "replace(fs path): action=replace"  "user/git" "replace" $plan
Assert-Summary   "replace(fs path): summary=1"        "replace"  1         $plan

# ---------------------------------------------------------------------------
# 6. replace_backend: same package key, different backend
# ---------------------------------------------------------------------------
Write-Host ""
Write-Host "── replace_backend ────────────────────────────────────────────"

$script:TestStateJson = '{"version":3,"features":{"core/node":{"resources":[{"kind":"package","id":"pkg:nodejs","backend":"brew","package":{"name":"nodejs","version":null}}]}}}'
$drg = '{"schema_version":1,"features":{"core/node":{"resources":[{"kind":"package","name":"nodejs","id":"pkg:nodejs","desired_backend":"apt"}]}}}'
$plan = Invoke-PlannerRun -DrgJson $drg -SortedFeatures @("core/node")

Assert-Operation "replace_backend: action=replace_backend"  "core/node" "replace_backend" $plan
Assert-Summary   "replace_backend: summary=1"               "replace_backend" 1             $plan

# ---------------------------------------------------------------------------
# 7. strengthen: state ⊂ desired, all common compatible, desired has extras
# ---------------------------------------------------------------------------
Write-Host ""
Write-Host "── strengthen ─────────────────────────────────────────────────"

$script:TestStateJson = '{"version":3,"features":{"core/dev":{"resources":[{"kind":"package","id":"pkg:git","backend":"brew","package":{"name":"git","version":null}}]}}}'
$drg = '{"schema_version":1,"features":{"core/dev":{"resources":[{"kind":"package","name":"git","id":"pkg:git","desired_backend":"brew"},{"kind":"package","name":"curl","id":"pkg:curl","desired_backend":"brew"}]}}}'
$plan = Invoke-PlannerRun -DrgJson $drg -SortedFeatures @("core/dev")

Assert-Operation "strengthen: action=strengthen"     "core/dev" "strengthen" $plan
Assert-Summary   "strengthen: summary=1"             "strengthen" 1           $plan
$planObj  = $plan | ConvertFrom-Json
$addCount = @(($planObj.actions | Where-Object { $_.feature -eq "core/dev" }).details.add_resources).Count
Assert-Equal "strengthen: add_resources has 1 entry" "1" "$addCount"

# ---------------------------------------------------------------------------
# 8. strengthen boundary: state has resource NOT in desired → replace
# ---------------------------------------------------------------------------
Write-Host ""
Write-Host "── strengthen boundary (s_only → replace) ─────────────────────"

$script:TestStateJson = '{"version":3,"features":{"core/dev":{"resources":[{"kind":"package","id":"pkg:git","backend":"brew","package":{"name":"git","version":null}},{"kind":"package","id":"pkg:curl","backend":"brew","package":{"name":"curl","version":null}}]}}}'
$drg = '{"schema_version":1,"features":{"core/dev":{"resources":[{"kind":"package","name":"git","id":"pkg:git","desired_backend":"brew"}]}}}'
$plan = Invoke-PlannerRun -DrgJson $drg -SortedFeatures @("core/dev")

Assert-Operation "strengthen-boundary: s_only→replace" "core/dev" "replace" $plan

# ---------------------------------------------------------------------------
# 9. blocked: unknown resource kind in desired
# ---------------------------------------------------------------------------
Write-Host ""
Write-Host "── blocked (unknown kind in desired) ──────────────────────────"

$script:TestStateJson = '{"version":3,"features":{}}'
$drg = '{"schema_version":1,"features":{"user/legacy":{"resources":[{"kind":"registry","name":"foo","id":"reg:foo","desired_backend":"winreg"}]}}}'
$plan = Invoke-PlannerRun -DrgJson $drg -SortedFeatures @("user/legacy")

Assert-Blocked "blocked(desired unknown): in blocked list"   "user/legacy" $plan
Assert-Summary "blocked(desired unknown): summary.blocked=1" "blocked"  1  $plan

# ---------------------------------------------------------------------------
# 10. blocked: unknown resource kind in state
# ---------------------------------------------------------------------------
Write-Host ""
Write-Host "── blocked (unknown kind in state) ────────────────────────────"

$script:TestStateJson = '{"version":3,"features":{"user/legacy":{"resources":[{"kind":"registry","id":"reg:foo","backend":"winreg","registry":{"name":"foo"}}]}}}'
$drg = '{"schema_version":1,"features":{"user/legacy":{"resources":[{"kind":"package","name":"git","id":"pkg:git","desired_backend":"brew"}]}}}'
$plan = Invoke-PlannerRun -DrgJson $drg -SortedFeatures @("user/legacy")

Assert-Blocked "blocked(state unknown): in blocked list"   "user/legacy" $plan
Assert-Summary "blocked(state unknown): summary.blocked=1" "blocked"  1  $plan

# ---------------------------------------------------------------------------
# 11. runtime: version mismatch → replace  (version read from profile file)
# ---------------------------------------------------------------------------
Write-Host ""
Write-Host "── runtime version mismatch → replace ─────────────────────────"

$_profileV11 = Join-Path $TmpPlannerDir "profile_v11.yaml"
@"
features:
  tools/node:
    version: "20.0.0"
"@ | Set-Content -Path $_profileV11 -Encoding UTF8

$script:TestStateJson = '{"version":3,"features":{"tools/node":{"resources":[{"kind":"runtime","id":"rt:node","backend":"mise","runtime":{"name":"node","version":"18.0.0"}}]}}}'
$drg = '{"schema_version":1,"features":{"tools/node":{"resources":[{"kind":"runtime","name":"node","id":"rt:node","desired_backend":"mise"}]}}}'
$plan = Invoke-PlannerRun -DrgJson $drg -SortedFeatures @("tools/node") -ProfileFile $_profileV11

Assert-Operation "runtime version mismatch: action=replace" "tools/node" "replace" $plan

# ---------------------------------------------------------------------------
# 12. runtime: version match → noop  (version read from profile file)
# ---------------------------------------------------------------------------
Write-Host ""
Write-Host "── runtime version match → noop ───────────────────────────────"

$_profileV12 = Join-Path $TmpPlannerDir "profile_v12.yaml"
@"
features:
  tools/node:
    version: "20.0.0"
"@ | Set-Content -Path $_profileV12 -Encoding UTF8

$script:TestStateJson = '{"version":3,"features":{"tools/node":{"resources":[{"kind":"runtime","id":"rt:node","backend":"mise","runtime":{"name":"node","version":"20.0.0"}}]}}}'
$drg = '{"schema_version":1,"features":{"tools/node":{"resources":[{"kind":"runtime","name":"node","id":"rt:node","desired_backend":"mise"}]}}}'
$plan = Invoke-PlannerRun -DrgJson $drg -SortedFeatures @("tools/node") -ProfileFile $_profileV12

Assert-Noop "runtime version match: noop" "tools/node" $plan

# ---------------------------------------------------------------------------
# 13. runtime: no version in profile → noop (no version constraint)
# ---------------------------------------------------------------------------
Write-Host ""
Write-Host "── runtime no version in profile → noop ───────────────────────"

$script:TestStateJson = '{"version":3,"features":{"tools/python":{"resources":[{"kind":"runtime","id":"rt:python","backend":"mise","runtime":{"name":"python","version":"3.11.0"}}]}}}'
$drg = '{"schema_version":1,"features":{"tools/python":{"resources":[{"kind":"runtime","name":"python","id":"rt:python","desired_backend":"mise"}]}}}'
$plan = Invoke-PlannerRun -DrgJson $drg -SortedFeatures @("tools/python")

Assert-Noop "runtime no version constraint: noop" "tools/python" $plan

# ---------------------------------------------------------------------------
# 14. mixed: create + destroy + noop in a single plan
# ---------------------------------------------------------------------------
Write-Host ""
Write-Host "── mixed (create + destroy + noop) ────────────────────────────"

$script:TestStateJson = '{"version":3,"features":{"core/old":{"resources":[{"kind":"package","id":"pkg:old","backend":"brew","package":{"name":"old","version":null}}]},"core/bash":{"resources":[]}}}'
$drg = '{"schema_version":1,"features":{"core/new":{"resources":[{"kind":"package","name":"new","id":"pkg:new","desired_backend":"brew"}]},"core/bash":{"resources":[]}}}'
$plan = Invoke-PlannerRun -DrgJson $drg -SortedFeatures @("core/new", "core/bash")

Assert-Operation "mixed: core/new=create"      "core/new"  "create"  $plan
Assert-Operation "mixed: core/old=destroy"     "core/old"  "destroy" $plan
Assert-Noop      "mixed: core/bash=noop"       "core/bash"           $plan
Assert-Summary   "mixed: summary.create=1"     "create"   1          $plan
Assert-Summary   "mixed: summary.destroy=1"    "destroy"  1          $plan
Assert-Summary   "mixed: summary.noop=1"       "noop"     1          $plan

# ── Summary ───────────────────────────────────────────────────────────────────

Write-Host ""
Show-TestSummary

} finally {
    Remove-Item -Recurse -Force $TmpPlannerDir -ErrorAction SilentlyContinue
}
