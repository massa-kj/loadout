# -----------------------------------------------------------------------------
# Module: backend_registry
#
# Responsibility:
#   Resolve, load, and dispatch backend plugin operations.
#   Acts as the sole stable gate between core and backend execution adapters.
#
# Public API (Stable):
#   Backend-Registry-LoadPolicy [PolicyFile]
#   Resolve-BackendFor <Kind> <Name>
#   Load-Backend <BackendId>
#   Backend-Call <Op> <Args...>
# -----------------------------------------------------------------------------

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# This library expects env.ps1 and logger.ps1 to be loaded by the caller.

if (-not (Get-Command Canonical-Id-Normalize -ErrorAction SilentlyContinue)) {
    . "$env:LOADOUT_ROOT\core\lib\source_registry.ps1"
}

# ── Private state ─────────────────────────────────────────────────────────────

# Cached parsed policy object.
$script:BrPolicyData = $null

# ID of the currently loaded backend plugin.
$script:BrLoadedBackend = ""

# ── Policy loading ────────────────────────────────────────────────────────────

# Backend-Registry-LoadPolicy [PolicyFile]
# Load policy YAML into memory cache.
# Uses LOADOUT_POLICY_FILE if no argument is given.
function Backend-Registry-LoadPolicy {
    param([string]$PolicyFile = "")

    $file = if ($PolicyFile) { $PolicyFile } else { $global:LOADOUT_POLICY_FILE }

    if (-not $file) { return }

    if (-not (Test-Path $file)) {
        Log-Warn "Backend-Registry-LoadPolicy: policy file not found: $file (using platform defaults)"
        return
    }

    # Parse YAML via yq into JSON, then into a PS object
    try {
        $json = & yq eval '.' -o=json $file 2>$null
        $script:BrPolicyData = $json | ConvertFrom-Json
    } catch {
        Log-Warn "Backend-Registry-LoadPolicy: failed to parse policy file: $_"
    }
}

# ── Backend resolution ────────────────────────────────────────────────────────

# Resolve-BackendFor <Kind> <Name>
# Return the backend_id for the given kind/name pair.
# Resolution order:
#   1. Policy overrides: .<kind>.overrides.<name>.backend  (resource name, not feature name)
#   2. Policy default:   .<kind>.default_backend
#   3. Platform default (hardcoded)
function Resolve-BackendFor {
    param(
        [Parameter(Mandatory=$true)] [string]$Kind,
        [Parameter(Mandatory=$true)] [string]$Name
    )

    # Lazy-load policy on first call
    if ($null -eq $script:BrPolicyData -and $global:LOADOUT_POLICY_FILE) {
        Backend-Registry-LoadPolicy
    }

    if ($null -ne $script:BrPolicyData) {
        $policy = $script:BrPolicyData

        if ($Kind -notin @("package", "runtime")) {
            Log-Error "Resolve-BackendFor: unsupported kind: $Kind"
            throw "Unsupported kind: $Kind"
        }

        # 1. Per-resource override (keyed by resource name, not feature name)
        if ($policy.PSObject.Properties[$Kind] -and
            $policy.$Kind.PSObject.Properties['overrides'] -and
            $policy.$Kind.overrides.PSObject.Properties[$Name] -and
            $policy.$Kind.overrides.$Name.PSObject.Properties['backend']) {
            return (Canonical-Id-Normalize -Name $policy.$Kind.overrides.$Name.backend -DefaultSourceId "core")
        }

        # 2. Kind-level default
        if ($policy.PSObject.Properties[$Kind] -and
            $policy.$Kind.PSObject.Properties['default_backend']) {
            return (Canonical-Id-Normalize -Name $policy.$Kind.default_backend -DefaultSourceId "core")
        }
    }

    return _Get-PlatformDefaultBackend -Kind $Kind
}

