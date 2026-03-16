# -----------------------------------------------------------------------------
# Module: state
#
# Responsibility:
#   Manage state file (v3) with atomic writes, migration, and patch operations.
#
# Stable Public API:
#   State-Load
#   State-Validate [Mode] [Json]         Mode = "load" | "execute"
#   State-CommitAtomic <StateObject>
#   State-QueryFeature <Feature>
#   State-QueryResources <Feature>
#   State-PatchBegin
#   State-PatchAddResource <Feature> <ResourceObject>
#   State-PatchRemoveFeature <Feature>
#   State-PatchFinalize
#   State-MigrateV2ToV3                              — called by cmd/migrate.ps1
#
# Compat API:
#   State-Init                                        — keep (used by scripts)
#   State-HasFeature <Feature>                        — keep
#   State-ListFeatures                                — keep
#   State-GetFiles <Feature>                          — keep
#   State-HasFile <path>                              — keep
#   State-AddFile <Feature> <File>                    — keep (git gitconfig complex merge)
#   State-GetRuntime <Feature> <Key>                  — keep (read-only)
# -----------------------------------------------------------------------------

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# ── Private state ─────────────────────────────────────────────────────────────

# In-memory cache of the authoritative state (PSCustomObject).
$script:StateData = $null

# Working copy for patch operations (deep clone).
$script:StatePatchData = $null

# ── Internal helpers ──────────────────────────────────────────────────────────

function _State-EnsureLoaded {
    if ($null -eq $script:StateData) {
        State-Load | Out-Null
    }
}

# Deep-clone a PSCustomObject via JSON round-trip.
function _State-DeepClone {
    param([Parameter(Mandatory=$true)] $Obj)
    return $Obj | ConvertTo-Json -Depth 20 | ConvertFrom-Json
}

# Convert PSCustomObject to canonical JSON string.
function _State-ToJson {
    param([Parameter(Mandatory=$true)] $Obj)
    return $Obj | ConvertTo-Json -Depth 20
}

# Resolve authoritative state file path from env module.
function _State-GetFilePath {
    if (-not (Get-Command Get-DotfilesStateFilePath -ErrorAction SilentlyContinue)) {
        Log-Error "_State-GetFilePath: Get-DotfilesStateFilePath is not available"
        return $null
    }
    return Get-DotfilesStateFilePath
}

    function _State-NormalizeFeatureId {
        param([Parameter(Mandatory=$true)] [string]$Feature)

        if ([string]::IsNullOrWhiteSpace($Feature)) {
            throw "_State-NormalizeFeatureId: feature name is required"
        }

        if (Get-Command Canonical-Id-Normalize -ErrorAction SilentlyContinue) {
            return (Canonical-Id-Normalize -Name $Feature -DefaultSourceId "core")
        }

        if ($Feature -match '/') {
            return $Feature
        }

        return "core/$Feature"
    }

# ── Stable Core API ───────────────────────────────────────────────────────────

# State-Load
# Load state from disk into in-memory cache.
# Creates empty v3 state if the file does not exist.
function State-Load {
    $path = _State-GetFilePath
    if (-not $path) { return $false }
    $dir  = Split-Path -Parent $path

    if (-not (Test-Path $dir)) {
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
    }

    if (-not (Test-Path $path)) {
        $script:StateData = [PSCustomObject]@{ version = 3; features = [PSCustomObject]@{} }
        _State-ToJson $script:StateData | Set-Content -Path $path -Encoding UTF8
        return $true
    }

    # Validate JSON parsability
    try {
        $script:StateData = Get-Content -Path $path -Raw | ConvertFrom-Json
    } catch {
        Log-Error "State-Load: state file is not valid JSON: $path"
        return $false
    }

    $ver = $script:StateData.version

    if ($ver -eq 3) {
        return $true
    } elseif ($ver -eq 1 -or $ver -eq 2) {
        Log-Error "State-Load: state is at version ${ver}, which requires migration."
        Log-Error "  Run: loadout migrate"
        return $false
    } else {
        Log-Error "State-Load: unknown state version: $ver"
        return $false
    }
}

