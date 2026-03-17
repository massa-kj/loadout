# -----------------------------------------------------------------------------
# tests/unit/helpers.ps1
#
# Shared test harness for PowerShell unit tests.
#
# Usage:
#   . "$PSScriptRoot\helpers.ps1"
#
# Provides:
#   Logger stubs   : Log-Error / Log-Warn / Log-Info / Log-Success / Log-Task
#   Counters       : $script:PassCount / $script:FailCount
#   Assertions     : Assert-Equal / Assert-Contains / Assert-Return0 / Assert-Return1
#   Summary        : Show-TestSummary (prints results + exits 1 on any failure)
# -----------------------------------------------------------------------------

Set-StrictMode -Version Latest

# ── PS 5.1 compatibility helpers ─────────────────────────────────────────────
# Mirrors the definitions in core/lib/env.ps1 for standalone test execution.
function _Prop { param($Obj, $Key) $p = $Obj.PSObject.Properties[$Key]; if ($null -ne $p) { $p.Value } else { $null } }
function _Coal { param($a, $b) if ($null -ne $a) { $a } else { $b } }

# ── Logger stubs (no colour; write to stderr) ─────────────────────────────────

function Log-Error   { param([string]$Msg) [Console]::Error.WriteLine("[ERROR] $Msg") }
function Log-Warn    { param([string]$Msg) [Console]::Error.WriteLine("[WARN]  $Msg") }
function Log-Info    { param([string]$Msg) [Console]::Error.WriteLine("[INFO]  $Msg") }
function Log-Success { param([string]$Msg) [Console]::Error.WriteLine("[OK]    $Msg") }
function Log-Task    { param([string]$Msg) [Console]::Error.WriteLine("[TASK]  $Msg") }

# ── Counters ──────────────────────────────────────────────────────────────────

$script:PassCount = 0
$script:FailCount = 0

# ── Assertions ────────────────────────────────────────────────────────────────

# Assert-Equal <TestName> <Expected> <Actual>
function Assert-Equal {
    param(
        [string]$TestName,
        [string]$Expected,
        [string]$Actual
    )
    if ($Expected -eq $Actual) {
        Write-Host "  PASS  $TestName"
        $script:PassCount++
    } else {
        Write-Host "  FAIL  $TestName"
        Write-Host "        expected: '$Expected'"
        Write-Host "        actual:   '$Actual'"
        $script:FailCount++
    }
}

# Assert-Contains <TestName> <Needle> <Haystack>
# Passes when Needle appears as an element in Haystack (string array or space-delimited string).
function Assert-Contains {
    param(
        [string]$TestName,
        [string]$Needle,
        [string]$Haystack
    )
    if ((" $Haystack ") -like "* $Needle *") {
        Write-Host "  PASS  $TestName"
        $script:PassCount++
    } else {
        Write-Host "  FAIL  $TestName"
        Write-Host "        expected '$Needle' in: '$Haystack'"
        $script:FailCount++
    }
}

# Assert-True <TestName> <Condition>
function Assert-True {
    param(
        [string]$TestName,
        [bool]$Condition
    )
    if ($Condition) {
        Write-Host "  PASS  $TestName"
        $script:PassCount++
    } else {
        Write-Host "  FAIL  $TestName (expected $true)"
        $script:FailCount++
    }
}

# Assert-False <TestName> <Condition>
function Assert-False {
    param(
        [string]$TestName,
        [bool]$Condition
    )
    if (-not $Condition) {
        Write-Host "  PASS  $TestName"
        $script:PassCount++
    } else {
        Write-Host "  FAIL  $TestName (expected $false)"
        $script:FailCount++
    }
}

# Assert-NotNull <TestName> <Value>
function Assert-NotNull {
    param(
        [string]$TestName,
        [object]$Value
    )
    if ($null -ne $Value) {
        Write-Host "  PASS  $TestName"
        $script:PassCount++
    } else {
        Write-Host "  FAIL  $TestName (expected non-null)"
        $script:FailCount++
    }
}

# Assert-Null <TestName> <Value>
function Assert-Null {
    param(
        [string]$TestName,
        [object]$Value
    )
    if ($null -eq $Value) {
        Write-Host "  PASS  $TestName"
        $script:PassCount++
    } else {
        Write-Host "  FAIL  $TestName (expected null, got: '$Value')"
        $script:FailCount++
    }
}

# ── Summary ───────────────────────────────────────────────────────────────────

function Show-TestSummary {
    Write-Host ""
    Write-Host "Results: $($script:PassCount) passed, $($script:FailCount) failed"
    if ($script:FailCount -gt 0) { exit 1 }
}
