#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Module: backend_registry
#
# Responsibility:
#   Resolve, load, and dispatch backend plugin operations.
#   Acts as the sole stable gate between core and backend execution adapters.
#
# Public API (Stable):
#   backend_registry_load_policy [policy_file]
#   resolve_backend_for <kind> <name>
#   load_backend <backend_id>
#   backend_call <op> <args...>
# -----------------------------------------------------------------------------

# This library expects core/lib/env.sh and core/lib/logger.sh to be sourced by the caller.

if [[ "$(type -t canonical_id_normalize)" != "function" ]]; then
    # shellcheck source=core/lib/source_registry.sh
    source "${LOADOUT_ROOT}/core/lib/source_registry.sh"
fi

# ── Private state ─────────────────────────────────────────────────────────────

# Cached raw content of the loaded policy file.
declare -g _BR_POLICY_DATA=""

# ID of the currently loaded backend plugin (for source-once optimisation).
declare -g _BR_LOADED_BACKEND=""

# ── Policy loading ────────────────────────────────────────────────────────────

# backend_registry_load_policy [policy_file]
# Load policy YAML into memory cache.
# Uses LOADOUT_POLICY_FILE if no argument is given.
# Idempotent: calling multiple times with the same file is safe.
# Non-fatal if the file does not exist – caller falls back to platform defaults.
backend_registry_load_policy() {
    local policy_file="${1:-${LOADOUT_POLICY_FILE:-}}"

    if [[ -z "$policy_file" ]]; then
        return 0  # No policy configured; will use platform defaults.
    fi

    if [[ ! -f "$policy_file" ]]; then
        log_warn "backend_registry_load_policy: policy file not found: $policy_file (using platform defaults)"
        return 0
    fi

    _BR_POLICY_DATA="$(cat "$policy_file")"
    return 0
}

# ── Backend resolution ────────────────────────────────────────────────────────

# resolve_backend_for <kind> <name>
# Output the backend_id to use for the given kind/name pair.
# Resolution order:
#   1. Policy overrides: .<kind>.overrides.<name>.backend
#   2. Policy default:   .<kind>.default_backend
#   3. Platform default (hardcoded)
#
# kind: "package" | "runtime"
# name: resource name (package name or runtime name) used for override lookup
resolve_backend_for() {
    local kind="$1"
    local name="$2"

    if [[ -z "$kind" ]] || [[ -z "$name" ]]; then
        log_error "resolve_backend_for: kind and name are required"
        return 1
    fi

    # Lazy-load policy on first call
    if [[ -z "$_BR_POLICY_DATA" ]] && [[ -n "${LOADOUT_POLICY_FILE:-}" ]]; then
        backend_registry_load_policy || true  # Non-fatal
    fi

    if [[ -n "$_BR_POLICY_DATA" ]]; then
        local backend_id

        case "$kind" in
            package|runtime)
                # 1. Per-resource override (keyed by resource name, not feature name)
                # Note: yq v4 returns the string "null" for missing keys.
                # "// empty" is jq syntax and causes a parse error in yq v4.
                backend_id=$(echo "$_BR_POLICY_DATA" | \
                    yq eval ".${kind}.overrides.\"${name}\".backend" - 2>/dev/null)
                if [[ -n "$backend_id" && "$backend_id" != "null" ]]; then
                    canonical_id_normalize "$backend_id" "core"
                    return 0
                fi
                # 2. Kind-level default
                backend_id=$(echo "$_BR_POLICY_DATA" | \
                    yq eval ".${kind}.default_backend" - 2>/dev/null)
                if [[ -n "$backend_id" && "$backend_id" != "null" ]]; then
                    canonical_id_normalize "$backend_id" "core"
                    return 0
                fi
                ;;
            *)
                log_error "resolve_backend_for: unsupported kind: $kind"
                return 1
                ;;
        esac
    fi

    # Platform-based defaults (used when policy file absent or incomplete)
    _backend_registry_platform_default "$kind"
}

