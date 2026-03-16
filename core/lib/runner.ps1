# -----------------------------------------------------------------------------
# Module: runner
#
# Responsibility:
#   Provide command execution utilities and helpers.
#
# Public API (Stable):
#   Test-Command <Command>
#   Assert-Command <Command>
#   Invoke-OrDie <ScriptBlock> [Description]
#   Test-Administrator
#   Assert-Administrator
#   Invoke-WithRetry <ScriptBlock> [MaxAttempts] [DelaySeconds]
#   Get-UserConfirmation <Message> [DefaultYes]
# -----------------------------------------------------------------------------

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# This library expects logger.ps1 to be loaded by the caller.

# Test-Command <Command>
# Check if a command exists in PATH.
function Test-Command {
    param(
        [Parameter(Mandatory=$true)]
        [string]$Command
    )
    
    return $null -ne (Get-Command $Command -ErrorAction SilentlyContinue)
}

# Assert-Command <Command>
# Ensure a command exists, throw exception if not found.
function Assert-Command {
    param(
        [Parameter(Mandatory=$true)]
        [string]$Command
    )
    
    if (-not (Test-Command -Command $Command)) {
        Log-Error "Required command not found: $Command"
        throw "Required command not found: $Command"
    }
}

# Invoke-OrDie <ScriptBlock> [Description]
# Execute command and throw exception on failure.
function Invoke-OrDie {
    param(
        [Parameter(Mandatory=$true)]
        [scriptblock]$ScriptBlock,
        [string]$Description
    )
    
    if ($Description) {
        Log-Info "Running: $Description"
    }
    
    try {
        & $ScriptBlock
        if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne $null) {
            throw "Command failed with exit code $LASTEXITCODE"
        }
    } catch {
        Log-Error "Command failed: $_"
        throw
    }
}

# Test-Administrator
# Check if running with administrator privileges.
function Test-Administrator {
    $currentPrincipal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
    return $currentPrincipal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

# Assert-Administrator
# Ensure administrator privileges, throw exception if not running as admin.
function Assert-Administrator {
    if (-not (Test-Administrator)) {
        Log-Error "This operation requires Administrator privileges"
        Log-Info "Please restart PowerShell as Administrator"
        throw "Administrator privileges required"
    }
}

# Invoke-WithRetry <ScriptBlock> [MaxAttempts] [DelaySeconds]
# Execute command with automatic retry logic on failure.
function Invoke-WithRetry {
    param(
        [Parameter(Mandatory=$true)]
        [scriptblock]$ScriptBlock,
        [int]$MaxAttempts = 3,
        [int]$DelaySeconds = 2
    )
    
    $attempt = 1
    while ($attempt -le $MaxAttempts) {
        try {
            & $ScriptBlock
            return $true
        } catch {
            if ($attempt -eq $MaxAttempts) {
                Log-Error "Failed after $MaxAttempts attempts: $_"
                throw
            }
            
            Log-Warn "Attempt $attempt failed, retrying in ${DelaySeconds}s..."
            Start-Sleep -Seconds $DelaySeconds
            $attempt++
        }
    }
}

# Get-UserConfirmation <Message> [DefaultYes]
# Prompt user for yes/no confirmation and return boolean result.
function Get-UserConfirmation {
    param(
        [Parameter(Mandatory=$true)]
        [string]$Message,
        [bool]$DefaultYes = $false
    )
    
    $prompt = if ($DefaultYes) { "$Message [Y/n]" } else { "$Message [y/N]" }
    Write-Host $prompt -NoNewline
    
    $response = Read-Host
    
    if ([string]::IsNullOrWhiteSpace($response)) {
        return $DefaultYes
    }
    
    return $response -match '^[Yy]'
}
