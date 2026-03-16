# -----------------------------------------------------------------------------
# Module: source_registry
#
# Responsibility:
#   Canonical ID utilities for feature and backend identification.
#   A canonical ID is a string of the form "<source_id>/<name>".
#
#   This module provides Phase 1 foundations only.
#   Source loading (sources.yaml, allow lists) is Phase 5.
#
# Reserved source IDs (may not be defined in sources.yaml):
#   core     -- built-in features shipped with this repository
#   user     -- local user overrides
#   official -- reserved for future use
#
# Public API (Stable):
#   Canonical-Id-Normalize <Name> <DefaultSourceId>
#   Canonical-Id-Parse     <CanonicalId>
#   Canonical-Id-Validate  <CanonicalId>
#   Source-Registry-Load [SourcesFile]
#   Source-Registry-GetFeatureDir <SourceId>
#   Source-Registry-GetBackendDir <SourceId>
#   Source-Registry-IsAllowed <SourceId> <FeatureName>
#   Source-Registry-IsBackendAllowed <SourceId> <BackendName>
# -----------------------------------------------------------------------------

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# This library expects logger.ps1 to be loaded by the caller.

# List of source IDs that cannot be defined by the user in sources.yaml.
$script:CanonicalIdReservedSources = @("core", "user", "official")
$script:SourceRegistryLoaded = $false
$script:SourceRegistrySourceTypes = @{}
$script:SourceRegistryFeatureDirs = @{}
$script:SourceRegistryBackendDirs = @{}
$script:SourceRegistryAllowFeaturesMode = @{}
$script:SourceRegistryAllowBackendsMode = @{}
$script:SourceRegistryAllowFeaturesList = @{}
$script:SourceRegistryAllowBackendsList = @{}

# Canonical-Id-Normalize <Name> <DefaultSourceId>
#
# Normalize a feature/backend name to a canonical ID.
# If <Name> is already a canonical ID ("source/name"), it is returned as-is.
# If <Name> is a bare name, "<DefaultSourceId>/<Name>" is produced.
#
# Returns the canonical ID string.
# Throws if the resulting canonical ID would be invalid.
#
# Examples:
#   Canonical-Id-Normalize "git"         "core"   -> "core/git"
#   Canonical-Id-Normalize "user/myfeat" "core"   -> "user/myfeat"
#   Canonical-Id-Normalize "repo-a/foo"  "core"   -> "repo-a/foo"
function Canonical-Id-Normalize {
    param(
        [string]$Name,
        [string]$DefaultSourceId
    )

    if ([string]::IsNullOrEmpty($Name)) {
        throw "Canonical-Id-Normalize: Name is required"
    }
    if ([string]::IsNullOrEmpty($DefaultSourceId)) {
        throw "Canonical-Id-Normalize: DefaultSourceId is required"
    }

    $result = if ($Name -match '/') {
        # Already contains a slash — treat as canonical
        $Name
    } else {
        # Bare name — prepend default source
        "${DefaultSourceId}/${Name}"
    }

    if (-not (Canonical-Id-Validate $result)) {
        throw "Canonical-Id-Normalize: resulting ID is invalid: '$result'"
    }

    return $result
}

# Canonical-Id-Parse <CanonicalId>
#
# Parse a canonical ID and return a hashtable with keys SourceId and Name.
# Throws if <CanonicalId> is not a valid canonical ID.
#
# Example:
#   $parts = Canonical-Id-Parse "core/git"
#   $parts.SourceId  # "core"
#   $parts.Name      # "git"
function Canonical-Id-Parse {
    param([string]$CanonicalId)

    if (-not (Canonical-Id-Validate $CanonicalId)) {
        throw "Canonical-Id-Parse: invalid canonical ID: '$CanonicalId'"
    }

    $slashIndex = $CanonicalId.IndexOf('/')
    return @{
        SourceId = $CanonicalId.Substring(0, $slashIndex)
        Name     = $CanonicalId.Substring($slashIndex + 1)
    }
}

