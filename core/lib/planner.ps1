# -----------------------------------------------------------------------------
# Module: planner (PowerShell)
#
# Responsibility:
#   PURE decision engine. Converts (desired_resource_graph, state) into a
#   structured plan object describing what the executor should do.
#   Never executes anything.
#
# Public API:
#   Invoke-PlannerRun <DrgJson> <SortedFeatures> <ProfileFile>  → plan JSON string
#
# Internal phases (each PURE function):
#   _Planner-Diff       rrg × state → diff array (includes profile_version per feature)
#   _Planner-Classify   diff array  → classified array
#   _Planner-Decide     classified  → plan PSCustomObject
#
# Inputs:
#   DrgJson        — ResolvedResourceGraph JSON (DRG + desired_backend, from PolicyResolver)
#   SortedFeatures — topologically sorted canonical feature IDs
#   ProfileFile    — path to profile YAML; used to read version hints per feature
#   State-*        — public state API (read-only)
#
# Planner receives policy-resolved resources (desired_backend in RRG) and reads
# profile version hints directly; it does NOT receive raw policy objects.
#
# Plan JSON schema:
#   {
#     "actions": [
#       {"feature": "core/git",   "operation": "create",   "details": {}},
#       {"feature": "core/node",  "operation": "replace",  "details": {}},
#       {"feature": "core/tmux",  "operation": "strengthen",
#        "details": {"add_resources": [{"kind": "fs", "id": "fs:tmux.conf"}]}}
#     ],
#     "noops":   [{"feature": "core/bash"}],
#     "blocked": [{"feature": "user/legacy", "reason": "unknown resource kind: registry"}],
#     "summary": {"create":1, "destroy":0, "replace":1, "replace_backend":0,
#                 "strengthen":1, "noop":1, "blocked":0}
#   }
#
# Classification table:
#   in_desired=false, in_state=true                              → destroy
#   in_desired=true,  in_state=false                             → create
#   in_desired=true,  in_state=true, desired_resources empty     → noop  (script feature)
#   in_desired=true,  in_state=true, incompatible resource change → replace
#   in_desired=true,  in_state=true, backend mismatch only       → replace_backend
#   in_desired=true,  in_state=true, strict superset + compat    → strengthen
#   in_desired=true,  in_state=true, identical resources         → noop
#   unknown resource kind in desired or state                    → blocked
# -----------------------------------------------------------------------------

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Valid resource kinds.  Anything else causes a "blocked" classification.
$script:PlannerValidKinds = @("package", "runtime", "fs")

# Profile file path set by Invoke-PlannerRun and read by _Planner-Diff.
$script:PlannerProfileFile = ""

# ── Profile helpers ───────────────────────────────────────────────────────────────

# _Planner-ProfileVersion <ProfileFile> <CanonicalId>
# Extract version hint for a feature from its profile YAML entry.
# Tries the canonical ID key first, then falls back to the bare name.
# Returns empty string if not found or no version is set.
function _Planner-ProfileVersion {
    param(
        [Parameter(Mandatory=$true)] [string]$ProfileFile,
        [Parameter(Mandatory=$true)] [string]$CanonicalId
    )

    if (-not (Test-Path $ProfileFile)) { return "" }

    try {
        # Try canonical ID (e.g. "tools/node")
        # Use env(YQ_KEY) to avoid PS 5.1 double-quote stripping on native exe args
        $env:YQ_KEY = $CanonicalId
        $val = Get-Content $ProfileFile -Raw |
               & yq eval '.features[env(YQ_KEY)].version' - 2>$null
        if ($LASTEXITCODE -eq 0 -and
            -not [string]::IsNullOrWhiteSpace($val) -and
            $val -ne 'null' -and $val -ne '') {
            return $val
        }

        # Fallback: bare name (e.g. "node")
        $bareName = if ($CanonicalId -match '/') { $CanonicalId -replace '^[^/]+/', '' } else { $CanonicalId }
        if ($bareName -ne $CanonicalId) {
            $env:YQ_KEY = $bareName
            $val = Get-Content $ProfileFile -Raw |
                   & yq eval '.features[env(YQ_KEY)].version' - 2>$null
            if ($LASTEXITCODE -eq 0 -and
                -not [string]::IsNullOrWhiteSpace($val) -and
                $val -ne 'null' -and $val -ne '') {
                return $val
            }
        }
    } catch { }

    return ""
}

