# -----------------------------------------------------------------------------
# Module: policy_resolver (PolicyResolver, PowerShell)
#
# Responsibility:
#   Convert a raw DesiredResourceGraph (compiler output) into a Resolved
#   Resource Graph (RRG) by adding desired_backend to each package/runtime
#   resource according to the active backend policy.
#
# Public API (Stable):
#   Invoke-PolicyResolverRun <DrgJson>  → RRG JSON string, or $null on error
#
# Contract:
#   - package/runtime resources receive desired_backend via Resolve-BackendFor
#   - fs resources pass through unchanged (no backend applies)
#   - unknown resource kinds pass through unchanged (Planner will block them)
#   - top-level DRG fields (schema_version, etc.) are preserved
#
# This module requires backend_registry.ps1 (and its loaded policy) to be
# available before calling Invoke-PolicyResolverRun.
# -----------------------------------------------------------------------------

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Invoke-PolicyResolverRun <DrgJson>
# Add desired_backend to each package/runtime resource in the raw DRG.
# Returns Resolved Resource Graph JSON string, or $null on error.
function Invoke-PolicyResolverRun {
    param(
        [Parameter(Mandatory=$true)] [string]$DrgJson
    )

    if ([string]::IsNullOrWhiteSpace($DrgJson)) {
        Log-Error "Invoke-PolicyResolverRun: DrgJson is required"
        return $null
    }

    $drg  = $DrgJson | ConvertFrom-Json
    $featureNames = @($drg.features.PSObject.Properties | ForEach-Object { $_.Name })

    $featuresOut = [ordered]@{}

    foreach ($canonicalId in $featureNames) {
        $drgFeature = $drg.features.PSObject.Properties[$canonicalId].Value
        $rawResources = @()
        if ((_Prop $drgFeature 'resources')) {
            $rawResources = @($drgFeature.resources)
        }

        $resolvedResources = @()
        foreach ($res in $rawResources) {
            $kind = (_Prop $res 'kind')

            # Build a copy of the resource as an ordered hashtable
            $updated = [ordered]@{}
            foreach ($prop in $res.PSObject.Properties) {
                $updated[$prop.Name] = $prop.Value
            }

            switch ($kind) {
                "package" {
                    $name    = (_Prop $res 'name')
                    $backend = Resolve-BackendFor -Kind "package" -Name $name 2>$null
                    if (-not $backend) { $backend = "unknown" }
                    $updated['desired_backend'] = $backend
                }
                "runtime" {
                    $name    = (_Prop $res 'name')
                    $backend = Resolve-BackendFor -Kind "runtime" -Name $name 2>$null
                    if (-not $backend) { $backend = "unknown" }
                    $updated['desired_backend'] = $backend
                }
                default {
                    # fs resources have no backend.
                    # Unknown kinds pass through; Planner will classify them as blocked.
                }
            }

            $resolvedResources += $updated
        }

        $featuresOut[$canonicalId] = [ordered]@{ resources = $resolvedResources }
    }

    # Reconstruct top-level DRG preserving schema_version
    $rrg = [ordered]@{
        schema_version = $drg.schema_version
        features       = $featuresOut
    }

    return $rrg | ConvertTo-Json -Depth 20 -Compress:$false
}
