# -----------------------------------------------------------------------------
# Backend plugin: uv (PowerShell)
#
# Manages Python packages installed via `uv pip install --system`.
# Uses `mise exec --` prefix to ensure the mise-managed uv binary is invoked
# even when mise is not activated in the caller's shell.
#
# Backend Plugin Contract (CORE.md §Backend Plugin Contract):
#   Backend-ApiVersion
#   Backend-ManagerExists
#   Backend-PackageExists <Name>
#   Backend-InstallPackage <Name> [<Version>]
#   Backend-UninstallPackage <Name>
#   Backend-InstallRuntime   — not supported
#   Backend-UninstallRuntime — not supported
#   Backend-RuntimeExists    — not supported
# -----------------------------------------------------------------------------

function Backend-ApiVersion { return "1" }

# _Uv-Cmd [Args...]
# Run a uv command, preferring the mise-managed uv binary when mise is available.
# Use `mise exec uv --` rather than `mise exec --` to activate ONLY the uv tool;
# `mise exec --` (no tool specified) activates ALL configured tools.
function _Uv-Cmd {
    param([Parameter(ValueFromRemainingArguments=$true)] $UvArgs)
    if (Get-Command mise -ErrorAction SilentlyContinue) {
        & mise exec uv -- uv @UvArgs
    } else {
        & uv @UvArgs
    }
}

# Backend-ManagerExists
# Return $true if uv is available (either directly or via mise).
function Backend-ManagerExists {
    try {
        if (Get-Command mise -ErrorAction SilentlyContinue) {
            & mise exec uv -- uv --version 2>$null | Out-Null
            return $LASTEXITCODE -eq 0
        } else {
            return [bool](Get-Command uv -ErrorAction SilentlyContinue)
        }
    } catch { return $false }
}

# Backend-PackageExists <Name>
# Return $true if the named package is installed in the system Python environment.
function Backend-PackageExists {
    param([Parameter(Mandatory=$true)] [string]$Name)
    try {
        $list = _Uv-Cmd pip list 2>$null
        return ($list -match "(?i)^$([regex]::Escape($Name))\s")
    } catch { return $false }
}

# Backend-InstallPackage <Name> [<Version>]
# Install a package into the system Python environment via uv.
function Backend-InstallPackage {
    param(
        [Parameter(Mandatory=$true)] [string]$Name,
        [string]$Version = ""
    )

    if (-not (Backend-ManagerExists)) {
        Log-Error "uv backend: uv not available (is python installed via mise?)"
        throw "uv not available"
    }

    $spec = if (-not [string]::IsNullOrWhiteSpace($Version)) { "${Name}==${Version}" } else { $Name }

    Log-Info "uv: installing package: $spec"
    _Uv-Cmd pip install --system $spec
    if ($LASTEXITCODE -ne 0) {
        Log-Error "uv backend: failed to install: $spec"
        throw "uv install failed: $spec"
    }
    Log-Info "uv: installed: $spec"
}

# Backend-UninstallPackage <Name>
# Remove a package from the system Python environment. No-ops if not installed.
function Backend-UninstallPackage {
    param([Parameter(Mandatory=$true)] [string]$Name)

    if (-not (Backend-ManagerExists)) {
        Log-Warn "uv backend: uv not available; skipping uninstall of $Name"
        return
    }

    if (-not (Backend-PackageExists -Name $Name)) {
        Log-Info "uv: package not installed, skipping: $Name"
        return
    }

    Log-Info "uv: uninstalling package: $Name"
    _Uv-Cmd pip uninstall --system $Name
    if ($LASTEXITCODE -ne 0) {
        Log-Error "uv backend: failed to uninstall: $Name"
        throw "uv uninstall failed: $Name"
    }
    Log-Info "uv: uninstalled: $Name"
}

# Runtime management — not supported by uv backend.
function Backend-InstallRuntime   { Log-Error "uv backend: runtime management not supported"; throw "not supported" }
function Backend-UninstallRuntime { Log-Error "uv backend: runtime management not supported"; throw "not supported" }
function Backend-RuntimeExists    { return $false }
