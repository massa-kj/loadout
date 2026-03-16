# -----------------------------------------------------------------------------
# Module: declarative_executor (PowerShell)
#
# Responsibility:
#   Execute plan actions for mode:declarative features.
#   Reads resources from feature.yaml spec.resources and installs/uninstalls
#   each resource through the appropriate backend or fs operation.
#
# Public API:
#   Invoke-DeclarativeExecutorRun <Feature> <Operation> <DetailsObject>
#
# Operations:
#   create          — install all spec.resources; record in state
#   destroy         — uninstall all state resources; remove feature from state
#   replace         — destroy (resources + state) then install all spec.resources
#   replace_backend — same as replace
#   strengthen      — install only resources listed in details.add_resources
#
# Design notes:
#   - Resources are read directly from feature.yaml .resources at execute time.
#   - desired_backend is re-resolved via Resolve-BackendFor (same as script executor).
#   - fs source paths use the explicit 'source' field (relative to feature_dir).
#     Fallback: files/<basename(path)> by convention.
#   - strengthen uses State-PatchBegin from current state so existing resources
#     are preserved; only the add_resources list is freshly installed.
#   - This module is dot-sourced by executor.ps1 and must NOT dot-source executor.ps1
#     (circular dependency). All _Executor-* helpers are provided by executor.ps1.
# -----------------------------------------------------------------------------

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# ── Spec resource reading ─────────────────────────────────────────────────────

# _DE-SpecResources <Feature>
# Read .resources from feature.yaml (and platform override if present).
# Platform-specific override replaces (not appends) base resources when non-empty.
# Returns PSCustomObject[].
function _DE-SpecResources {
    param([Parameter(Mandatory = $true)] [string]$Feature)

    $metaFile     = _Executor-ResolveFeature -Feature $Feature
    $platformMeta = _Executor-ResolvePlatformFeature -Feature $Feature

    # Platform override replaces base when it declares a non-empty resources list
    if (-not [string]::IsNullOrWhiteSpace($platformMeta) -and (Test-Path $platformMeta)) {
        $platJson = & yq eval -o=json '.resources // []' $platformMeta 2>$null
        if (-not [string]::IsNullOrWhiteSpace($platJson) -and
            $platJson -ne "null" -and $platJson -ne "[]") {
            return @($platJson | ConvertFrom-Json)
        }
    }

    $baseJson = & yq eval -o=json '.resources // []' $metaFile 2>$null
    if ([string]::IsNullOrWhiteSpace($baseJson)) { return @() }
    return @($baseJson | ConvertFrom-Json)
}

# ── Resource installation ─────────────────────────────────────────────────────