# State-Validate [Mode] [StateObject]
# Validate structural invariants.
#   Mode=load    - allow unknown resource kinds; check structural sanity.
#   Mode=execute - additionally abort on features containing unknown kinds.
function State-Validate {
    param(
        [string]$Mode = "load",
        $StateObject = $null
    )

    $obj = if ($null -ne $StateObject) { $StateObject } else { $script:StateData }

    if ($null -eq $obj) {
        Log-Error "State-Validate: no state loaded"
        return $false
    }

    # 1. version MUST be 3
    if ($obj.version -ne 3) {
        Log-Error "State-Validate: version MUST be 3, got: $($obj.version)"
        return $false
    }

    # 2. features MUST exist and be an object
    if ($null -eq $obj.features) {
        Log-Error "State-Validate: .features must exist"
        return $false
    }

    $knownKinds = @("package", "runtime", "fs")
    $allFsPaths = [System.Collections.Generic.List[string]]::new()

    foreach ($prop in $obj.features.PSObject.Properties) {
        $featureId = $prop.Name
        $featureVal = $prop.Value

        # 3. Each feature entry MUST have resources array
        if ($null -eq $featureVal.resources) {
            Log-Error "State-Validate: feature ${featureId}: resources must be an array"
            return $false
        }

        $seenIds = [System.Collections.Generic.HashSet[string]]::new()

        foreach ($res in $featureVal.resources) {
            # 4. Each resource MUST have kind and id
            if ([string]::IsNullOrEmpty($res.kind) -or [string]::IsNullOrEmpty($res.id)) {
                Log-Error "State-Validate: feature ${featureId}: resource missing kind or id"
                return $false
            }

            # 5. Within a feature: no duplicate resource.id
            if (-not $seenIds.Add($res.id)) {
                Log-Error "State-Validate: feature ${featureId}: duplicate resource id: $($res.id)"
                return $false
            }

            # Collect fs paths for cross-feature duplicate check
            if ($res.kind -eq "fs") {
                # 7. fs.path MUST be absolute
                if (-not ($res.fs.path -match '^(/|[A-Za-z]:\\)')) {
                    Log-Error "State-Validate: fs.path not absolute: $($res.fs.path)"
                    return $false
                }
                $allFsPaths.Add($res.fs.path)
            }

            # mode=execute: reject unknown kinds
            if ($Mode -eq "execute" -and ($knownKinds -notcontains $res.kind)) {
                Log-Error "State-Validate(execute): feature ${featureId}: unknown kind: $($res.kind)"
                return $false
            }
        }
    }

    # 6. Across all features: no duplicate fs.path
    $dupPaths = $allFsPaths | Group-Object | Where-Object { $_.Count -gt 1 }
    if ($dupPaths) {
        foreach ($dup in $dupPaths) {
            Log-Error "State-Validate: duplicate fs.path across features: $($dup.Name)"
        }
        return $false
    }

    return $true
}

# State-CommitAtomic <StateObject>
# Write a new state atomically: write to .tmp → validate → atomic rename.
function State-CommitAtomic {
    param(
        [Parameter(Mandatory=$true)]
        $StateObject
    )

    $path = _State-GetFilePath
    if (-not $path) { return $false }
    $tmp  = "$path.tmp"

    try {
        _State-ToJson $StateObject | Set-Content -Path $tmp -Encoding UTF8
    } catch {
        Log-Error "State-CommitAtomic: failed to write tmp file: $_"
        Remove-Item -Path $tmp -ErrorAction SilentlyContinue
        return $false
    }

    # Validate before committing
    $tmpObj = Get-Content -Path $tmp -Raw | ConvertFrom-Json
    if (-not (State-Validate -Mode "load" -StateObject $tmpObj)) {
        Log-Error "State-CommitAtomic: validation failed, aborting commit"
        Remove-Item -Path $tmp -ErrorAction SilentlyContinue
        return $false
    }

    # Atomic rename (Move-Item -Force replaces destination)
    try {
        Move-Item -Path $tmp -Destination $path -Force
    } catch {
        Log-Error "State-CommitAtomic: atomic rename failed: $_"
        Remove-Item -Path $tmp -ErrorAction SilentlyContinue
        return $false
    }

    # Update in-memory cache
    $script:StateData = $tmpObj
    return $true
}

# State-QueryFeature <Feature>
# Return the feature entry PSCustomObject, or $null if not found.
function State-QueryFeature {
    param([Parameter(Mandatory=$true)] [string]$Feature)
    _State-EnsureLoaded
        $Feature = _State-NormalizeFeatureId -Feature $Feature
    $prop = $script:StateData.features.PSObject.Properties[$Feature]
    if ($null -ne $prop) { return $prop.Value }
    return $null
}

