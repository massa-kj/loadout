#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Backend plugin: brew
#
# Adapter for Homebrew (Linux / macOS / WSL).
# Implements the Backend Plugin Contract defined in CORE.md.
#
# Capabilities: package
# Does NOT support: runtime (use mise backend)
# -----------------------------------------------------------------------------

# This plugin expects core/lib/logger.sh to be available in the calling shell.

# backend_api_version
# Required by Backend Plugin Contract.
backend_api_version() { echo "1"; }

# backend_capabilities
# Informational: operations this backend supports.
backend_capabilities() { echo "package"; }

# ── Internal helpers ──────────────────────────────────────────────────────────

# _brew_ensure
# Make brew available in PATH. Returns 1 if not found.
_brew_ensure() {
    if command -v brew >/dev/null 2>&1; then
        return 0
    fi
    if [[ -x "/home/linuxbrew/.linuxbrew/bin/brew" ]]; then
        eval "$(/home/linuxbrew/.linuxbrew/bin/brew shellenv)" 2>/dev/null
        return 0
    fi
    if [[ -x "/opt/homebrew/bin/brew" ]]; then
        eval "$(/opt/homebrew/bin/brew shellenv)" 2>/dev/null
        return 0
    fi
    log_error "brew backend: Homebrew not found"
    return 1
}

# ── Observation API (plan use) ────────────────────────────────────────────────

# backend_manager_exists
# Return 0 if Homebrew is available, 1 otherwise.
backend_manager_exists() {
    _brew_ensure >/dev/null 2>&1
}

# backend_package_exists <name> [version]
# Return 0 if the named package is installed via Homebrew.
backend_package_exists() {
    local name="$1"
    # version="${2:-}" # brew does not support arbitrary version pinning for formulae
    _brew_ensure || return 1
    brew list --formula --versions "$name" >/dev/null 2>&1
}

# backend_runtime_exists <name> <version>
# Runtimes are not managed by brew; always returns 1.
backend_runtime_exists() { return 1; }

# ── Execution API ─────────────────────────────────────────────────────────────

# backend_install_package <name> [version]
# Install a Homebrew formula. Version argument is accepted but ignored
# (Homebrew formulae install the latest). Idempotent.
backend_install_package() {
    local name="$1"
    # version="${2:-null}"  # brew does not support pinning via CLI for most formulae

    _brew_ensure || return 1

    if backend_package_exists "$name"; then
        log_info "brew: package already installed: $name"
        return 0
    fi

    log_info "brew: installing package: $name"
    brew install "$name"
}

# backend_uninstall_package <name> [version]
# Uninstall a Homebrew formula. Idempotent.
backend_uninstall_package() {
    local name="$1"

    _brew_ensure || return 1

    if ! backend_package_exists "$name"; then
        log_info "brew: package not installed: $name"
        return 0
    fi

    log_info "brew: uninstalling package: $name"
    brew uninstall "$name"
}

# backend_install_runtime <name> <version>
# NOT SUPPORTED by brew backend.
backend_install_runtime() {
    log_error "brew: runtime installation is not supported; use the mise backend"
    return 1
}

# backend_uninstall_runtime <name> [version]
# NOT SUPPORTED by brew backend.
backend_uninstall_runtime() {
    log_error "brew: runtime uninstallation is not supported; use the mise backend"
    return 1
}