# ── Semantic key helpers ──────────────────────────────────────────────────────

# _Planner-DesiredSemanticKey <Res>
# Returns a stable semantic key for a desired resource (from DRG):
#   package → "pkg:<name>"
#   runtime → "rt:<name>"
#   fs      → "fs:<basename(target or path)>"
function _Planner-DesiredSemanticKey {
    param([Parameter(Mandatory=$true)] [object]$Res)

    $kind = (_Prop $Res 'kind')
    switch ($kind) {
        "package" { return "pkg:" + ((_Coal (_Prop $Res 'name') "?")) }
        "runtime" { return "rt:"  + ((_Coal (_Prop $Res 'name') "?")) }
        "fs" {
            $target = (_Prop $Res 'target')
            $path   = (_Prop $Res 'path')
            $p      = if ($target) { $target } elseif ($path) { $path } else { "" }
            return "fs:" + [System.IO.Path]::GetFileName($p)
        }
        default { return "other:$kind" }
    }
}

# _Planner-StateSemanticKey <Res>
# Returns a stable semantic key for a state resource:
#   package → "pkg:<package.name>"
#   runtime → "rt:<runtime.name>"
#   fs      → "fs:<basename(fs.path)>"
function _Planner-StateSemanticKey {
    param([Parameter(Mandatory=$true)] [object]$Res)

    $kind = (_Prop $Res 'kind')
    switch ($kind) {
        "package" {
            $pkg  = (_Prop $Res 'package')
            $name = if ($pkg) { (_Prop $pkg 'name') } else { $null }
            return "pkg:" + ((_Coal $name "?"))
        }
        "runtime" {
            $rt   = (_Prop $Res 'runtime')
            $name = if ($rt) { (_Prop $rt 'name') } else { $null }
            return "rt:" + ((_Coal $name "?"))
        }
        "fs" {
            $fs = (_Prop $Res 'fs')
            $p  = if ($fs) { (_Coal (_Prop $fs 'path') "") } else { "" }
            return "fs:" + [System.IO.Path]::GetFileName($p)
        }
        default { return "other:$kind" }
    }
}

# _Planner-CheckResourceCompat <DesiredRes> <StateRes> <ProfileVersion>
# Compare one desired resource against its matched state resource.
# Returns one of: "compatible", "backend_mismatch", "version_mismatch", "incompatible".
function _Planner-CheckResourceCompat {
    param(
        [Parameter(Mandatory=$true)]  [object]$DesiredRes,
        [Parameter(Mandatory=$true)]  [object]$StateRes,
        [Parameter(Mandatory=$false)] [string]$ProfileVersion = ""
    )

    $kind = (_Prop $DesiredRes 'kind')
    switch ($kind) {
        "package" {
            $dBackend = (_Coal (_Prop $DesiredRes 'desired_backend') "?")
            $sBackend = (_Coal (_Prop $StateRes 'backend') "?")
            if ($dBackend -ne $sBackend) { return "backend_mismatch" }
            return "compatible"
        }
        "runtime" {
            $dBackend = (_Coal (_Prop $DesiredRes 'desired_backend') "?")
            $sBackend = (_Coal (_Prop $StateRes 'backend') "?")
            if ($dBackend -ne $sBackend) { return "backend_mismatch" }

            # Use profile-supplied version for comparison (not a field on the RRG resource)
            if ($ProfileVersion -ne "") {
                $rt   = (_Prop $StateRes 'runtime')
                $sVer = if ($rt) { (_Prop $rt 'version') } else { $null }
                if ($ProfileVersion -ne ((_Coal $sVer ""))) { return "version_mismatch" }
            }
            return "compatible"
        }
        "fs" {
            $dTarget = (_Coal (_Coal (_Prop $DesiredRes 'target') (_Prop $DesiredRes 'path')) "")
            $fs    = (_Prop $StateRes 'fs')
            $sPath = if ($fs) { (_Coal (_Prop $fs 'path') "") } else { "" }
            if ($dTarget -ne $sPath) { return "incompatible" }

            $dEt = (_Prop $DesiredRes 'entry_type')
            if ($dEt -and $dEt -ne "") {
                $sEt = if ($fs) { (_Prop $fs 'entry_type') } else { $null }
                if ($dEt -ne ((_Coal $sEt ""))) { return "incompatible" }
            }

            $dOp = (_Coal (_Prop $DesiredRes 'op') "link")
            $sOp = if ($fs) { (_Coal (_Prop $fs 'op') "link") } else { "link" }
            if ($dOp -ne $sOp) { return "incompatible" }

            return "compatible"
        }
        default { return "compatible" }
    }
}

