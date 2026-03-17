#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# tests/unit/helpers.sh
#
# Shared test harness for unit tests.
#
# Usage:
#   source "$(dirname "${BASH_SOURCE[0]}")/helpers.sh"
#
# Provides:
#   Logger stubs   : log_error / log_warn / log_info / log_success
#   Counters       : _PASS / _FAIL  (initialized to 0)
#   Assertions     : _assert_eq / _assert_contains / _assert_return0 / _assert_return1
#   Summary        : _print_summary (prints results + exits 1 on any failure)
# -----------------------------------------------------------------------------

# ── Logger stubs (no colour; write to stderr) ─────────────────────────────────

log_error()   { echo "[ERROR] $*" >&2; }
log_warn()    { echo "[WARN]  $*" >&2; }
log_info()    { echo "[INFO]  $*" >&2; }
log_success() { echo "[OK]    $*" >&2; }

# ── Counters ──────────────────────────────────────────────────────────────────

_PASS=0
_FAIL=0

# ── Assertions ────────────────────────────────────────────────────────────────

# _assert_eq <test_name> <expected> <actual>
_assert_eq() {
    local name="$1" expected="$2" actual="$3"
    if [[ "$expected" == "$actual" ]]; then
        echo "  PASS  $name"
        (( _PASS++ )) || true
    else
        echo "  FAIL  $name"
        echo "        expected: '$expected'"
        echo "        actual:   '$actual'"
        (( _FAIL++ )) || true
    fi
}

# _assert_contains <test_name> <needle> <haystack>
# Passes when <needle> appears as a space-delimited word in <haystack>.
_assert_contains() {
    local name="$1" needle="$2" haystack="$3"
    if [[ " $haystack " == *" $needle "* ]]; then
        echo "  PASS  $name"
        (( _PASS++ )) || true
    else
        echo "  FAIL  $name"
        echo "        expected '$needle' in: '$haystack'"
        (( _FAIL++ )) || true
    fi
}

# _assert_return0 <test_name> <command...>
# Passes when the command exits 0.
_assert_return0() {
    local name="$1"; shift
    if "$@" > /dev/null 2>&1; then
        echo "  PASS  $name"
        (( _PASS++ )) || true
    else
        echo "  FAIL  $name (expected exit 0, got non-zero)"
        (( _FAIL++ )) || true
    fi
}

# _assert_return1 <test_name> <command...>
# Passes when the command exits non-zero.
_assert_return1() {
    local name="$1"; shift
    if "$@" > /dev/null 2>&1; then
        echo "  FAIL  $name (expected non-zero, got exit 0)"
        (( _FAIL++ )) || true
    else
        echo "  PASS  $name"
        (( _PASS++ )) || true
    fi
}

# ── Summary ───────────────────────────────────────────────────────────────────

# _print_summary
# Print pass/fail counts and exit 1 if any failures occurred.
_print_summary() {
    echo ""
    echo "Results: ${_PASS} passed, ${_FAIL} failed"
    if [[ "$_FAIL" -gt 0 ]]; then
        exit 1
    fi
}
