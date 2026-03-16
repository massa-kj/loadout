#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Backend plugin: npm
#
# Manages npm global packages (npm install -g / npm uninstall -g).
# Uses `mise exec node --` prefix to ensure the mise-managed npm binary is
# invoked even when mise is not activated in the caller's shell.
#
# Backend Plugin Contract (CORE.md §Backend Plugin Contract):
#   backend_api_version
#   backend_manager_exists
#   backend_package_exists <name>
#   backend_install_package <name> [version]
#   backend_uninstall_package <name>
#   backend_install_runtime   — not supported
#   backend_uninstall_runtime — not supported
#   backend_runtime_exists    — not supported
# -----------------------------------------------------------------------------

backend_api_version() { echo "1"; }

# _npm_cmd [args...]
# Run an npm command, preferring the mise-managed npm when mise is available.
_npm_cmd() {
    if command -v mise >/dev/null 2>&1; then
        mise exec node -- npm "$@"
    else
        npm "$@"
    fi
}

# backend_manager_exists
# Return 0 if npm is available (either directly or via mise).
backend_manager_exists() {
    if command -v mise >/dev/null 2>&1; then
        mise exec node -- npm --version >/dev/null 2>&1
    else
        command -v npm >/dev/null 2>&1
    fi
}

# backend_package_exists <name>
# Return 0 if the named npm global package is installed.
# Uses the global node_modules directory rather than `npm list` exit code,
# because `npm list` can return exit 1 due to peer dep issues even when
# the queried package itself is present.
backend_package_exists() {
    local name="$1"
    local prefix
    prefix=$(_npm_cmd prefix -g 2>/dev/null) || return 1
    [[ -d "$prefix/lib/node_modules/$name" ]]
}

# backend_install_package <name> [version]
# Install an npm global package. Version pinning is not supported; ignored if provided.
backend_install_package() {
    local name="$1"
    # version="${2:-}"  # npm global installs do not reliably support version pinning

    backend_manager_exists || {
        log_error "npm backend: npm not available (is node installed via mise?)"
        return 1
    }

    log_info "npm: installing global package: $name"
    _npm_cmd install -g "$name" || {
        log_error "npm backend: failed to install: $name"
        return 1
    }
    log_info "npm: installed: $name"
}

# backend_uninstall_package <name>
# Remove an npm global package. No-ops if the package is not installed.
backend_uninstall_package() {
    local name="$1"

    backend_manager_exists || {
        log_warn "npm backend: npm not available; skipping uninstall of $name"
        return 0
    }

    if ! backend_package_exists "$name"; then
        log_info "npm: package not installed, skipping: $name"
        return 0
    fi

    log_info "npm: uninstalling global package: $name"
    _npm_cmd uninstall -g "$name" || {
        log_error "npm backend: failed to uninstall: $name"
        return 1
    }
    log_info "npm: uninstalled: $name"
}

# Runtime management — not supported by npm backend.
backend_install_runtime()   { log_error "npm backend: runtime management not supported"; return 1; }
backend_uninstall_runtime() { log_error "npm backend: runtime management not supported"; return 1; }
backend_runtime_exists()    { return 1; }
