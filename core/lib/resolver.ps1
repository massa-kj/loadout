# -----------------------------------------------------------------------------
# Module: resolver
#
# Responsibility:
#   Resolve feature dependencies and perform topological sorting.
#   Supports capability-based dependencies via requires/provides fields.
#
# Public API (Stable):
#   Resolve-Dependencies <DesiredFeatures>
#   Read-FeatureMetadata <FeatureIndexJson> <Features>
#   Invoke-TopoSortDFS <Feature> <DesiredFeatures>
#
# Input/output format:
#   All feature identifiers are canonical IDs of the form "<source_id>/<name>".
#   Dependency data is read exclusively from the Feature Index (feature_index.ps1).
#   Resolver does NOT read feature.yaml or any other file directly.
# -----------------------------------------------------------------------------

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# This library expects env.ps1 and logger.ps1 to be loaded by the caller.

# Lazily source source_registry if not already loaded
if (-not (Get-Command Canonical-Id-Normalize -ErrorAction SilentlyContinue)) {
    . "$env:LOADOUT_ROOT\core\lib\source_registry.ps1"
}

# Global variables for dependency graph
$script:FeatureDeps = @{}
$script:Visited = @{}
$script:InStack = @{}
$script:Sorted = @()

# Capability maps
$script:Provides = @{}   # capability -> [features that provide it]
$script:Requires = @{}   # feature    -> [capabilities required]

# Read-FeatureMetadata <FeatureIndexJson> <Features>
# Populate resolver globals from the Feature Index JSON.
# Reads dep fields exclusively from the Feature Index — no file I/O.
#
# Populates:
#   FeatureDeps – canonical depends per feature
#   Provides    – capability -> canonical features that provide it
#   Requires    – canonical feature -> required capabilities
function Read-FeatureMetadata {
    param(
        [Parameter(Mandatory=$true)] [string]$FeatureIndexJson,
        [Parameter(Mandatory=$true)] [string[]]$Features
    )

    $script:FeatureDeps = @{}
    $script:Provides    = @{}
    $script:Requires    = @{}

    $index = $FeatureIndexJson | ConvertFrom-Json

    Log-Info "Reading feature metadata..."

    foreach ($feature in $Features) {
        $prop = $index.features.PSObject.Properties[$feature]
        if ($null -eq $prop) {
            Log-Error "Read-FeatureMetadata: feature not found in index: $feature"
            return $false
        }
        $entry = $prop.Value

        try {
            # ── depends ───────────────────────────────────────────────────────
            $depArr = @()
            if ($entry.dep.depends) {
                $depArr = @($entry.dep.depends | Where-Object { $_ -and $_ -ne 'null' })
            }
            $script:FeatureDeps[$feature] = $depArr

            # ── provides ──────────────────────────────────────────────────────
            $provCaps = @()
            if ($entry.dep.provides) {
                $provCaps = @($entry.dep.provides | ForEach-Object { $_.name } |
                             Where-Object { $_ -and $_ -ne 'null' })
            }
            foreach ($cap in $provCaps) {
                if (-not $script:Provides.ContainsKey($cap)) {
                    $script:Provides[$cap] = @()
                }
                $script:Provides[$cap] += $feature
            }

            # ── requires ──────────────────────────────────────────────────────
            $reqArr = @()
            if ($entry.dep.requires) {
                $reqArr = @($entry.dep.requires | ForEach-Object { $_.name } |
                            Where-Object { $_ -and $_ -ne 'null' })
            }
            $script:Requires[$feature] = $reqArr

            # ── log ───────────────────────────────────────────────────────────
            if ($depArr.Count -gt 0) {
                Log-Info "  $feature depends on: $($depArr -join ', ')"
            }
            if ($provCaps.Count -gt 0) {
                Log-Info "  $feature provides: $($provCaps -join ', ')"
            }
            if ($reqArr.Count -gt 0) {
                Log-Info "  $feature requires capabilities: $($reqArr -join ', ')"
            }
            if ($depArr.Count -eq 0 -and $provCaps.Count -eq 0 -and $reqArr.Count -eq 0) {
                Log-Info "  $feature has no dependencies"
            }
        } catch {
            Log-Error "Failed to read metadata for ${feature}: $_"
            return $false
        }
    }

    return $true
}

