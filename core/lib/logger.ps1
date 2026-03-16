# -----------------------------------------------------------------------------
# Module: logger
#
# Responsibility:
#   Provide logging functions with color-coded output.
#
# Public API (Stable):
#   Log-Debug <Message>
#   Log-Info <Message>
#   Log-Success <Message>
#   Log-Warn <Message>
#   Log-Error <Message>
#   Log-Task <Message>
# -----------------------------------------------------------------------------

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Color support check
$script:SupportsColor = $Host.UI.SupportsVirtualTerminal

# Color codes (ANSI escape sequences)
$script:COLOR_RESET = "`e[0m"
$script:COLOR_RED = "`e[0;31m"
$script:COLOR_GREEN = "`e[0;32m"
$script:COLOR_YELLOW = "`e[0;33m"
$script:COLOR_BLUE = "`e[0;34m"
$script:COLOR_GRAY = "`e[0;90m"

function Write-ColorLog {
    param(
        [string]$Message,
        [string]$Color,
        [string]$Level
    )
    
    if ($script:SupportsColor) {
        Write-Host "${Color}[${Level}] ${Message}${script:COLOR_RESET}"
    } else {
        Write-Host "[${Level}] ${Message}"
    }
}

# Log-Debug <Message>
# Output debug level log message.
function Log-Debug {
    param([string]$Message)
    Write-ColorLog -Message $Message -Color $script:COLOR_GRAY -Level "DEBUG"
}

# Log-Info <Message>
# Output info level log message.
function Log-Info {
    param([string]$Message)
    Write-ColorLog -Message $Message -Color $script:COLOR_BLUE -Level "INFO"
}

# Log-Success <Message>
# Output success level log message.
function Log-Success {
    param([string]$Message)
    Write-ColorLog -Message $Message -Color $script:COLOR_GREEN -Level "SUCCESS"
}

# Log-Warn <Message>
# Output warning level log message.
function Log-Warn {
    param([string]$Message)
    Write-ColorLog -Message $Message -Color $script:COLOR_YELLOW -Level "WARN"
}

# Log-Error <Message>
# Output error level log message.
function Log-Error {
    param([string]$Message)
    Write-ColorLog -Message $Message -Color $script:COLOR_RED -Level "ERROR"
}

# Log-Task <Message>
# Output task execution marker for start/end of processing.
function Log-Task {
    param([string]$Message)
    
    if ($script:SupportsColor) {
        Write-Host "${script:COLOR_GREEN}==>${script:COLOR_RESET} ${Message}"
    } else {
        Write-Host "==> ${Message}"
    }
}
