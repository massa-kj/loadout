# -----------------------------------------------------------------------------
# Backend plugin: mise (PowerShell)
#
# Adapter for mise (https://mise.jdx.dev).
# Handles runtime installation on Windows (node, python, rust, lua, etc.).
# Implements the Backend Plugin Contract defined in docs/specs/api/backend.md.
#
# Capabilities: runtime
# Does NOT support: system package installation (use scoop or winget backend)
# -----------------------------------------------------------------------------

# ── Contract ──────────────────────────────────────────────────────────────────

function Backend-ApiVersion { return "1" }
function Backend-Capabilities { return "runtime" }

# ── Internal helpers ──────────────────────────────────────────────────────────

function _Mise-Ensure {
    <#
    .SYNOPSIS Make mise available in PATH. Returns $true if found, $false otherwise.
    #>
    if (Get-Command mise -ErrorAction SilentlyContinue) {
        return $true
    }

    $candidates = @(
        "$env:USERPROFILE\.local\bin\mise.exe",
        "$env:LOCALAPPDATA\mise\bin\mise.exe",
        "$env:APPDATA\mise\bin\mise.exe"
    )
    foreach ($path in $candidates) {
        if (Test-Path $path) {
            $dir = Split-Path -Parent $path
            $env:PATH = "$dir;$env:PATH"
            return $true
        }
    }

    Log-Error "mise backend: mise not found"
    return $false
}

function _Mise-ResolveVersion {
    <#
    .SYNOPSIS Resolve a version alias ("latest", "22") to a concrete version string.
              Returns the resolved version, or the original alias on failure.
    #>
    param(
        [Parameter(Mandatory=$true)] [string]$Name,
        [Parameter(Mandatory=$true)] [string]$Version
    )

    # Already a concrete version (digits and dots only)
    if ($Version -match '^\d+(\.\d+)*$') {
        return $Version
    }

    # Resolve via mise latest
    $resolved = $null
    try {
        $ea = $ErrorActionPreference; $ErrorActionPreference = "Continue"
        $raw = & mise latest "${Name}@${Version}" 2>&1
        $ErrorActionPreference = $ea
        if ($LASTEXITCODE -eq 0) {
            $resolved = ($raw | Where-Object { $_ -notmatch '^mise' } | Select-Object -First 1)
        }
    } catch { $ErrorActionPreference = $ea }
    if (-not [string]::IsNullOrWhiteSpace($resolved)) {
        return $resolved.Trim()
    }

    # Fall back to the alias as-is
    return $Version
}

# ── Observation API ───────────────────────────────────────────────────────────

function Backend-ManagerExists {
    return (_Mise-Ensure)
}

function Backend-PackageExists {
    # mise does not manage system packages
    return $false
}

function Backend-RuntimeExists {
    <#
    .SYNOPSIS Return $true if the named runtime version is installed via mise.
    #>
    param(
        [Parameter(Mandatory=$true)] [string]$Name,
        [string]$Version = ""
    )

    if (-not (_Mise-Ensure)) { return $false }

    if (-not [string]::IsNullOrWhiteSpace($Version)) {
        $resolved = _Mise-ResolveVersion -Name $Name -Version $Version
        $ea = $ErrorActionPreference; $ErrorActionPreference = "Continue"
        try { & mise where "${Name}@${resolved}" 2>&1 | Out-Null } catch {}
        $ErrorActionPreference = $ea
        return ($LASTEXITCODE -eq 0)
    } else {
        $ea = $ErrorActionPreference; $ErrorActionPreference = "Continue"
        try { & mise ls --installed $Name 2>&1 | Out-Null } catch {}
        $ErrorActionPreference = $ea
        return ($LASTEXITCODE -eq 0)
    }
}

# ── Execution API ─────────────────────────────────────────────────────────────

function Backend-InstallPackage {
    Log-Error "mise: system package installation is not supported; use the scoop backend"
    return 1
}

function Backend-UninstallPackage {
    Log-Error "mise: system package uninstallation is not supported; use the scoop backend"
    return 1
}

function Backend-InstallRuntime {
    <#
    .SYNOPSIS Install a runtime via mise and set it as the global default.
              Prints the concrete resolved version to stdout. Idempotent.
    #>
    param(
        [Parameter(Mandatory=$true)] [string]$Name,
        [Parameter(Mandatory=$true)] [string]$Version
    )

    if ([string]::IsNullOrWhiteSpace($Name) -or [string]::IsNullOrWhiteSpace($Version)) {
        Log-Error "mise: Backend-InstallRuntime: name and version are required"
        return 1
    }

    if (-not (_Mise-Ensure)) { return 1 }

    $resolved = _Mise-ResolveVersion -Name $Name -Version $Version

    if (Backend-RuntimeExists -Name $Name -Version $resolved) {
        Log-Info "mise: runtime already installed: ${Name}@${resolved}"
        return $resolved
    }

    Log-Info "mise: installing runtime: ${Name}@${resolved}"
    & mise install "${Name}@${resolved}"
    if ($LASTEXITCODE -ne 0) {
        Log-Error "mise: install failed: ${Name}@${resolved}"
        return 1
    }

    Log-Info "mise: setting global: ${Name}@${resolved}"
    & mise use -g "${Name}@${resolved}"
    if ($LASTEXITCODE -ne 0) {
        Log-Warn "mise: failed to set global default for: ${Name}@${resolved}"
    }

    # Add runtime's bin directory to PATH for the current session
    $runtimePath = $null
    try {
        $ea = $ErrorActionPreference; $ErrorActionPreference = "Continue"
        $raw = & mise where "${Name}@${resolved}" 2>&1
        $ErrorActionPreference = $ea
        if ($LASTEXITCODE -eq 0) {
            $runtimePath = ($raw | Where-Object { $_ -notmatch '^mise' } | Select-Object -First 1)
        }
    } catch { $ErrorActionPreference = $ea }
    if (-not [string]::IsNullOrWhiteSpace($runtimePath)) {
        $binPath = Join-Path $runtimePath.Trim() "bin"
        if (Test-Path $binPath) {
            $env:PATH = "$binPath;$env:PATH"
        }
    }

    return $resolved
}

function Backend-UninstallRuntime {
    <#
    .SYNOPSIS Uninstall a runtime via mise. Idempotent.
    #>
    param(
        [Parameter(Mandatory=$true)] [string]$Name,
        [string]$Version = ""
    )

    if (-not (_Mise-Ensure)) { return 1 }

    if (-not [string]::IsNullOrWhiteSpace($Version)) {
        if (-not (Backend-RuntimeExists -Name $Name -Version $Version)) {
            Log-Info "mise: runtime not installed: ${Name}@${Version}"
            return 0
        }
        Log-Info "mise: uninstalling runtime: ${Name}@${Version}"
        & mise uninstall "${Name}@${Version}"
        if ($LASTEXITCODE -ne 0) {
            Log-Error "mise: uninstall failed: ${Name}@${Version}"
            return 1
        }
    } else {
        if (-not (Backend-RuntimeExists -Name $Name)) {
            Log-Info "mise: runtime not installed: $Name"
            return 0
        }
        Log-Info "mise: uninstalling runtime: $Name"
        & mise uninstall $Name
        if ($LASTEXITCODE -ne 0) {
            Log-Error "mise: uninstall failed: $Name"
            return 1
        }
    }

    return 0
}