# Canonical-Id-Validate <CanonicalId>
#
# Return $true if <CanonicalId> is a well-formed canonical ID, $false otherwise.
# A valid canonical ID:
#   - is non-empty
#   - contains exactly one "/" separator
#   - has a non-empty source_id part (left of "/")
#   - has a non-empty name part (right of "/")
#   - neither part contains a "/"
#
# Does NOT check whether the source_id is reserved — that is the caller's responsibility.
function Canonical-Id-Validate {
    param([string]$CanonicalId)

    if ([string]::IsNullOrEmpty($CanonicalId)) {
        return $false
    }

    # Must contain exactly one "/"
    # Use -split which always returns an array (PS 5.1 safe; Where-Object returns $null on no match)
    $slashCount = ($CanonicalId -split '/').Count - 1
    if ($slashCount -ne 1) {
        return $false
    }

    $slashIndex = $CanonicalId.IndexOf('/')
    $sourcePart = $CanonicalId.Substring(0, $slashIndex)
    $namePart   = $CanonicalId.Substring($slashIndex + 1)

    if ([string]::IsNullOrEmpty($sourcePart) -or [string]::IsNullOrEmpty($namePart)) {
        return $false
    }

    return $true
}

function _Source-Registry-RegisterSource {
    param(
        [string]$SourceId,
        [string]$SourceType,
        [string]$FeatureDir,
        [string]$BackendDir
    )

    $script:SourceRegistrySourceTypes[$SourceId] = $SourceType
    $script:SourceRegistryFeatureDirs[$SourceId] = $FeatureDir
    $script:SourceRegistryBackendDirs[$SourceId] = $BackendDir

    if ($SourceId -in @("core", "user")) {
        $script:SourceRegistryAllowFeaturesMode[$SourceId] = "all"
        $script:SourceRegistryAllowBackendsMode[$SourceId] = "all"
    } else {
        $script:SourceRegistryAllowFeaturesMode[$SourceId] = "none"
        $script:SourceRegistryAllowBackendsMode[$SourceId] = "none"
    }
    $script:SourceRegistryAllowFeaturesList[$SourceId] = @()
    $script:SourceRegistryAllowBackendsList[$SourceId] = @()
}

function _Source-Registry-LoadImplicit {
    _Source-Registry-RegisterSource -SourceId "core" -SourceType "implicit" -FeatureDir (Join-Path $global:LOADOUT_ROOT "features") -BackendDir (Join-Path $global:LOADOUT_ROOT "backends")
    _Source-Registry-RegisterSource -SourceId "user" -SourceType "implicit" -FeatureDir (Join-Path $global:LOADOUT_CONFIG_HOME "features") -BackendDir (Join-Path $global:LOADOUT_CONFIG_HOME "backends")
}

function _Source-Registry-EnsureLoaded {
    if (-not $script:SourceRegistryLoaded) {
        Source-Registry-Load | Out-Null
    }
}

