# -----------------------------------------------------------------------------
# Backend plugin: scoop (PowerShell)
#
# Adapter for Scoop (https://scoop.sh).
# Handles system package installation on Windows.
# Implements the Backend Plugin Contract defined in CORE.md.
#
# Capabilities: package
# Does NOT support: runtime management (use mise backend)
# -----------------------------------------------------------------------------

# ── Contract ──────────────────────────────────────────────────────────────────

function Backend-ApiVersion { return "1" }
function Backend-Capabilities { return "package" }

# ── Internal helpers ──────────────────────────────────────────────────────────

function _Scoop-Ensure {
    <#
    .SYNOPSIS Make scoop available in PATH. Returns $true if found, $false otherwise.
    #>
    if (Get-Command scoop -ErrorAction SilentlyContinue) {
        return $true
    }

    $candidates = @(
        "$env:USERPROFILE\scoop\shims\scoop.cmd",
        "$env:LOCALAPPDATA\Programs\scoop\shims\scoop.cmd"
    )
    foreach ($path in $candidates) {
        if (Test-Path $path) {
            $dir = Split-Path -Parent $path
            $env:PATH = "$dir;$env:PATH"
            return $true
        }
    }

    Log-Error "scoop backend: scoop not found"
    return $false
}

# ── Observation API ───────────────────────────────────────────────────────────

function Backend-ManagerExists {
    return (_Scoop-Ensure)
}

function Backend-PackageExists {
    <#
    .SYNOPSIS Return $true if package is installed via scoop.
    #>
    param([string]$Name, [string]$Version = "")

    if (-not (_Scoop-Ensure)) { return $false }

    $installed = scoop list 2>$null | Where-Object { $_ -match "^\s+$([regex]::Escape($Name))\b" }
    return ($null -ne $installed -and $installed.Count -gt 0)
}

function Backend-RuntimeExists {
    # Scoop does not manage runtimes
    return $false
}

# ── Execution API ─────────────────────────────────────────────────────────────

function Backend-InstallPackage {
    <#
    .SYNOPSIS Install a package via scoop. Idempotent.
    #>
    param([string]$Name, [string]$Version = "")

    if ([string]::IsNullOrWhiteSpace($Name)) {
        Log-Error "scoop: Backend-InstallPackage: name is required"
        return 1
    }

    if (-not (_Scoop-Ensure)) { return 1 }

    if (Backend-PackageExists -Name $Name) {
        Log-Info "scoop: package already installed: $Name"
        return 0
    }

    Log-Info "scoop: installing package: $Name"
    scoop install $Name
    if ($LASTEXITCODE -ne 0) {
        Log-Error "scoop: install failed: $Name"
        return 1
    }

    return 0
}

function Backend-UninstallPackage {
    <#
    .SYNOPSIS Uninstall a package via scoop. Idempotent.
    #>
    param([string]$Name, [string]$Version = "")

    if ([string]::IsNullOrWhiteSpace($Name)) {
        Log-Error "scoop: Backend-UninstallPackage: name is required"
        return 1
    }

    if (-not (_Scoop-Ensure)) { return 1 }

    if (-not (Backend-PackageExists -Name $Name)) {
        Log-Info "scoop: package not installed: $Name"
        return 0
    }

    Log-Info "scoop: uninstalling package: $Name"
    scoop uninstall $Name
    if ($LASTEXITCODE -ne 0) {
        Log-Error "scoop: uninstall failed: $Name"
        return 1
    }

    return 0
}

function Backend-InstallRuntime {
    Log-Error "scoop: runtime management is not supported; use the mise backend"
    return 1
}

function Backend-UninstallRuntime {
    Log-Error "scoop: runtime management is not supported; use the mise backend"
    return 1
}