# ── State helpers (read-only) ─────────────────────────────────────────────────

function _Planner-StateHasUnknownKind {
    param([string]$Feature)
    $resources = @(State-QueryResources -Feature $Feature)
    foreach ($r in $resources) {
        if ((_Prop $r 'kind') -notin $script:PlannerValidKinds) { return $true }
    }
    return $false
}

function _Planner-StateUnknownKindsList {
    param([string]$Feature)
    $resources = @(State-QueryResources -Feature $Feature)
    $unknown   = @($resources | Where-Object {
        (_Prop $_ 'kind') -notin $script:PlannerValidKinds
    } | Select-Object -ExpandProperty kind | Sort-Object -Unique)
    return ($unknown -join ", ")
}

# ── Phase 1: Diff ─────────────────────────────────────────────────────────────

# _Planner-Diff <DrgJson> <SortedFeatures>
# Compare desired features (ResolvedResourceGraph) against current state.
# Returns an array of diff objects.
function _Planner-Diff {
    param(
        [Parameter(Mandatory=$true)] [string]   $DrgJson,
        [Parameter(Mandatory=$true)] [string[]] $SortedFeatures
    )

    $drg  = $DrgJson | ConvertFrom-Json
    $diff = @()

    # ── Desired features in sorted dependency order ──
    foreach ($feature in $SortedFeatures) {
        $inState = State-HasFeature -Feature $feature

        # Extract desired resources from RRG
        $drgFeature       = (_Prop $drg.features $feature)
        $desiredResources = @()
        if ($drgFeature -and (_Prop $drgFeature 'resources')) {
            $desiredResources = @($drgFeature.resources)
        }
        $desiredCount = $desiredResources.Count

        # Extract state resources
        $stateResources = @()
        if ($inState) {
            $stateResources = @(State-QueryResources -Feature $feature)
        }

        # Check for unknown resource kinds
        $hasBlocked    = $false
        $blockedReason = $null

        $unkDesired = @($desiredResources | Where-Object {
            (_Prop $_ 'kind') -notin $script:PlannerValidKinds
        })
        if ($unkDesired.Count -gt 0) {
            $kinds         = ($unkDesired | ForEach-Object { (_Prop $_ 'kind') } | Sort-Object -Unique) -join ", "
            $hasBlocked    = $true
            $blockedReason = "unknown resource kind: $kinds"
        }

        if ($inState -and (-not $hasBlocked)) {
            $unkState = @($stateResources | Where-Object {
                (_Prop $_ 'kind') -notin $script:PlannerValidKinds
            })
            if ($unkState.Count -gt 0) {
                $kinds         = ($unkState | ForEach-Object { (_Prop $_ 'kind') } | Sort-Object -Unique) -join ", "
                $hasBlocked    = $true
                $blockedReason = "unknown resource kind in state: $kinds"
            }
        }

        # Read version hint for this feature from profile (used for runtime version comparison)
        $profileVersion = ""
        if (-not [string]::IsNullOrWhiteSpace($script:PlannerProfileFile)) {
            $pv = _Planner-ProfileVersion -ProfileFile $script:PlannerProfileFile -CanonicalId $feature
            if ($pv) { $profileVersion = $pv }
        }

        $diff += [PSCustomObject]@{
            feature                = $feature
            in_desired             = $true
            in_state               = $inState
            desired_resource_count = $desiredCount
            desired_resources      = $desiredResources
            state_resources        = $stateResources
            has_blocked_resources  = $hasBlocked
            blocked_reason         = $blockedReason
            profile_version        = $profileVersion
        }
    }

    # ── Installed features not in desired (candidates for destroy) ──
    $installedFeatures = @(State-ListFeatures)
    foreach ($installedFeat in $installedFeatures) {
        if ($SortedFeatures -contains $installedFeat) { continue }

        $diff += [PSCustomObject]@{
            feature                = $installedFeat
            in_desired             = $false
            in_state               = $true
            desired_resource_count = 0
            desired_resources      = @()
            state_resources        = @()
            has_blocked_resources  = $false
            blocked_reason         = $null
        }
    }

    return $diff
}

