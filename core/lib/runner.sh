#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Module: runner
#
# Responsibility:
#   Provide command execution utilities and helpers.
#
# Public API (Stable):
#   has_command <cmd>
#   require_command <cmd>
#   run_or_die <command...>
#   ensure_sudo
# -----------------------------------------------------------------------------

# This library expects core/lib/logger.sh to be sourced by the caller.

# has_command <cmd>
# Check if a command exists in PATH.
has_command() {
    local cmd="$1"

    if [[ -z "$cmd" ]]; then
        log_error "has_command: command name is required"
        return 1
    fi

    command -v "$cmd" >/dev/null 2>&1
}

# require_command <cmd>
# Ensure a command exists, exit with error if not found.
require_command() {
    local cmd="$1"

    if [[ -z "$cmd" ]]; then
        log_error "require_command: command name is required"
        return 1
    fi

    if ! has_command "$cmd"; then
        log_error "Required command not found: $cmd"
        return 1
    fi
}

# run_or_die <command...>
# Execute command and exit on failure.
run_or_die() {
    if [[ $# -eq 0 ]]; then
        log_error "run_or_die: command is required"
        return 1
    fi

    log_info "Running: $*"
    "$@"
    local status=$?
    if [[ $status -ne 0 ]]; then
        log_error "Command failed ($status): $*"
        return $status
    fi
}

# ensure_sudo
# Request sudo privileges if on Linux/WSL platforms.
ensure_sudo() {
    if [[ "${LOADOUT_PLATFORM:-}" == "linux" || "${LOADOUT_PLATFORM:-}" == "wsl" ]]; then
        if ! sudo -n true 2>/dev/null; then
            log_info "Requesting sudo access..."
            sudo -v
        fi
    fi
}