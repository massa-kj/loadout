#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Backend plugin: mise
#
# Adapter for mise (https://mise.jdx.dev).
# Handles runtime installation for node, python, rust, lua, etc.
# Implements the Backend Plugin Contract defined in CORE.md.
#
# Capabilities: runtime
# Does NOT support: system package install (use brew backend)
# -----------------------------------------------------------------------------

# This plugin expects core/lib/logger.sh to be available in the calling shell.

# backend_api_version
# Required by Backend Plugin Contract.
backend_api_version() { echo "1"; }

# backend_capabilities
# Informational: operations this backend supports.
backend_capabilities() { echo "runtime"; }

# ── Internal helpers ──────────────────────────────────────────────────────────

# _mise_ensure
# Make mise available in PATH. Returns 1 if not found.
_mise_ensure() {
    if command -v mise >/dev/null 2>&1; then
        return 0
    fi

    local paths=(
        "$HOME/.local/bin/mise"
        "/home/linuxbrew/.linuxbrew/bin/mise"
        "/usr/local/bin/mise"
    )
    for p in "${paths[@]}"; do
        if [[ -x "$p" ]]; then
            export PATH="$(dirname "$p"):$PATH"
            eval "$("$p" activate bash)" 2>/dev/null
            return 0
        fi
    done

    log_error "mise backend: mise not found"
    return 1
}

# _mise_resolve_version <name> <version>
# Resolve a version alias (e.g. "latest", "22") to the concrete version string.
# Prints the resolved version to stdout.
_mise_resolve_version() {
    local name="$1"
    local version="$2"

    # Already a concrete version (all digits and dots)
    if [[ "$version" =~ ^[0-9]+(\.[0-9]+)*$ ]]; then
        echo "$version"
        return 0
    fi

    # Resolve via mise
    local resolved
    resolved=$(mise latest "${name}@${version}" 2>/dev/null)
    if [[ -n "$resolved" ]]; then
        echo "$resolved"
        return 0
    fi

    # Fall back to the alias as-is
    echo "$version"
}

# ── Observation API (plan use) ────────────────────────────────────────────────

# backend_manager_exists
# Return 0 if mise is available, 1 otherwise.
backend_manager_exists() {
    _mise_ensure >/dev/null 2>&1
}

# backend_package_exists <name> [version]
# System packages are not managed by mise; always returns 1.
backend_package_exists() { return 1; }

# backend_runtime_exists <name> <version>
# Return 0 if the named runtime version is installed via mise.
backend_runtime_exists() {
    local name="$1"
    local version="${2:-}"

    _mise_ensure || return 1

    if [[ -n "$version" ]]; then
        local resolved
        resolved=$(_mise_resolve_version "$name" "$version")
        mise where "${name}@${resolved}" >/dev/null 2>&1
    else
        mise ls --installed "$name" >/dev/null 2>&1
    fi
}

# ── Execution API ─────────────────────────────────────────────────────────────

# backend_install_package <name> [version]
# NOT SUPPORTED by mise backend.
backend_install_package() {
    log_error "mise: system package installation is not supported; use the brew backend"
    return 1
}

# backend_uninstall_package <name> [version]
# NOT SUPPORTED by mise backend.
backend_uninstall_package() {
    log_error "mise: system package uninstallation is not supported; use the brew backend"
    return 1
}

# backend_install_runtime <name> <version>
# Install a runtime via mise and set it as the global default.
# Prints the concrete resolved version to stdout.
# Idempotent: does nothing if the version is already installed.
backend_install_runtime() {
    local name="$1"
    local version="$2"

    if [[ -z "$name" ]] || [[ -z "$version" ]]; then
        log_error "mise: backend_install_runtime: name and version are required"
        return 1
    fi

    _mise_ensure || return 1

    local resolved
    resolved=$(_mise_resolve_version "$name" "$version")

    if backend_runtime_exists "$name" "$resolved"; then
        log_info "mise: runtime already installed: ${name}@${resolved}"
        echo "$resolved"
        return 0
    fi

    log_info "mise: installing runtime: ${name}@${resolved}"
    mise install "${name}@${resolved}" >&2 || {
        log_error "mise: install failed: ${name}@${resolved}"
        return 1
    }

    log_info "mise: setting global: ${name}@${resolved}"
    mise use -g "${name}@${resolved}" >&2

    # Add runtime's bin directory to PATH for the current session
    local runtime_path
    runtime_path=$(mise where "${name}@${resolved}" 2>/dev/null || true)
    if [[ -n "$runtime_path" ]] && [[ -d "$runtime_path/bin" ]]; then
        export PATH="$runtime_path/bin:$PATH"
    fi

    # Re-activate mise to refresh shims
    eval "$(mise activate bash)" >&2

    echo "$resolved"
}

# backend_uninstall_runtime <name> [version]
# Uninstall a runtime via mise. Idempotent.
backend_uninstall_runtime() {
    local name="$1"
    local version="${2:-}"

    _mise_ensure || return 1

    if [[ -n "$version" ]]; then
        if ! backend_runtime_exists "$name" "$version"; then
            log_info "mise: runtime not installed: ${name}@${version}"
            return 0
        fi
        log_info "mise: uninstalling runtime: ${name}@${version}"
        mise uninstall "${name}@${version}" || {
            log_error "mise: uninstall failed: ${name}@${version}"
            return 1
        }
    else
        if ! backend_runtime_exists "$name"; then
            log_info "mise: runtime not installed: $name"
            return 0
        fi
        log_info "mise: uninstalling runtime: $name"
        mise uninstall "$name" || {
            log_error "mise: uninstall failed: $name"
            return 1
        }
    fi
}