# Invoke-InjectCapabilityDeps <DesiredFeatures>
# For each feature with requires[], locate providers in the desired feature set
# and add them as implicit entries in FeatureDeps.
# Returns $false if a required capability has no provider in the profile.
function Invoke-InjectCapabilityDeps {
    param(
        [Parameter(Mandatory=$true)]
        [string[]]$DesiredFeatures
    )

    foreach ($feature in $DesiredFeatures) {
        $caps = $script:Requires[$feature]
        if (-not $caps -or $caps.Count -eq 0) { continue }

        foreach ($cap in $caps) {
            $allProviders = @()
            if ($script:Provides.ContainsKey($cap)) {
                $allProviders = $script:Provides[$cap]
            }

            # Filter to providers present in the desired feature set
            $found = @($allProviders | Where-Object { $_ -in $DesiredFeatures })

            if ($found.Count -eq 0) {
                $knownList = if ($allProviders.Count -gt 0) { $allProviders -join ', ' } else { "(none registered)" }
                Log-Error "Feature '$feature' requires capability '$cap' but no provider is present in the profile."
                Log-Error "  Known providers: $knownList"
                return $false
            }

            # Inject as implicit depends (deduplicate)
            foreach ($p in $found) {
                if ($script:FeatureDeps[$feature] -notcontains $p) {
                    $script:FeatureDeps[$feature] = @($script:FeatureDeps[$feature]) + $p
                }
            }

            Log-Info "  ${feature}: capability '$cap' provided by: $($found -join ', ')"
        }
    }

    return $true
}

# Invoke-TopoSortDFS <Feature> <DesiredFeatures>
# Perform depth-first search for topological sorting.
function Invoke-TopoSortDFS {
    param(
        [Parameter(Mandatory=$true)]
        [string]$Feature,
        [Parameter(Mandatory=$true)]
        [string[]]$DesiredFeatures
    )

    # Check if already visited
    if ($script:Visited[$Feature]) {
        return $true
    }
    
    # Check for cycle
    if ($script:InStack[$Feature]) {
        Log-Error "Circular dependency detected involving: $Feature"
        return $false
    }
    
    $script:InStack[$Feature] = $true
    
    # Visit dependencies first
    $deps = $script:FeatureDeps[$Feature]
    if ($deps) {
        foreach ($dep in $deps) {
            # Check if dependency is in desired features
            if ($dep -notin $DesiredFeatures) {
                Log-Error "Dependency '$dep' (required by '$Feature') is not in profile"
                return $false
            }
            
            if (-not (Invoke-TopoSortDFS -Feature $dep -DesiredFeatures $DesiredFeatures)) {
                return $false
            }
        }
    }
    
    $script:InStack[$Feature] = $false
    $script:Visited[$Feature] = $true
    $script:Sorted += $Feature
    
    return $true
}

# Resolve-Dependencies <DesiredFeatures>
# Resolve capability dependencies and return topologically sorted feature list.
function Resolve-Dependencies {
    param(
        [Parameter(Mandatory=$true)]
        [string[]]$DesiredFeatures
    )

    $script:Visited  = @{}
    $script:InStack  = @{}
    $script:Sorted   = @()

    Log-Info "Resolving dependencies..."

    # Inject implicit deps derived from requires/provides into FeatureDeps
    if (-not (Invoke-InjectCapabilityDeps -DesiredFeatures $DesiredFeatures)) {
        return $null
    }

    # Sort all features
    foreach ($feature in $DesiredFeatures) {
        if (-not (Invoke-TopoSortDFS -Feature $feature -DesiredFeatures $DesiredFeatures)) {
            return $null
        }
    }

    Log-Success "Install order (canonical IDs): $($script:Sorted -join ' ')"
    return $script:Sorted
}