# _backend_registry_platform_default <kind>
# Output the hardcoded platform default backend_id for the given kind.
_backend_registry_platform_default() {
    local kind="$1"

    case "${LOADOUT_PLATFORM:-linux}" in
        linux|wsl)
            case "$kind" in
                package) echo "core/brew" ;;
                runtime) echo "core/mise" ;;
                *)
                    log_error "_backend_registry_platform_default: unsupported kind: $kind"
                    return 1
                    ;;
            esac
            ;;
        windows)
            case "$kind" in
                package) echo "core/scoop" ;;
                runtime) echo "core/mise" ;;
                *)
                    log_error "_backend_registry_platform_default: unsupported kind: $kind"
                    return 1
                    ;;
            esac
            ;;
        *)
            log_error "_backend_registry_platform_default: unsupported platform: ${LOADOUT_PLATFORM:-unknown}"
            return 1
            ;;
    esac
}

# ── Plugin loading ────────────────────────────────────────────────────────────

# load_backend <backend_id>
# Source the backend plugin file and validate the Backend Plugin Contract.
# Sets the active backend: subsequent backend_call() calls operate on this backend.
# Idempotent for the same backend_id (re-sourcing is skipped).
load_backend() {
    local backend_id="$1"

    if [[ -z "$backend_id" ]]; then
        log_error "load_backend: backend_id is required"
        return 1
    fi

    local canonical_backend_id
    canonical_backend_id=$(canonical_id_normalize "$backend_id" "core") || {
        log_error "load_backend: invalid backend id: $backend_id"
        return 1
    }

    local source_id backend_name
    canonical_id_parse "$canonical_backend_id" source_id backend_name || return 1
    source_registry_load || return 1

    if ! source_registry_is_backend_allowed "$source_id" "$backend_name"; then
        log_error "load_backend: backend is not allowed by source registry: $canonical_backend_id"
        return 1
    fi

    # Skip re-sourcing if the same backend is already loaded
    if [[ "$_BR_LOADED_BACKEND" == "$canonical_backend_id" ]]; then
        return 0
    fi

    local backend_dir
    backend_dir=$(source_registry_get_backend_dir "$source_id") || return 1

    local plugin_file="${backend_dir}/${backend_name}.sh"
    if [[ ! -f "$plugin_file" ]]; then
        log_error "load_backend: plugin not found: $plugin_file"
        return 1
    fi

    # shellcheck disable=SC1090
    source "$plugin_file"

    # Validate minimum Backend Plugin Contract
    if ! declare -F "backend_api_version" >/dev/null 2>&1; then
        log_error "load_backend: contract violation: backend_api_version not defined (plugin: $plugin_file)"
        _BR_LOADED_BACKEND=""
        return 1
    fi

    _BR_LOADED_BACKEND="$canonical_backend_id"
    log_info "load_backend: loaded backend plugin: $canonical_backend_id (api_version=$(backend_api_version))"
    return 0
}

# ── Dispatch ──────────────────────────────────────────────────────────────────

# backend_call <op> <args...>
# Call an operation on the currently loaded backend plugin.
# Requires load_backend to have been called first.
#
# op: the function suffix (e.g. "install_package" → calls backend_install_package)
# args: forwarded as-is to the backend function
backend_call() {
    local op="$1"
    shift

    if [[ -z "$op" ]]; then
        log_error "backend_call: op is required"
        return 1
    fi

    if [[ -z "$_BR_LOADED_BACKEND" ]]; then
        log_error "backend_call: no backend loaded; call load_backend first"
        return 1
    fi

    local func="backend_${op}"
    if ! declare -F "$func" >/dev/null 2>&1; then
        log_error "backend_call: operation not defined: ${func} (loaded backend: $_BR_LOADED_BACKEND)"
        return 1
    fi

    "$func" "$@"
}