# ── Phase 2: Classification ───────────────────────────────────────────────────

# _Planner-Classify <Diff>
# Apply the decision table to each diff entry.
# Returns an array of classified objects.
function _Planner-Classify {
    param([Parameter(Mandatory=$true)] [object[]] $Diff)

    $classified = @()

    foreach ($entry in $Diff) {
        if ($entry.has_blocked_resources) {
            $classified += [PSCustomObject]@{
                feature        = $entry.feature
                classification = "blocked"
                reason         = ((_Coal $entry.blocked_reason "unknown resource kind"))
            }

        } elseif ($entry.in_desired -and (-not $entry.in_state)) {
            $classified += [PSCustomObject]@{
                feature        = $entry.feature
                classification = "create"
            }

        } elseif ((-not $entry.in_desired) -and $entry.in_state) {
            $classified += [PSCustomObject]@{
                feature        = $entry.feature
                classification = "destroy"
            }

        } elseif ($entry.in_desired -and $entry.in_state) {

            if ($entry.desired_resource_count -eq 0) {
                # Script feature: classify by presence only
                $classified += [PSCustomObject]@{
                    feature        = $entry.feature
                    classification = "noop"
                }
            } else {
                # Build semantic key maps
                $dKeyed = @{}
                foreach ($r in @($entry.desired_resources)) {
                    $key = _Planner-DesiredSemanticKey -Res $r
                    $dKeyed[$key] = $r
                }
                $sKeyed = @{}
                foreach ($r in @($entry.state_resources)) {
                    $key = _Planner-StateSemanticKey -Res $r
                    $sKeyed[$key] = $r
                }

                $dKeys = @($dKeyed.Keys)
                $sKeys = @($sKeyed.Keys)

                # Set operations
                $common = @($dKeys | Where-Object { $sKeys -contains $_ })
                $dOnly  = @($dKeys | Where-Object { $sKeys -notcontains $_ })
                $sOnly  = @($sKeys | Where-Object { $dKeys -notcontains $_ })

                # Compatibility of common resources
                $hasInc = $false   # has incompatible (non-backend) change
                $hasBm  = $false   # has backend mismatch only

                foreach ($k in $common) {
                    $compat = _Planner-CheckResourceCompat -DesiredRes $dKeyed[$k] -StateRes $sKeyed[$k] -ProfileVersion ((_Coal (_Prop $entry 'profile_version') ""))
                    if     ($compat -eq "backend_mismatch") { $hasBm  = $true }
                    elseif ($compat -ne "compatible")       { $hasInc = $true }
                }

                if ($hasInc -or $sOnly.Count -gt 0) {
                    # Incompatible mutation or state has resources removed from desired
                    $classified += [PSCustomObject]@{
                        feature        = $entry.feature
                        classification = "replace"
                    }
                } elseif ($hasBm) {
                    $classified += [PSCustomObject]@{
                        feature        = $entry.feature
                        classification = "replace_backend"
                    }
                } elseif ($dOnly.Count -gt 0) {
                    # All state resources present in desired, all common compatible, desired has extras
                    $addResources = @($dOnly | ForEach-Object {
                        $r = $dKeyed[$_]
                        [PSCustomObject]@{
                            kind = (_Prop $r 'kind')
                            id   = (_Coal (_Prop $r 'id') (_Prop $r 'kind'))
                        }
                    })
                    $classified += [PSCustomObject]@{
                        feature        = $entry.feature
                        classification = "strengthen"
                        add_resources  = $addResources
                    }
                } else {
                    $classified += [PSCustomObject]@{
                        feature        = $entry.feature
                        classification = "noop"
                    }
                }
            }

        } else {
            # Unreachable, but the table must be total
            $classified += [PSCustomObject]@{
                feature        = $entry.feature
                classification = "noop"
            }
        }
    }

    return $classified
}

