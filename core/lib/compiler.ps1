# -----------------------------------------------------------------------------
# Module: compiler (FeatureCompiler, PowerShell)
#
# Responsibility:
#   Compile the raw DesiredResourceGraph from Feature Index + resolved feature
#   order. Assigns stable resource IDs; does NOT resolve backends or read
#   profile/policy. That is PolicyResolver's job.
#
# Public API (Stable):
#   Invoke-FeatureCompilerRun <FeatureIndexJson> <SortedFeatures>
#   Returns raw DesiredResourceGraph JSON string, or $null on error.
#
# Contract:
#   - mode:script     → entry with empty resources array
#   - mode:declarative → resources expanded with stable IDs (no desired_backend)
#   - Declarative validation: error if no resources, error if install.ps1 present
#   - PolicyResolver is responsible for adding desired_backend to each resource
#
# JSON schema: see docs/specs/data/desired_resource_graph.md
# -----------------------------------------------------------------------------

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# This library expects env.ps1, logger.ps1, and source_registry.ps1 to be
# loaded by the caller. backend_registry is NOT needed by this module.

# _Compiler-ResourceId <Kind> <ResourceObj>
# Derive a stable resource id from a resource object.
function _Compiler-ResourceId {
    param(
        [Parameter(Mandatory=$true)] [string]$Kind,
        [Parameter(Mandatory=$true)] [object]$ResourceObj
    )

    # Use explicit id if present
    $existingId = if ($ResourceObj.PSObject.Properties['id']) {
        $ResourceObj.PSObject.Properties['id'].Value
    }
    if ($existingId -and $existingId -ne "null") { return $existingId }

    switch ($Kind) {
        "package" {
            $name = if ($ResourceObj.PSObject.Properties['name']) {
                $ResourceObj.PSObject.Properties['name'].Value
            }
            return "package:$name"
        }
        "runtime" {
            $name = if ($ResourceObj.PSObject.Properties['name']) {
                $ResourceObj.PSObject.Properties['name'].Value
            }
            return "runtime:$name"
        }
        "fs" {
            $target = if ($ResourceObj.PSObject.Properties['target']) {
                $ResourceObj.PSObject.Properties['target'].Value
            }
            $path   = if ($ResourceObj.PSObject.Properties['path']) {
                $ResourceObj.PSObject.Properties['path'].Value
            }
            $p      = if ($target) { $target } elseif ($path) { $path } else { "unknown" }
            return "fs:$([System.IO.Path]::GetFileName($p))"
        }
        default {
            return "${Kind}:unknown"
        }
    }
}

# _Compiler-ResolveResource <CanonicalId> <ResourceObj>
# Add a stable id to a resource object (as new PSCustomObject). Does not resolve backends.
# PolicyResolver will add desired_backend in a subsequent step.
function _Compiler-ResolveResource {
    param(
        [Parameter(Mandatory=$true)] [string]$CanonicalId,
        [Parameter(Mandatory=$true)] [object]$ResourceObj
    )

    $kind = if ($ResourceObj.PSObject.Properties['kind']) {
        $ResourceObj.PSObject.Properties['kind'].Value
    }
    if (-not $kind -or $kind -eq "null") {
        throw "resource missing 'kind' in $CanonicalId"
    }

    $resourceId = _Compiler-ResourceId -Kind $kind -ResourceObj $ResourceObj

    # Build a copy of the resource with id added. All other fields pass through unchanged.
    $updated = [ordered]@{}
    foreach ($prop in $ResourceObj.PSObject.Properties) {
        $updated[$prop.Name] = $prop.Value
    }
    $updated['id'] = $resourceId

    switch ($kind) {
        { $_ -in @("package", "runtime", "fs") } {
            # Pass through; PolicyResolver will add desired_backend for package/runtime
        }
        default {
            throw "unknown resource kind '$kind' in $CanonicalId"
        }
    }

    return $updated
}

# Invoke-FeatureCompilerRun <FeatureIndexJson> <SortedFeatures>
# Compile raw DesiredResourceGraph from Feature Index and resolved feature order.
# Returns DesiredResourceGraph JSON string, or $null on error.
#
# For mode:script features: entry with empty resources array.
# For mode:declarative features: resources expanded with stable IDs.
# Call Invoke-PolicyResolverRun on the output to add desired_backend per resource.
function Invoke-FeatureCompilerRun {
    param(
        [Parameter(Mandatory=$true)]  [string]   $FeatureIndexJson,
        [Parameter(Mandatory=$true)]  [string[]] $SortedFeatures
    )

    $index    = $FeatureIndexJson | ConvertFrom-Json
    $features = [ordered]@{}

    foreach ($canonicalId in $SortedFeatures) {
        $prop = $index.features.PSObject.Properties[$canonicalId]
        if ($null -eq $prop) {
            Log-Error "Invoke-FeatureCompilerRun: feature not found in index: $canonicalId"
            return $null
        }
        $entry = $prop.Value
        $mode  = $entry.mode

        $resources = @()

        if ($mode -eq "declarative") {
            # ── Declarative validation ─────────────────────────────────────
            $sourceDir = $entry.source_dir

            # Must not have install.ps1 or uninstall.ps1
            $hasInstall   = Test-Path (Join-Path $sourceDir "install.ps1")
            $hasUninstall = Test-Path (Join-Path $sourceDir "uninstall.ps1")
            if ($hasInstall -or $hasUninstall) {
                Log-Error "Invoke-FeatureCompilerRun: declarative feature must not have install.ps1/uninstall.ps1: $canonicalId"
                return $null
            }

            # spec must exist with at least one resource
            # Use ForEach-Object to enumerate: PS5.1 ConvertFrom-Json returns ArrayList for JSON
            # arrays, and @(ArrayList) wraps the list object itself rather than unwrapping items.
            # Pipeline enumeration handles both Object[] and ArrayList correctly.
            $specResources = @()
            if ($entry.spec -and $entry.spec.resources) {
                $specResources = @($entry.spec.resources | ForEach-Object { $_ })
            }
            if ($specResources.Count -eq 0) {
                Log-Error "Invoke-FeatureCompilerRun: declarative feature has no resources defined: $canonicalId"
                return $null
            }

            # ── Expand resources with stable IDs ──────────────────────────────────
            foreach ($res in $specResources) {
                try {
                    $resolved  = _Compiler-ResolveResource -CanonicalId $canonicalId -ResourceObj $res
                    $resources = @($resources) + $resolved
                } catch {
                    Log-Error "Invoke-FeatureCompilerRun: failed to resolve resource in ${canonicalId}: $_"
                    return $null
                }
            }
        }

        $features[$canonicalId] = [ordered]@{ resources = $resources }
    }

    $drg = [ordered]@{
        schema_version = 1
        features       = $features
    }

    return $drg | ConvertTo-Json -Depth 20 -Compress
}
