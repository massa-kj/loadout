# -----------------------------------------------------------------------------
# Module: feature_index (PowerShell)
#
# Responsibility:
#   Build the Feature Index by scanning all registered sources.
#   Produces a JSON Feature Index consumed by Resolver and FeatureCompiler.
#
# Public API (Stable):
#   Invoke-FeatureIndexBuild
#   Invoke-FeatureIndexFilter <FeatureIndexJson> <DesiredFeatures>
#
# JSON schema: see docs/specs/data/feature_index.md
# -----------------------------------------------------------------------------

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# This library expects env.ps1, logger.ps1, and source_registry.ps1 to be
# loaded by the caller.

# Lazily source source_registry if not already loaded
if (-not (Get-Command Canonical-Id-Normalize -ErrorAction SilentlyContinue)) {
    . "$env:LOADOUT_ROOT\core\lib\source_registry.ps1"
}

# _FeatureIndex-PlatformYaml <FeatureDir>
# Return the platform-specific feature file path, or $null if none.
function _FeatureIndex-PlatformYaml {
    param([string]$FeatureDir)

    $platform = $global:LOADOUT_PLATFORM
    if (-not $platform) { return $null }

    if ($platform -eq "wsl") {
        $wslFile   = Join-Path $FeatureDir "feature.wsl.yaml"
        $linuxFile = Join-Path $FeatureDir "feature.linux.yaml"
        if (Test-Path $wslFile)   { return $wslFile }
        if (Test-Path $linuxFile) { return $linuxFile }
    } else {
        $platFile = Join-Path $FeatureDir "feature.${platform}.yaml"
        if (Test-Path $platFile) { return $platFile }
    }
    return $null
}

# _FeatureIndex-ParseEntry <CanonicalId> <SourceId> <FeatureDir> <FeatureYaml>
# Return a hashtable describing one feature for the Feature Index.
function _FeatureIndex-ParseEntry {
    param(
        [Parameter(Mandatory=$true)] [string]$CanonicalId,
        [Parameter(Mandatory=$true)] [string]$SourceId,
        [Parameter(Mandatory=$true)] [string]$FeatureDir,
        [Parameter(Mandatory=$true)] [string]$FeatureYaml
    )

    # ── base fields ───────────────────────────────────────────────────────────
    # Trim all yq scalar output: Windows yq appends \r\n which breaks comparisons.
    $rawSv = (& yq eval '.spec_version // 1' $FeatureYaml 2>$null)
    if ($rawSv) { $rawSv = $rawSv.Trim() }
    $specVersion = if ($rawSv -match '^\d+$') { [int]$rawSv } else { 1 }

    $mode        = (& yq eval '.mode' $FeatureYaml 2>$null)
    if ($mode) { $mode = $mode.Trim() }
    if (-not $mode -or $mode -eq 'null') { $mode = 'declarative' }

    $description = (& yq eval '.description' $FeatureYaml 2>$null)
    if ($description) { $description = $description.Trim() }
    if (-not $description -or $description -eq 'null') { $description = '' }

    # ── platform override ─────────────────────────────────────────────────────
    $platformYaml = _FeatureIndex-PlatformYaml -FeatureDir $FeatureDir

    # ── spec_version check ────────────────────────────────────────────────────
    $maxVer = if ($global:SUPPORTED_FEATURE_SPEC_VERSION) {
        [int]$global:SUPPORTED_FEATURE_SPEC_VERSION
    } else { 1 }

    $blocked       = $specVersion -gt $maxVer
    $blockedReason = if ($blocked) {
        "unsupported spec_version: $specVersion (max: $maxVer)"
    } else { $null }

    # ── helper: read a yq list field ─────────────────────────────────────────
    $readYqList = {
        param($file, $expr)
        if (-not $file -or -not (Test-Path $file)) { return @() }
        $raw = & yq eval $expr $file 2>$null
        if ($LASTEXITCODE -ne 0 -or -not $raw) { return @() }
        return @($raw -split "`n" | Where-Object { $_ -and $_ -ne "null" })
    }

    # ── depends ───────────────────────────────────────────────────────────────
    $rawDeps = @(& $readYqList $FeatureYaml '.depends[]')
    if ($platformYaml) { $rawDeps += @(& $readYqList $platformYaml '.depends[]') }
    $normDeps = @($rawDeps | Select-Object -Unique | Where-Object { $_ } | ForEach-Object {
        Canonical-Id-Normalize -Name $_ -DefaultSourceId $SourceId
    })

    # ── provides ──────────────────────────────────────────────────────────────
    $provCaps = @(& $readYqList $FeatureYaml '.provides[].name')
    if ($platformYaml) { $provCaps += @(& $readYqList $platformYaml '.provides[].name') }
    $provides = @($provCaps | Select-Object -Unique | Where-Object { $_ } | ForEach-Object {
        [ordered]@{ name = $_ }
    })

    # ── requires ──────────────────────────────────────────────────────────────
    $reqCaps = @(& $readYqList $FeatureYaml '.requires[].name')
    if ($platformYaml) { $reqCaps += @(& $readYqList $platformYaml '.requires[].name') }
    $requires = @($reqCaps | Select-Object -Unique | Where-Object { $_ } | ForEach-Object {
        [ordered]@{ name = $_ }
    })

    # ── spec (resources for declarative mode) ─────────────────────────────────
    $spec = $null
    if ($mode -eq "declarative") {
        $resJson = & yq eval -o=json '.resources // []' $FeatureYaml 2>$null
        $resources = if ($resJson -and $resJson -ne "null") {
            @($resJson | ConvertFrom-Json)
        } else { @() }

        if ($platformYaml) {
            # Platform override replaces base resources if non-empty
            $platResJson = & yq eval -o=json '.resources // []' $platformYaml 2>$null
            if ($platResJson -and $platResJson -ne "null" -and $platResJson -ne "[]") {
                $resources = @($platResJson | ConvertFrom-Json)
            }
        }
        $spec = [ordered]@{ resources = $resources }
    }

    return [ordered]@{
        spec_version   = $specVersion
        mode           = $mode
        description    = $description
        source_dir     = $FeatureDir
        blocked        = $blocked
        blocked_reason = $blockedReason
        dep            = [ordered]@{
            depends  = $normDeps
            provides = $provides
            requires = $requires
        }
        spec           = $spec
    }
}