# _DE-InstallResource <Feature> <FeatureDir> <ResourceObj>
# Install a single resource and record it in the active state patch.
function _DE-InstallResource {
    param(
        [Parameter(Mandatory = $true)] [string]$Feature,
        [Parameter(Mandatory = $true)] [string]$FeatureDir,
        [Parameter(Mandatory = $true)] [object]$ResourceObj
    )

    $kind = $ResourceObj.kind

    switch ($kind) {
        "package" {
            $name    = $ResourceObj.name
            $backend = Resolve-BackendFor -Kind "package" -Name $name
            Load-Backend -BackendId $backend

            if (Backend-Call "PackageExists" $name 2>$null) {
                Log-Info "    package already installed: $name"
            } else {
                Log-Info "    installing package: $name"
                Backend-Call "InstallPackage" $name
                if ($LASTEXITCODE -ne 0) {
                    Log-Error "declarative_executor: failed to install package: $name"
                    return $false
                }
            }

            $rid = if ($ResourceObj.PSObject.Properties['id'] -and
                       -not [string]::IsNullOrWhiteSpace($ResourceObj.id)) {
                $ResourceObj.id
            } else { "pkg:$name" }

            $resource = [PSCustomObject]@{
                kind    = "package"
                id      = $rid
                backend = $backend
                package = [PSCustomObject]@{ name = $name; version = $null }
            }
            State-PatchAddResource -Feature $Feature -ResourceObject $resource
        }

        "runtime" {
            $name    = $ResourceObj.name
            $backend = Resolve-BackendFor -Kind "runtime" -Name $name
            Load-Backend -BackendId $backend

            $rtVersion = if ($ResourceObj.PSObject.Properties['version'] -and
                             -not [string]::IsNullOrWhiteSpace($ResourceObj.version)) {
                [string]$ResourceObj.version
            } else { "latest" }

            $actualVersion = $rtVersion
            if (Backend-Call "RuntimeExists" $name $rtVersion 2>$null) {
                Log-Info "    runtime already installed: ${name}@${rtVersion}"
            } else {
                Log-Info "    installing runtime: ${name}@${rtVersion}"
                $installed = Backend-Call "InstallRuntime" $name $rtVersion
                if ($LASTEXITCODE -ne 0) {
                    Log-Error "declarative_executor: failed to install runtime: ${name}@${rtVersion}"
                    return $false
                }
                if (-not [string]::IsNullOrWhiteSpace($installed)) {
                    $actualVersion = [string]$installed
                }
            }

            $rid = if ($ResourceObj.PSObject.Properties['id'] -and
                       -not [string]::IsNullOrWhiteSpace($ResourceObj.id)) {
                $ResourceObj.id
            } else { "rt:${name}@${actualVersion}" }

            $resource = [PSCustomObject]@{
                kind    = "runtime"
                id      = $rid
                backend = $backend
                runtime = [PSCustomObject]@{ name = $name; version = $actualVersion }
            }
            State-PatchAddResource -Feature $Feature -ResourceObject $resource
        }

        "fs" {
            $rawPath  = $ResourceObj.path
            $op       = if ($ResourceObj.PSObject.Properties['op'] -and
                            -not [string]::IsNullOrWhiteSpace($ResourceObj.op)) {
                $ResourceObj.op
            } else { "link" }
            $sourceRel = if ($ResourceObj.PSObject.Properties['source'] -and
                             -not [string]::IsNullOrWhiteSpace($ResourceObj.source)) {
                $ResourceObj.source
            } else { "" }

            if ([string]::IsNullOrWhiteSpace($rawPath)) {
                Log-Error "declarative_executor: fs resource missing 'path' field"
                return $false
            }

            # Expand ~ in target path
            $targetPath = _Executor-ExpandPath -Path $rawPath

            # Resolve source file:
            #   explicit: feature_dir/<source_rel>
            #   fallback: feature_dir/files/<basename(path)>
            $src = if (-not [string]::IsNullOrWhiteSpace($sourceRel)) {
                Join-Path $FeatureDir $sourceRel
            } else {
                $basename = [System.IO.Path]::GetFileName($targetPath)
                Join-Path (Join-Path $FeatureDir "files") $basename
            }

            if (-not (Test-Path $src)) {
                Log-Error "declarative_executor: fs source not found: $src"
                return $false
            }

            # Ensure parent directory
            $parent = [System.IO.Path]::GetDirectoryName($targetPath)
            if ($parent -and -not (Test-Path $parent)) {
                New-Item -ItemType Directory -Path $parent -Force | Out-Null
            }

            # Handle existing path: only remove if managed by loadout
            if (Test-Path $targetPath -ErrorAction SilentlyContinue) {
                if (State-HasFile -File $targetPath) {
                    Remove-Item -Path $targetPath -Recurse -Force
                } else {
                    Log-Error "declarative_executor: path exists and is not managed: $targetPath"
                    return $false
                }
            }

            $srcItem   = Get-Item $src
            $isDir     = $srcItem.PSIsContainer
            $deployed  = $false
            $entryType = "file"
            $actualOp  = "copy"

            if ($op -eq "link") {
                if (_Executor-TrySymlink -Src $src -Dst $targetPath) {
                    Log-Success "  Linked $targetPath"
                    $deployed  = $true
                    $entryType = if ($isDir) { "dir" } else { "symlink" }
                    $actualOp  = "link"
                } elseif ($isDir -and (_Executor-TryJunction -Src $src -Dst $targetPath)) {
                    Log-Success "  Junctioned $targetPath"
                    $deployed  = $true
                    $entryType = "dir"
                    $actualOp  = "link"
                }
            }

            if (-not $deployed) {
                Copy-Item -Path $src -Destination $targetPath -Recurse -Force -ErrorAction Stop
                $suffix    = if ($op -eq "link") { " (link fallback)" } else { "" }
                Log-Success "  Copied $targetPath$suffix"
                $entryType = if ($isDir) { "dir" } else { "file" }
                $actualOp  = "copy"
            }

            $rid = if ($ResourceObj.PSObject.Properties['id'] -and
                       -not [string]::IsNullOrWhiteSpace($ResourceObj.id)) {
                $ResourceObj.id
            } else {
                $basename = [System.IO.Path]::GetFileName($targetPath)
                "fs:$basename"
            }

            $resource = [PSCustomObject]@{
                kind = "fs"
                id   = $rid
                fs   = [PSCustomObject]@{
                    path       = $targetPath
                    entry_type = $entryType
                    op         = $actualOp
                }
            }
            State-PatchAddResource -Feature $Feature -ResourceObject $resource
        }

        default {
            Log-Error "declarative_executor: unsupported resource kind: $kind"
            return $false
        }
    }

    return $true
}