# ── Phase 3: Decision ─────────────────────────────────────────────────────────

# _Planner-Decide <Classified>
# Apply ordering rules and produce the final plan object.
# Ordering: destroy (reversed) → replace → replace_backend → strengthen → create
function _Planner-Decide {
    param([Parameter(Mandatory=$true)] [object[]] $Classified)

    $destroys        = @($Classified | Where-Object { $_.classification -eq "destroy"         })
    $replaces        = @($Classified | Where-Object { $_.classification -eq "replace"         })
    $replaceBackends = @($Classified | Where-Object { $_.classification -eq "replace_backend" })
    $strengthens     = @($Classified | Where-Object { $_.classification -eq "strengthen"      })
    $creates         = @($Classified | Where-Object { $_.classification -eq "create"          })
    $blocked         = @($Classified | Where-Object { $_.classification -eq "blocked"         })
    $noops           = @($Classified | Where-Object { $_.classification -eq "noop"            })

    # Reverse destroy order (uninstall in reverse dependency order)
    [array]::Reverse($destroys)

    $actions = @()

    foreach ($d  in $destroys)        { $actions += [PSCustomObject]@{ feature = $d.feature;  operation = "destroy";         details = [PSCustomObject]@{} } }
    foreach ($r  in $replaces)        { $actions += [PSCustomObject]@{ feature = $r.feature;  operation = "replace";         details = [PSCustomObject]@{} } }
    foreach ($rb in $replaceBackends) { $actions += [PSCustomObject]@{ feature = $rb.feature; operation = "replace_backend"; details = [PSCustomObject]@{} } }
    foreach ($s  in $strengthens) {
        $addRes = if ((_Prop $s 'add_resources')) { @($s.add_resources) } else { @() }
        $actions += [PSCustomObject]@{
            feature   = $s.feature
            operation = "strengthen"
            details   = [PSCustomObject]@{ add_resources = $addRes }
        }
    }
    foreach ($c  in $creates)         { $actions += [PSCustomObject]@{ feature = $c.feature;  operation = "create";          details = [PSCustomObject]@{} } }

    $blockedList = @($blocked | ForEach-Object {
        [PSCustomObject]@{ feature = $_.feature; reason = ((_Coal $_.reason "unknown resource kind")) }
    })
    $noopList = @($noops | ForEach-Object { [PSCustomObject]@{ feature = $_.feature } })

    return [PSCustomObject]@{
        actions = $actions
        blocked = $blockedList
        noops   = $noopList
        summary = [PSCustomObject]@{
            create          = $creates.Count
            destroy         = $destroys.Count
            replace         = $replaces.Count
            replace_backend = $replaceBackends.Count
            strengthen      = $strengthens.Count
            noop            = $noops.Count
            blocked         = $blocked.Count
        }
    }
}

# ── Public API ────────────────────────────────────────────────────────────────

# Invoke-PlannerRun <DrgJson> <SortedFeatures> <ProfileFile>
# Full planning pipeline: diff → classify → decide.
# Returns the plan as a JSON string, or throws on error.
#
# Reads state via State-HasFeature / State-ListFeatures / State-QueryResources (read-only).
function Invoke-PlannerRun {
    param(
        [Parameter(Mandatory=$true)]  [string]   $DrgJson,
        [Parameter(Mandatory=$true)]  [string[]] $SortedFeatures,
        [Parameter(Mandatory=$false)] [string]   $ProfileFile = ""
    )

    if ([string]::IsNullOrWhiteSpace($DrgJson)) {
        Log-Error "Invoke-PlannerRun: DrgJson is required"
        throw "Invoke-PlannerRun: DrgJson is required"
    }

    $script:PlannerProfileFile = $ProfileFile

    $diff       = _Planner-Diff     -DrgJson $DrgJson -SortedFeatures $SortedFeatures
    $classified = _Planner-Classify -Diff $diff
    $plan       = _Planner-Decide   -Classified $classified

    return ($plan | ConvertTo-Json -Depth 10 -Compress:$false)
}
