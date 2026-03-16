#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Backend plugin: uv
#
# Manages Python packages installed via `uv pip install --system`.
# Uses `mise exec --` prefix to ensure the mise-managed uv binary is invoked
# even when mise is not activated in the caller's shell.
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

# _uv_cmd [args...]
# Run a uv command, preferring the mise-managed uv binary when mise is available.
# Use `mise exec uv -- uv` rather than `mise exec -- uv` to activate ONLY the uv
# tool; `mise exec --` (no tool specified) activates ALL configured tools and
# triggers installation of any that are missing (including rust, rust-analyzer, etc.).
_uv_cmd() {
    if command -v mise >/dev/null 2>&1; then
        mise exec uv -- uv "$@"
    else
        uv "$@"
    fi
}

# backend_manager_exists
# Return 0 if uv is available (either directly or via mise).
backend_manager_exists() {
    if command -v mise >/dev/null 2>&1; then
        mise exec uv -- uv --version >/dev/null 2>&1
    else
        command -v uv >/dev/null 2>&1
    fi
}

# backend_package_exists <name>
# Return 0 if the named package is installed in the system Python environment.
backend_package_exists() {
    local name="$1"
    _uv_cmd pip list 2>/dev/null | grep -qi "^${name}[[:space:]]"
}

# backend_install_package <name> [version]
# Install a package into the system Python environment via uv.
# If version is provided, installs name==version.
backend_install_package() {
    local name="$1"
    local version="${2:-}"

    backend_manager_exists || {
        log_error "uv backend: uv not available (is python installed via mise?)"
        return 1
    }

    local spec="$name"
    [[ -n "$version" ]] && spec="${name}==${version}"

    log_info "uv: installing package: $spec"
    _uv_cmd pip install --system "$spec" || {
        log_error "uv backend: failed to install: $spec"
        return 1
    }
    log_info "uv: installed: $spec"
}

# backend_uninstall_package <name>
# Remove a package from the system Python environment. No-ops if not installed.
backend_uninstall_package() {
    local name="$1"

    backend_manager_exists || {
        log_warn "uv backend: uv not available; skipping uninstall of $name"
        return 0
    }

    if ! backend_package_exists "$name"; then
        log_info "uv: package not installed, skipping: $name"
        return 0
    fi

    log_info "uv: uninstalling package: $name"
    _uv_cmd pip uninstall --system "$name" || {
        log_error "uv backend: failed to uninstall: $name"
        return 1
    }
    log_info "uv: uninstalled: $name"
}

# Runtime management — not supported by uv backend.
backend_install_runtime()   { log_error "uv backend: runtime management not supported"; return 1; }
backend_uninstall_runtime() { log_error "uv backend: runtime management not supported"; return 1; }
backend_runtime_exists()    { return 1; }
