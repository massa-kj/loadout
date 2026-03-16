# -----------------------------------------------------------------------------
# Module: repo
#
# Responsibility:
#   Provide repository-based tool installation utilities.
#   Manages cloning of source repositories and path resolution for
#   locally installed tools (not managed by a package manager).
#
# Convention:
#   Source repositories are cloned under $env:USERPROFILE\.local\src\<tool>
#   Tool binaries are placed under $env:USERPROFILE\.local\bin\<tool>
#
# Public API (Stable):
#   Clone-Repository <Feature> <RepoUrl> <DestPath>
#   Resolve-ToolPath <ToolName>
#   Test-ToolInstalled <ToolName>
# -----------------------------------------------------------------------------

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# This library expects logger.ps1, state.ps1, and runner.ps1
# to be loaded by the caller.

# Clone-Repository <Feature> <RepoUrl> <DestPath>
# Clone a git repository to DestPath, or pull if it already exists.
# Registers the destination directory to feature state for uninstall tracking.
function Clone-Repository {
    param(
        [Parameter(Mandatory=$true)]
        [string]$Feature,

        [Parameter(Mandatory=$true)]
        [string]$RepoUrl,

        [Parameter(Mandatory=$true)]
        [string]$DestPath
    )

    Assert-Command "git"

    if (Test-Path (Join-Path $DestPath ".git")) {
        Log-Info "Repository already cloned, pulling: $DestPath"
        try {
            & git -C $DestPath pull --ff-only
            if ($LASTEXITCODE -ne 0) {
                throw "git pull failed"
            }
        } catch {
            Log-Error "Clone-Repository: git pull failed: $DestPath"
            throw
        }
    } else {
        $parentDir = Split-Path -Parent $DestPath
        if (-not (Test-Path $parentDir)) {
            New-Item -ItemType Directory -Path $parentDir -Force | Out-Null
        }

        Log-Info "Cloning $RepoUrl into $DestPath"
        try {
            & git clone $RepoUrl $DestPath
            if ($LASTEXITCODE -ne 0) {
                throw "git clone failed"
            }
        } catch {
            Log-Error "Clone-Repository: git clone failed: $RepoUrl"
            throw
        }
    }

    State-AddFile -Feature $Feature -File $DestPath
    Log-Success "Repository ready: $DestPath"
}

# Resolve-ToolPath <ToolName>
# Returns the canonical install path for a locally managed tool binary.
# Output: $env:USERPROFILE\.local\bin\<ToolName>
function Resolve-ToolPath {
    param(
        [Parameter(Mandatory=$true)]
        [string]$ToolName
    )

    return Join-Path $env:USERPROFILE ".local\bin\$ToolName"
}

# Test-ToolInstalled <ToolName>
# Check if a tool exists at the local install path (~\.local\bin\<ToolName>).
# Returns $true if installed, $false otherwise.
function Test-ToolInstalled {
    param(
        [Parameter(Mandatory=$true)]
        [string]$ToolName
    )

    $toolPath = Resolve-ToolPath -ToolName $ToolName
    return (Test-Path $toolPath)
}