# Invoke-FeatureIndexBuild
# Build the Feature Index by scanning all registered sources (1 level deep).
# Returns Feature Index JSON string, or $null on error.
function Invoke-FeatureIndexBuild {
    if (-not (Source-Registry-Load)) { return $null }

    $sourceIds = Source-Registry-GetRegisteredSources
    $features  = [ordered]@{}

    foreach ($sourceId in $sourceIds) {
        $featureDirRoot = Source-Registry-GetFeatureDir -SourceId $sourceId
        if (-not (Test-Path $featureDirRoot)) { continue }

        $subDirs = @(Get-ChildItem -Path $featureDirRoot -Directory -ErrorAction SilentlyContinue |
                     Sort-Object Name)

        foreach ($dir in $subDirs) {
            $featureYaml = Join-Path $dir.FullName "feature.yaml"
            if (-not (Test-Path $featureYaml)) { continue }  # skip dirs without feature.yaml

            $canonicalId = "${sourceId}/$($dir.Name)"

            try {
                $entry = _FeatureIndex-ParseEntry `
                    -CanonicalId $canonicalId `
                    -SourceId    $sourceId `
                    -FeatureDir  $dir.FullName `
                    -FeatureYaml $featureYaml
                $features[$canonicalId] = $entry
            } catch {
                Log-Error "Invoke-FeatureIndexBuild: failed to parse $canonicalId - $_"
                return $null
            }
        }
    }

    $index = [ordered]@{
        schema_version = 1
        features       = $features
    }

    return $index | ConvertTo-Json -Depth 20 -Compress
}

# Invoke-FeatureIndexFilter <FeatureIndexJson> <DesiredFeatures>
# For each feature in DesiredFeatures, check the Feature Index:
#   - Not present → error
#   - blocked:true → adds to BlockedJson
#   - blocked:false → adds to Valid
# Returns PSCustomObject { Valid: string[]; BlockedJson: string }, or $null on error.
function Invoke-FeatureIndexFilter {
    param(
        [Parameter(Mandatory=$true)] [string]$FeatureIndexJson,
        [Parameter(Mandatory=$true)] [string[]]$DesiredFeatures
    )

    $index = $FeatureIndexJson | ConvertFrom-Json

    $valid   = [System.Collections.Generic.List[string]]::new()
    $blocked = @()

    foreach ($feature in $DesiredFeatures) {
        # Use PSObject.Properties to handle "/" in key names
        $prop = $index.features.PSObject.Properties[$feature]
        if ($null -eq $prop) {
            Log-Error "Invoke-FeatureIndexFilter: feature not found in index: $feature"
            return $null
        }
        $entry = $prop.Value

        if ($entry.blocked -eq $true) {
            $reason = if ($entry.blocked_reason) { $entry.blocked_reason } else { "" }
            Log-Warn "Blocked: $feature — $reason"
            $blocked += [PSCustomObject]@{ feature = $feature; reason = $reason }
        } else {
            $valid.Add($feature)
        }
    }

    $blockedJson = if ($blocked.Count -eq 0) {
        "[]"
    } else {
        $arr = $blocked | ConvertTo-Json -Compress -Depth 5
        if ($blocked.Count -eq 1) { "[$arr]" } else { $arr }
    }

    return [PSCustomObject]@{
        Valid       = $valid.ToArray()
        BlockedJson = $blockedJson
    }
}