function Source-Registry-Load {
    param([string]$SourcesFile = "")

    $file = if ($SourcesFile) { $SourcesFile } elseif ($global:LOADOUT_SOURCES_FILE) { $global:LOADOUT_SOURCES_FILE } else { $null }

    $script:SourceRegistrySourceTypes = @{}
    $script:SourceRegistryFeatureDirs = @{}
    $script:SourceRegistryBackendDirs = @{}
    $script:SourceRegistryAllowFeaturesMode = @{}
    $script:SourceRegistryAllowBackendsMode = @{}
    $script:SourceRegistryAllowFeaturesList = @{}
    $script:SourceRegistryAllowBackendsList = @{}

    _Source-Registry-LoadImplicit

    if (-not $file -or -not (Test-Path $file)) {
        $script:SourceRegistryLoaded = $true
        return $true
    }

    try {
        $json = & yq eval -o=json '.' $file 2>$null
        if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($json)) {
            throw "failed to parse sources file"
        }
        $data = $json | ConvertFrom-Json
    } catch {
        Log-Error "Source-Registry-Load: failed to parse sources file: $file"
        return $false
    }

    foreach ($src in @($data.sources)) {
        if ($null -eq $src -or [string]::IsNullOrWhiteSpace($src.id)) { continue }

        if ($src.id -in $script:CanonicalIdReservedSources) {
            Log-Error "Source-Registry-Load: reserved source id may not be defined in sources.yaml: $($src.id)"
            return $false
        }
        if ($src.type -ne 'git') {
            Log-Error "Source-Registry-Load: unsupported source type for '$($src.id)': $($src.type)"
            return $false
        }

        _Source-Registry-RegisterSource -SourceId $src.id -SourceType $src.type -FeatureDir (Join-Path $global:LOADOUT_DATA_HOME "sources\$($src.id)\features") -BackendDir (Join-Path $global:LOADOUT_DATA_HOME "sources\$($src.id)\backends")

        if ($src.PSObject.Properties['allow']) {
            if ($src.allow -is [string] -and $src.allow -eq '*') {
                $script:SourceRegistryAllowFeaturesMode[$src.id] = 'all'
                $script:SourceRegistryAllowBackendsMode[$src.id] = 'all'
            } else {
                if ($src.allow.PSObject.Properties['features']) {
                    if ($src.allow.features -is [string] -and $src.allow.features -eq '*') {
                        $script:SourceRegistryAllowFeaturesMode[$src.id] = 'all'
                    } else {
                        $script:SourceRegistryAllowFeaturesMode[$src.id] = 'list'
                        $script:SourceRegistryAllowFeaturesList[$src.id] = @($src.allow.features)
                    }
                }
                if ($src.allow.PSObject.Properties['backends']) {
                    if ($src.allow.backends -is [string] -and $src.allow.backends -eq '*') {
                        $script:SourceRegistryAllowBackendsMode[$src.id] = 'all'
                    } else {
                        $script:SourceRegistryAllowBackendsMode[$src.id] = 'list'
                        $script:SourceRegistryAllowBackendsList[$src.id] = @($src.allow.backends)
                    }
                }
            }
        }
    }

    $script:SourceRegistryLoaded = $true
    return $true
}

function Source-Registry-GetFeatureDir {
    param([string]$SourceId)
    _Source-Registry-EnsureLoaded
    if (-not $script:SourceRegistryFeatureDirs.ContainsKey($SourceId)) {
        throw "Source-Registry-GetFeatureDir: unknown source id: $SourceId"
    }
    return $script:SourceRegistryFeatureDirs[$SourceId]
}

function Source-Registry-GetBackendDir {
    param([string]$SourceId)
    _Source-Registry-EnsureLoaded
    if (-not $script:SourceRegistryBackendDirs.ContainsKey($SourceId)) {
        throw "Source-Registry-GetBackendDir: unknown source id: $SourceId"
    }
    return $script:SourceRegistryBackendDirs[$SourceId]
}

# Source-Registry-GetRegisteredSources
# Return a sorted array of all registered source IDs.
function Source-Registry-GetRegisteredSources {
    _Source-Registry-EnsureLoaded
    return @($script:SourceRegistrySourceTypes.Keys | Sort-Object)
}

function _Source-Registry-IsAllowedKind {
    param(
        [string]$SourceId,
        [string]$Name,
        [ValidateSet('feature','backend')] [string]$Kind
    )

    _Source-Registry-EnsureLoaded

    if (-not $script:SourceRegistrySourceTypes.ContainsKey($SourceId)) {
        return $false
    }
    if ($SourceId -in @('core', 'user')) {
        return $true
    }

    if ($Kind -eq 'feature') {
        $mode = $script:SourceRegistryAllowFeaturesMode[$SourceId]
        $list = @($script:SourceRegistryAllowFeaturesList[$SourceId])
    } else {
        $mode = $script:SourceRegistryAllowBackendsMode[$SourceId]
        $list = @($script:SourceRegistryAllowBackendsList[$SourceId])
    }

    switch ($mode) {
        'all' { return $true }
        'list' { return $Name -in $list }
        default { return $false }
    }
}

function Source-Registry-IsAllowed {
    param([string]$SourceId, [string]$FeatureName)
    return _Source-Registry-IsAllowedKind -SourceId $SourceId -Name $FeatureName -Kind feature
}

function Source-Registry-IsBackendAllowed {
    param([string]$SourceId, [string]$BackendName)
    return _Source-Registry-IsAllowedKind -SourceId $SourceId -Name $BackendName -Kind backend
}