# State-QueryResources <Feature>
# Return the resources array for a feature, or empty array if not found.
function State-QueryResources {
    param([Parameter(Mandatory=$true)] [string]$Feature)
    _State-EnsureLoaded
        $Feature = _State-NormalizeFeatureId -Feature $Feature
    $feat = State-QueryFeature -Feature $Feature
    if ($null -eq $feat) { return @() }
    return @($feat.resources)
}

# ── Patch Operations ──────────────────────────────────────────────────────────

# State-PatchBegin
# Initialize a patch working copy from the current state cache.
function State-PatchBegin {
    _State-EnsureLoaded
    $script:StatePatchData = _State-DeepClone $script:StateData
}

# State-PatchAddResource <Feature> <ResourceObject>
# Add (or replace by id) a resource in the patch working copy.
# Creates the feature entry if it does not exist.
function State-PatchAddResource {
    param(
        [Parameter(Mandatory=$true)] [string]$Feature,
        [Parameter(Mandatory=$true)] $ResourceObject
    )

        $Feature = _State-NormalizeFeatureId -Feature $Feature

    if ($null -eq $script:StatePatchData) {
        Log-Error "State-PatchAddResource: no patch in progress; call State-PatchBegin first"
        return
    }

    # Create feature entry if missing
    if ($null -eq $script:StatePatchData.features.PSObject.Properties[$Feature]) {
        $script:StatePatchData.features | Add-Member -MemberType NoteProperty `
            -Name $Feature -Value ([PSCustomObject]@{ resources = @() })
    }

    $feat = $script:StatePatchData.features.$Feature
    # Replace existing resource with same id, or append
    $newResources = @($feat.resources | Where-Object { $_.id -ne $ResourceObject.id }) + @($ResourceObject)
    $feat.resources = $newResources
}

# State-PatchRemoveFeature <Feature>
# Remove a feature entry from the patch working copy.
function State-PatchRemoveFeature {
    param([Parameter(Mandatory=$true)] [string]$Feature)

        $Feature = _State-NormalizeFeatureId -Feature $Feature

    if ($null -eq $script:StatePatchData) {
        Log-Error "State-PatchRemoveFeature: no patch in progress; call State-PatchBegin first"
        return
    }

    $script:StatePatchData.features.PSObject.Properties.Remove($Feature)
}

# State-PatchFinalize
# Commit the patch working copy atomically and clear the buffer.
function State-PatchFinalize {
    if ($null -eq $script:StatePatchData) {
        Log-Error "State-PatchFinalize: no patch in progress"
        return $false
    }

    $result = State-CommitAtomic -StateObject $script:StatePatchData
    $script:StatePatchData = $null
    return $result
}

# ── Migration ─────────────────────────────────────────────────────────────────

# _Invoke-TransformV2ToV3 <V2Object>
# Pure transformation: convert v2 PSCustomObject (bare feature keys) to v3
# (canonical IDs). All bare names are prefixed with "core/". Already-canonical
# keys (containing "/") are unchanged.
# Returns the v3 PSCustomObject.
function _Invoke-TransformV2ToV3 {
    param([Parameter(Mandatory=$true)] $V2Object)

    $v3Features = [PSCustomObject]@{}

    foreach ($prop in $V2Object.features.PSObject.Properties) {
        $key   = $prop.Name
        $value = $prop.Value
        $newKey = if ($key -match '/') { $key } else { "core/$key" }
        $v3Features | Add-Member -MemberType NoteProperty -Name $newKey -Value $value
    }

    return [PSCustomObject]@{ version = 3; features = $v3Features }
}

# State-MigrateV2ToV3
# Migrate v2 state (bare feature keys) to v3 (canonical IDs).
# Must be called after State-Load with v2 content in $script:StateData.
# Performs: timestamped backup → transform → validate → atomic commit.
# Called exclusively by cmd/migrate.ps1; NOT called automatically by State-Load.
function State-MigrateV2ToV3 {
    $path      = _State-GetFilePath
    if (-not $path) { return $false }
    $timestamp = Get-Date -Format 'yyyyMMdd_HHmmss'
    $backup    = "$path.bak.$timestamp"

    if (Test-Path $path) {
        try {
            Copy-Item -Path $path -Destination $backup -Force
            Log-Info "State-MigrateV2ToV3: backup created: $backup"
        } catch {
            Log-Error "State-MigrateV2ToV3: failed to create backup: $_"
            return $false
        }
    }

    $v3 = _Invoke-TransformV2ToV3 -V2Object $script:StateData
    if ($null -eq $v3) {
        Log-Error "State-MigrateV2ToV3: transformation failed; restore from: $backup"
        return $false
    }

    if (-not (State-CommitAtomic -StateObject $v3)) {
        Log-Error "State-MigrateV2ToV3: commit failed; restore from: $backup"
        return $false
    }

    Log-Info "State-MigrateV2ToV3: migration to v3 complete"
    return $true
}

# State-Migrate
# Migrate the in-memory v1 state to v2: backup → transform → commit atomically.
# Called by cmd/migrate.ps1 for v1 state, before chaining into State-MigrateV2ToV3.
function State-Migrate {
    $path   = _State-GetFilePath
    if (-not $path) { return $false }
    $backup = "$path.bak"

    if (Test-Path $path) {
        try {
            Copy-Item -Path $path -Destination $backup -Force
            Log-Info "State-Migrate: backup created: $backup"
        } catch {
            Log-Error "State-Migrate: failed to create backup: $_"
            return $false
        }
    }

    $v2 = _Invoke-MigrateV1ToV2 -V1Object $script:StateData
    if ($null -eq $v2) {
        Log-Error "State-Migrate: transformation failed; restore from: $backup"
        return $false
    }

    if (-not (State-CommitAtomic -StateObject $v2)) {
        Log-Error "State-Migrate: commit failed; restore from: $backup"
        return $false
    }

    Log-Info "State-Migrate: migration to v2 complete"
    return $true
}

# _Invoke-MigrateV1ToV2 <V1Object>
# Pure transformation: convert v1 PSCustomObject to v2 format.
# Returns the v2 PSCustomObject.
#
# v1 → v2 mapping:
#   packages[]         → kind:package resources
#   files[]            → kind:fs resources  (entry_type/op inferred from filesystem)
#   runtime.version    → kind:runtime resource (runtime name = feature_id)
#   "{fid}@{rv}"       → skipped when runtime.version matches
function _Invoke-MigrateV1ToV2 {
    param([Parameter(Mandatory=$true)] $V1Object)

    $v2Features = [PSCustomObject]@{}

    foreach ($prop in $V1Object.features.PSObject.Properties) {
        $fid  = $prop.Name
        $feat = $prop.Value

        $rv       = if ($feat.PSObject.Properties['runtime'] -and $feat.runtime.PSObject.Properties['version']) { $feat.runtime.version } else { $null }
        $packages = if ($feat.PSObject.Properties['packages']) { @($feat.packages) } else { @() }
        $files    = if ($feat.PSObject.Properties['files'])    { @($feat.files)    } else { @() }

        $resources = [System.Collections.Generic.List[PSCustomObject]]::new()

        # ── package resources ──────────────────────────────────────────────
        foreach ($pkg in $packages) {
            if ($null -ne $rv -and $pkg -eq "${fid}@${rv}") { continue }  # captured by runtime resource

            $resources.Add([PSCustomObject]@{
                kind    = "package"
                id      = "pkg:$pkg"
                backend = "unknown"
                package = [PSCustomObject]@{ name = $pkg; version = $null }
            })
        }

        # ── fs resources ──────────────────────────────────────────────────
        foreach ($filePath in $files) {
            $entryType = "file"
            $op        = "copy"

            if (Test-Path $filePath -PathType Any) {
                $item = Get-Item -Path $filePath -Force -ErrorAction SilentlyContinue
                if ($null -ne $item) {
                    # RegistryKey objects have no .Attributes; only check on filesystem items.
                    if (($item -is [System.IO.FileSystemInfo]) -and
                        ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint)) {
                        $entryType = "symlink"
                        $op        = "link"
                    } elseif ($item.PSIsContainer) {
                        $entryType = "dir"
                        $op        = "copy"
                    } else {
                        $entryType = "file"
                        $op        = "copy"
                    }
                }
            }

            $resources.Add([PSCustomObject]@{
                kind = "fs"
                id   = "fs:$filePath"
                fs   = [PSCustomObject]@{
                    path       = $filePath
                    entry_type = $entryType
                    op         = $op
                }
            })
        }

        # ── runtime resource ──────────────────────────────────────────────
        if ($null -ne $rv) {
            $resources.Add([PSCustomObject]@{
                kind    = "runtime"
                id      = "rt:${fid}@${rv}"
                backend = "unknown"
                runtime = [PSCustomObject]@{ name = $fid; version = $rv }
            })
        }

        $v2Features | Add-Member -MemberType NoteProperty -Name $fid -Value (
            [PSCustomObject]@{ resources = $resources.ToArray() }
        )
    }

    return [PSCustomObject]@{ version = 2; features = $v2Features }
}

# ── Compat API ────────────────────────────────────────────────────────────────
# These functions provide backwards compatibility for feature scripts written
# against the v1 API. Will be removed when Phase 4 rewrites feature scripts.

# State-Init
# Initialize or load state. Calls State-Load (which auto-migrates v1 if needed).
function State-Init {
    return State-Load
}

# State-FeatureKeyFor — REMOVED in Phase 3 (state v3 uses canonical IDs directly).

# State-HasFeature <Feature>
function State-HasFeature {
    param([Parameter(Mandatory=$true)] [string]$Feature)
    _State-EnsureLoaded
        $Feature = _State-NormalizeFeatureId -Feature $Feature
    return $null -ne $script:StateData.features.PSObject.Properties[$Feature]
}

# State-ListFeatures
function State-ListFeatures {
    _State-EnsureLoaded
    return @($script:StateData.features.PSObject.Properties | ForEach-Object { $_.Name })
}

# State-GetFiles <Feature>
# Return file paths (strings) for the feature.
function State-GetFiles {
    param([Parameter(Mandatory=$true)] [string]$Feature)
        $Feature = _State-NormalizeFeatureId -Feature $Feature
    $resources = State-QueryResources -Feature $Feature
    return @($resources | Where-Object { $_.kind -eq "fs" } | ForEach-Object { $_.fs.path })
}

# State-HasFile <File>
function State-HasFile {
    param([Parameter(Mandatory=$true)] [string]$File)
    _State-EnsureLoaded
    foreach ($prop in $script:StateData.features.PSObject.Properties) {
        foreach ($res in @($prop.Value.resources)) {
            if ($res.kind -eq "fs" -and $res.fs.path -eq $File) { return $true }
        }
    }
    return $false
}

# State-AddFile <Feature> <File>
# Register an fs resource and commit atomically.
# entry_type and op are inferred from the filesystem at call time.
function State-AddFile {
    param(
        [Parameter(Mandatory=$true)] [string]$Feature,
        [Parameter(Mandatory=$true)] [string]$File
    )
    _State-EnsureLoaded
        $Feature = _State-NormalizeFeatureId -Feature $Feature

    # Infer entry_type and op from actual filesystem state
    $entryType = "file"
    $op        = "copy"

    if (Test-Path $File -PathType Any) {
        $item = Get-Item -Path $File -Force -ErrorAction SilentlyContinue
        if ($null -ne $item) {
            # RegistryKey objects have no .Attributes; only check on filesystem items.
            if (($item -is [System.IO.FileSystemInfo]) -and
                ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint)) {
                $entryType = "symlink"
                $op        = "link"
            } elseif ($item.PSIsContainer) {
                $entryType = "dir"
                $op        = "copy"
            }
        }
    }

    $resource = [PSCustomObject]@{
        kind = "fs"
        id   = "fs:$File"
        fs   = [PSCustomObject]@{
            path       = $File
            entry_type = $entryType
            op         = $op
        }
    }

    $newState = _State-DeepClone $script:StateData

    if ($null -eq $newState.features.PSObject.Properties[$Feature]) {
        $newState.features | Add-Member -MemberType NoteProperty -Name $Feature `
            -Value ([PSCustomObject]@{ resources = @() })
    }

    $feat = $newState.features.$Feature
    $newResources = @($feat.resources | Where-Object { $_.id -ne $resource.id }) + @($resource)
    $feat.resources = $newResources

    return State-CommitAtomic -StateObject $newState
}

# State-GetRuntime <Feature> <Key>
# Return the value for <Key> from the runtime resource of a feature.
# Only Key="version" is supported.
function State-GetRuntime {
    param(
        [Parameter(Mandatory=$true)] [string]$Feature,
        [Parameter(Mandatory=$true)] [string]$Key
    )
    _State-EnsureLoaded
        $Feature = _State-NormalizeFeatureId -Feature $Feature

    if ($Key -ne "version") { return $null }

    $resources = State-QueryResources -Feature $Feature
    $rt = $resources | Where-Object { $_.kind -eq "runtime" } | Select-Object -First 1
    if ($null -ne $rt) { return $rt.runtime.version }
    return $null
}
