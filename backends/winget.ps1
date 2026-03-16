# -----------------------------------------------------------------------------
# Backend plugin: winget (PowerShell) — STUB
#
# Adapter for Windows Package Manager (winget).
# This implementation is a stub; winget support is not yet fully implemented.
# Implements the Backend Plugin Contract defined in CORE.md.
#
# Capabilities: package
# Does NOT support: runtime management (use mise backend)
# -----------------------------------------------------------------------------

# ── Contract ──────────────────────────────────────────────────────────────────

function Backend-ApiVersion { return "1" }
function Backend-Capabilities { return "package" }

# ── Observation API ───────────────────────────────────────────────────────────

function Backend-ManagerExists {
    return [bool](Get-Command winget -ErrorAction SilentlyContinue)
}

function Backend-PackageExists {
    <#
    .SYNOPSIS Return $true if package is installed (STUB — always returns $false).
    #>
    param([string]$Name, [string]$Version = "")

    Log-Warn "winget: Backend-PackageExists is a stub; returning false"
    return $false
}

function Backend-RuntimeExists {
    return $false
}

# ── Execution API ─────────────────────────────────────────────────────────────

function Backend-InstallPackage {
    <#
    .SYNOPSIS Install a package via winget. (STUB)
    #>
    param([string]$Name, [string]$Version = "")

    Log-Error "winget backend is not yet fully implemented"
    return 1
}

function Backend-UninstallPackage {
    <#
    .SYNOPSIS Uninstall a package via winget. (STUB)
    #>
    param([string]$Name, [string]$Version = "")

    Log-Error "winget backend is not yet fully implemented"
    return 1
}

function Backend-InstallRuntime {
    Log-Error "winget: runtime management is not supported; use the mise backend"
    return 1
}

function Backend-UninstallRuntime {
    Log-Error "winget: runtime management is not supported; use the mise backend"
    return 1
}
