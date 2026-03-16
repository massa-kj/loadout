# -----------------------------------------------------------------------------
# Backend plugin: npm (PowerShell)
#
# Manages npm global packages (npm install -g / npm uninstall -g).
# Uses `mise exec node --` prefix to ensure the mise-managed npm binary is
# invoked even when mise is not activated in the caller's shell.
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

# _Npm-Cmd [Args...]
# Run an npm command, preferring the mise-managed npm when mise is available.
function _Npm-Cmd {
    param([Parameter(ValueFromRemainingArguments=$true)] $NpmArgs)
    if (Get-Command mise -ErrorAction SilentlyContinue) {
        & mise exec node -- npm @NpmArgs
    } else {
        & npm @NpmArgs
    }
}

# Backend-ManagerExists
# Return $true if npm is available (either directly or via mise).
function Backend-ManagerExists {
    try {
        if (Get-Command mise -ErrorAction SilentlyContinue) {
            & mise exec node -- npm --version 2>$null | Out-Null
            return $LASTEXITCODE -eq 0
        } else {
            return [bool](Get-Command npm -ErrorAction SilentlyContinue)
        }
    } catch { return $false }
}

# Backend-PackageExists <Name>
# Return $true if the named npm global package is installed.
function Backend-PackageExists {
    param([Parameter(Mandatory=$true)] [string]$Name)
    try {
        _Npm-Cmd list -g $Name 2>$null | Out-Null
        return $LASTEXITCODE -eq 0
    } catch { return $false }
}

# Backend-InstallPackage <Name> [<Version>]
# Install an npm global package. Version pinning is not supported; ignored if provided.
function Backend-InstallPackage {
    param(
        [Parameter(Mandatory=$true)] [string]$Name,
        [string]$Version = ""
    )

    if (-not (Backend-ManagerExists)) {
        Log-Error "npm backend: npm not available (is node installed via mise?)"
        throw "npm not available"
    }

    Log-Info "npm: installing global package: $Name"
    _Npm-Cmd install -g $Name
    if ($LASTEXITCODE -ne 0) {
        Log-Error "npm backend: failed to install: $Name"
        throw "npm install failed: $Name"
    }
    Log-Info "npm: installed: $Name"
}

# Backend-UninstallPackage <Name>
# Remove an npm global package. No-ops if the package is not installed.
function Backend-UninstallPackage {
    param([Parameter(Mandatory=$true)] [string]$Name)

    if (-not (Backend-ManagerExists)) {
        Log-Warn "npm backend: npm not available; skipping uninstall of $Name"
        return
    }

    if (-not (Backend-PackageExists -Name $Name)) {
        Log-Info "npm: package not installed, skipping: $Name"
        return
    }

    Log-Info "npm: uninstalling global package: $Name"
    _Npm-Cmd uninstall -g $Name
    if ($LASTEXITCODE -ne 0) {
        Log-Error "npm backend: failed to uninstall: $Name"
        throw "npm uninstall failed: $Name"
    }
    Log-Info "npm: uninstalled: $Name"
}

# Runtime management — not supported by npm backend.
function Backend-InstallRuntime   { Log-Error "npm backend: runtime management not supported"; throw "not supported" }
function Backend-UninstallRuntime { Log-Error "npm backend: runtime management not supported"; throw "not supported" }
function Backend-RuntimeExists    { return $false }