# _DE-InstallResources <Feature> <ResourceObjs>
# Install all resources in the given PSCustomObject[].
function _DE-InstallResources {
    param(
        [Parameter(Mandatory = $true)] [string]$Feature,
        [Parameter(Mandatory = $true)] [object[]]$ResourceObjs
    )

    $featureDir = _Executor-GetFeatureDir -Feature $Feature

    foreach ($res in $ResourceObjs) {
        $ok = _DE-InstallResource -Feature $Feature -FeatureDir $featureDir -ResourceObj $res
        if (-not $ok) { return $false }
    }
    return $true
}

# ── Public API ────────────────────────────────────────────────────────────────

# Invoke-DeclarativeExecutorRun <Feature> <Operation> <DetailsObject>
# Execute a plan action for a mode:declarative feature.
# DetailsObject is a PSCustomObject (may be $null for operations that don't use it).
function Invoke-DeclarativeExecutorRun {
    param(
        [Parameter(Mandatory = $true)] [string]$Feature,
        [Parameter(Mandatory = $true)] [string]$Operation,
        [object]$Details = $null
    )

    Log-Info "[$Operation] $Feature (declarative)"

    switch ($Operation) {
        "create" {
            $specResources = _DE-SpecResources -Feature $Feature
            State-PatchBegin
            if (-not (_DE-InstallResources -Feature $Feature -ResourceObjs $specResources)) {
                return $false
            }
            State-PatchFinalize | Out-Null
        }

        "destroy" {
            if (-not (_Executor-RemoveResources -Feature $Feature)) { return $false }
            State-PatchBegin
            State-PatchRemoveFeature -Feature $Feature
            State-PatchFinalize | Out-Null
        }

        { $_ -in @("replace", "replace_backend") } {
            # Destroy phase: remove currently installed resources + clear state entry
            if (-not (_Executor-RemoveResources -Feature $Feature)) { return $false }
            State-PatchBegin
            State-PatchRemoveFeature -Feature $Feature
            State-PatchFinalize | Out-Null
            # Install phase: install all spec resources from scratch
            $specResources = _DE-SpecResources -Feature $Feature
            State-PatchBegin
            if (-not (_DE-InstallResources -Feature $Feature -ResourceObjs $specResources)) {
                return $false
            }
            State-PatchFinalize | Out-Null
        }

        "strengthen" {
            # Install only the resources in details.add_resources.
            # State-PatchBegin starts from the current state, so existing
            # resources in the feature are preserved automatically.
            $addIds = @()
            if ($null -ne $Details -and
                $Details.PSObject.Properties['add_resources'] -and
                $null -ne $Details.add_resources) {
                $addIds = @($Details.add_resources | ForEach-Object { $_.id })
            }

            $specResources = _DE-SpecResources -Feature $Feature

            # Filter spec resources to only those listed in add_ids
            $addResources = @($specResources | Where-Object {
                $_.PSObject.Properties['id'] -and ($addIds -contains $_.id)
            })

            if ($addResources.Count -eq 0) {
                Log-Warn "declarative_executor: strengthen: no matching add_resources found (noop)"
                return $true
            }

            State-PatchBegin
            if (-not (_DE-InstallResources -Feature $Feature -ResourceObjs $addResources)) {
                return $false
            }
            State-PatchFinalize | Out-Null
        }

        default {
            Log-Error "Invoke-DeclarativeExecutorRun: unsupported operation: $Operation"
            return $false
        }
    }

    return $true
}