# _Get-PlatformDefaultBackend <Kind>
# Return the hardcoded platform default backend_id.
function _Get-PlatformDefaultBackend {
    param([string]$Kind)

    switch ($global:LOADOUT_PLATFORM) {
        { $_ -in @("linux", "wsl") } {
            switch ($Kind) {
                "package" { return "core/brew" }
                "runtime" { return "core/mise" }
                default { throw "Unsupported kind: $Kind" }
            }
        }
        "windows" {
            switch ($Kind) {
                "package" { return "core/scoop" }
                "runtime" { return "core/mise" }
                default { throw "Unsupported kind: $Kind" }
            }
        }
        default {
            throw "Unsupported platform: $($global:LOADOUT_PLATFORM)"
        }
    }
}

# ── Plugin loading ────────────────────────────────────────────────────────────

# Load-Backend <BackendId>
# Dot-source the backend plugin file and validate the Backend Plugin Contract.
function Load-Backend {
    param([Parameter(Mandatory=$true)] [string]$BackendId)

    $canonicalBackendId = Canonical-Id-Normalize -Name $BackendId -DefaultSourceId "core"
    $parts = Canonical-Id-Parse $canonicalBackendId

    if (-not (Source-Registry-Load)) {
        throw "Failed to load source registry"
    }
    if (-not (Source-Registry-IsBackendAllowed -SourceId $parts.SourceId -BackendName $parts.Name)) {
        Log-Error "Load-Backend: backend is not allowed by source registry: $canonicalBackendId"
        throw "Backend is not allowed by source registry: $canonicalBackendId"
    }

    # Skip re-loading if same backend is already loaded
    if ($script:BrLoadedBackend -eq $canonicalBackendId) { return }

    $backendDir = Source-Registry-GetBackendDir -SourceId $parts.SourceId
    $pluginFile = Join-Path $backendDir "$($parts.Name).ps1"
    if (-not (Test-Path $pluginFile)) {
        Log-Error "Load-Backend: plugin not found: $pluginFile"
        throw "Backend plugin not found: $pluginFile"
    }

    # Dot-source inside a function would confine definitions to function scope,
    # making them invisible after Load-Backend returns.  Rewrite all plugin
    # function declarations to "function global:*" so they persist in the global
    # scope and remain callable from Backend-Call and from each other.
    $pluginContent = [System.IO.File]::ReadAllText($pluginFile)
    $pluginContent  = $pluginContent -replace '(?m)^function ([A-Za-z_])', 'function global:$1'
    Invoke-Expression $pluginContent

    # Validate minimum contract
    if (-not (Get-Command "Backend-ApiVersion" -ErrorAction SilentlyContinue)) {
        $script:BrLoadedBackend = ""
        Log-Error "Load-Backend: contract violation: Backend-ApiVersion not defined (plugin: $pluginFile)"
        throw "Backend Plugin Contract violation"
    }

    $script:BrLoadedBackend = $canonicalBackendId
    Log-Info "Load-Backend: loaded backend plugin: $canonicalBackendId (api_version=$(Backend-ApiVersion))"
}

# ── Dispatch ──────────────────────────────────────────────────────────────────

# Backend-Call <Op> [Args...]
# Call an operation on the currently loaded backend plugin.
function Backend-Call {
    param(
        [Parameter(Mandatory=$true)] [string]$Op,
        [Parameter(ValueFromRemainingArguments=$true)] $Args
    )

    if (-not $script:BrLoadedBackend) {
        Log-Error "Backend-Call: no backend loaded; call Load-Backend first"
        throw "No backend loaded"
    }

    # Callers pass PascalCase op names (e.g. "PackageExists"); use directly.
    $funcName = "Backend-$Op"

    if (-not (Get-Command $funcName -ErrorAction SilentlyContinue)) {
        Log-Error "Backend-Call: operation not defined: $funcName (loaded backend: $($script:BrLoadedBackend))"
        throw "Backend operation not defined: $funcName"
    }

    & $funcName @Args
}
