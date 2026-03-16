#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Backend plugin: apt
#
# Adapter for Debian/Ubuntu APT package management.
#
# Capabilities: package
# Does NOT support: runtime (use the mise backend)
# -----------------------------------------------------------------------------

# This plugin expects core/lib/logger.sh to be available in the calling shell.

backend_api_version() { echo "1"; }

backend_capabilities() { echo "package"; }

# _apt_sudo_prefix
# Print the command prefix required to run apt commands.
# Uses direct execution as root, otherwise falls back to sudo if available.
_apt_sudo_prefix() {
    if [[ "${EUID:-$(id -u)}" -eq 0 ]]; then
        return 0
    fi

    if command -v sudo >/dev/null 2>&1; then
        echo "sudo"
        return 0
    fi

    log_error "apt backend: root privileges or sudo are required"
    return 1
}

# _apt_ensure
# Ensure apt-get and dpkg-query are available.
_apt_ensure() {
    if ! command -v apt-get >/dev/null 2>&1; then
        log_error "apt backend: apt-get not found"
        return 1
    fi
    if ! command -v dpkg-query >/dev/null 2>&1; then
        log_error "apt backend: dpkg-query not found"
        return 1
    fi
}

# backend_manager_exists
# Return 0 if APT is available.
backend_manager_exists() {
    _apt_ensure >/dev/null 2>&1
}

# backend_package_exists <name> [version]
# Return 0 if the named package is installed.
backend_package_exists() {
    local name="$1"
    _apt_ensure || return 1
    dpkg-query -W -f='${Status}' "$name" 2>/dev/null | grep -q '^install ok installed$'
}

# backend_runtime_exists <name> <version>
# Runtimes are not managed by apt; always returns 1.
backend_runtime_exists() { return 1; }

# backend_install_package <name> [version]
# Install a package via apt-get. Version is accepted but ignored.
backend_install_package() {
    local name="$1"

    _apt_ensure || return 1

    if backend_package_exists "$name"; then
        log_info "apt: package already installed: $name"
        return 0
    fi

    local sudo_prefix
    sudo_prefix=$(_apt_sudo_prefix) || return 1

    log_info "apt: updating package index"
    if [[ -n "$sudo_prefix" ]]; then
        DEBIAN_FRONTEND=noninteractive "$sudo_prefix" apt-get update -y
    else
        DEBIAN_FRONTEND=noninteractive apt-get update -y
    fi

    log_info "apt: installing package: $name"
    if [[ -n "$sudo_prefix" ]]; then
        DEBIAN_FRONTEND=noninteractive "$sudo_prefix" apt-get install -y "$name"
    else
        DEBIAN_FRONTEND=noninteractive apt-get install -y "$name"
    fi
}

# backend_uninstall_package <name> [version]
# Remove a package via apt-get. Idempotent.
backend_uninstall_package() {
    local name="$1"

    _apt_ensure || return 1

    if ! backend_package_exists "$name"; then
        log_info "apt: package not installed: $name"
        return 0
    fi

    local sudo_prefix
    sudo_prefix=$(_apt_sudo_prefix) || return 1

    log_info "apt: removing package: $name"
    if [[ -n "$sudo_prefix" ]]; then
        DEBIAN_FRONTEND=noninteractive "$sudo_prefix" apt-get remove -y "$name"
    else
        DEBIAN_FRONTEND=noninteractive apt-get remove -y "$name"
    fi
}

# backend_install_runtime <name> <version>
# NOT SUPPORTED by apt backend.
backend_install_runtime() {
    log_error "apt: runtime installation is not supported; use the mise backend"
    return 1
}

# backend_uninstall_runtime <name> [version]
# NOT SUPPORTED by apt backend.
backend_uninstall_runtime() {
    log_error "apt: runtime uninstallation is not supported; use the mise backend"
    return 1
}
