#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Module: orchestrator
#
# Responsibility:
#   Orchestrate feature installation and uninstallation workflow.
#   Owns the full apply pipeline: profile read → diff → execute → summarise.
#
# Public API:
#   orchestrator_apply <profile_file>   — run full apply pipeline
#
# Internal helpers (also usable by tests):
#   read_profile <profile_file> <output_array>
#   extract_feature_config <feature>
#   check_version_mismatch <feature>
#   calculate_diff <sorted> <to_install> <to_uninstall> <to_reinstall>
#   run_uninstall <features>
#   run_install <features>
#   print_summary
# -----------------------------------------------------------------------------

# This library expects env.sh, logger.sh, and state.sh to be sourced by the caller.

# Lazily source source_registry if not already loaded
if [[ "$(type -t canonical_id_normalize)" != "function" ]]; then
    # shellcheck source=core/lib/source_registry.sh
    source "${LOADOUT_ROOT}/core/lib/source_registry.sh"
fi

# Lazily source feature_index if not already loaded
if [[ "$(type -t feature_index_build)" != "function" ]]; then
    # shellcheck source=core/lib/feature_index.sh
    source "${LOADOUT_ROOT}/core/lib/feature_index.sh"
fi

# Lazily source compiler if not already loaded
if [[ "$(type -t feature_compiler_run)" != "function" ]]; then
    # shellcheck source=core/lib/compiler.sh
    source "${LOADOUT_ROOT}/core/lib/compiler.sh"
fi

# Lazily source resolver if not already loaded
if [[ "$(type -t read_feature_metadata)" != "function" ]]; then
    # shellcheck source=core/lib/resolver.sh
    source "${LOADOUT_ROOT}/core/lib/resolver.sh"
fi

# Lazily source backend_registry if not already loaded
if [[ "$(type -t backend_registry_load_policy)" != "function" ]]; then
    # shellcheck source=core/lib/backend_registry.sh
    source "${LOADOUT_ROOT}/core/lib/backend_registry.sh"
fi

# Lazily source planner if not already loaded
if [[ "$(type -t planner_run)" != "function" ]]; then
    # shellcheck source=core/lib/planner.sh
    source "${LOADOUT_ROOT}/core/lib/planner.sh"
fi

# Lazily source policy_resolver if not already loaded
if [[ "$(type -t policy_resolver_run)" != "function" ]]; then
    # shellcheck source=core/lib/policy_resolver.sh
    source "${LOADOUT_ROOT}/core/lib/policy_resolver.sh"
fi

# Lazily source executor if not already loaded
if [[ "$(type -t executor_run)" != "function" ]]; then
    # shellcheck source=core/lib/executor.sh
    source "${LOADOUT_ROOT}/core/lib/executor.sh"
fi

# Global variable to cache profile data
declare -g _PROFILE_DATA=""

# ── Profile parsing ───────────────────────────────────────────────────────────

# read_profile <profile_file> <output_array>
# Read profile YAML file and extract feature names from map.
read_profile() {
    local profile_file="$1"
    local -n output_array=$2

    if [[ ! -f "$profile_file" ]]; then
        log_error "Profile file not found: $profile_file"
        return 1
    fi

    log_info "Reading profile..."

    # Cache full profile data for later config extraction
    _PROFILE_DATA=$(cat "$profile_file")

    # Extract feature names (keys from features map) and normalize to canonical IDs.
    # Bare names (e.g. "git") are treated as "core/<name>" per the bare-name=core rule.
    local -a raw_features
    # shellcheck disable=SC2207
    raw_features=($(yq eval '.features | keys | .[]' "$profile_file"))

    local canonical_id
    output_array=()
    for feat in "${raw_features[@]}"; do
        canonical_id=$(canonical_id_normalize "$feat" "core") || {
            log_error "read_profile: invalid feature name: $feat"
            return 1
        }
        output_array+=("$canonical_id")
    done

    if [[ ${#output_array[@]} -eq 0 ]]; then
        log_warn "Empty profile (no features specified)"
        log_info "All installed features will be uninstalled"
        return 0
    fi

    log_info "Desired features: ${output_array[*]}"
    return 0
}

# extract_feature_config <feature>
# Extract configuration for a specific feature from cached profile data.
# <feature> may be a canonical ID ("core/git") or a bare name ("git").
# Returns JSON string or "null" if no config.
extract_feature_config() {
    local feature="$1"

    if [[ -z "$_PROFILE_DATA" ]]; then
        return 0
    fi

    # Try canonical ID via bracket notation (handles "/" in key name)
    local result
    result=$(echo "$_PROFILE_DATA" | yq eval ".features[\"${feature}\"]" -o=json - 2>/dev/null)
    if [[ -n "$result" ]] && [[ "$result" != "null" ]]; then
        echo "$result"
        return 0
    fi

    # Bare name fallback: profile may have been written with bare names (e.g. "git")
    # while feature is canonical ("core/git"). Try the name part only.
    local bare="${feature#*/}"
    if [[ "$bare" != "$feature" ]]; then
        result=$(echo "$_PROFILE_DATA" | yq eval ".features[\"${bare}\"]" -o=json - 2>/dev/null)
    fi

    echo "${result:-null}"
}

# ── Diff calculation ──────────────────────────────────────────────────────────

# check_version_mismatch <feature>
# Return 0 if versions match (or no version pinned), 1 if mismatch.
check_version_mismatch() {
    local feature="$1"

    local feature_config
    feature_config=$(extract_feature_config "$feature")
    local desired_version
    desired_version=$(echo "$feature_config" | jq -r '.version // empty')

    if [[ -z "$desired_version" ]]; then
        return 0
    fi

    local installed_version
    installed_version=$(state_get_runtime "$feature" "version")

    if [[ -z "$installed_version" ]] || [[ "$desired_version" != "$installed_version" ]]; then
        return 1
    fi

    return 0
}

# calculate_diff <sorted_features> <to_install> <to_uninstall> <to_reinstall>
# Calculate difference between desired and installed features.
calculate_diff() {
    local -n sorted_features=$1
    local -n to_install=$2
    local -n to_uninstall=$3
    local -n to_reinstall=$4

    local installed_features
    installed_features=($(state_list_features))

    to_install=()
    to_uninstall=()
    to_reinstall=()

    for feature in "${sorted_features[@]}"; do
        if ! state_has_feature "$feature"; then
            to_install+=("$feature")
        elif ! check_version_mismatch "$feature"; then
            log_info "Version mismatch detected for: $feature"
            to_reinstall+=("$feature")
        fi
    done

    for feature in "${installed_features[@]}"; do
        if [[ ! " ${sorted_features[*]} " =~ " ${feature} " ]]; then
            to_uninstall+=("$feature")
        fi
    done

    log_info "Features to install:   ${to_install[*]:-none}"
    log_info "Features to uninstall: ${to_uninstall[*]:-none}"
    log_info "Features to reinstall: ${to_reinstall[*]:-none}"
}

# ── Execution ─────────────────────────────────────────────────────────────────

# run_uninstall <features>
# Execute uninstall scripts in reverse dependency order.
run_uninstall() {
    local -n features=$1

    if [[ ${#features[@]} -eq 0 ]]; then
        return 0
    fi

    log_task "Uninstalling features..."

    for ((i=${#features[@]}-1; i>=0; i--)); do
        local feature="${features[$i]}"
        local uninstall_script="$LOADOUT_FEATURES_DIR/$feature/uninstall.sh"

        if [[ ! -f "$uninstall_script" ]]; then
            log_error "Uninstall script not found: $uninstall_script"
            return 1
        fi

        log_info "Uninstalling: $feature"
        if ! bash "$uninstall_script"; then
            log_error "Failed to uninstall: $feature"
            return 1
        fi
    done

    return 0
}

# run_install <features>
# Execute install scripts in dependency order.
run_install() {
    local -n features=$1

    if [[ ${#features[@]} -eq 0 ]]; then
        return 0
    fi

    log_task "Installing features..."

    for feature in "${features[@]}"; do
        local install_script="$LOADOUT_FEATURES_DIR/$feature/install.sh"

        if [[ ! -f "$install_script" ]]; then
            log_error "Install script not found: $install_script"
            return 1
        fi

        local feature_config
        feature_config=$(extract_feature_config "$feature")
        local feature_version
        feature_version=$(echo "$feature_config" | jq -r '.version // empty')

        log_info "Installing: $feature"

        export LOADOUT_FEATURE_CONFIG_VERSION="$feature_version"
        if ! bash "$install_script"; then
            log_error "Failed to install: $feature"
            unset LOADOUT_FEATURE_CONFIG_VERSION
            return 1
        fi
        unset LOADOUT_FEATURE_CONFIG_VERSION
    done

    return 0
}

# ── Summary ───────────────────────────────────────────────────────────────────

# print_summary
# Display installed features after a successful apply.
print_summary() {
    echo ""
    log_success "Profile applied successfully!"
    echo ""
    echo "Installed features:"
    for feature in $(state_list_features); do
        echo "  ✓ $feature"
    done
    echo ""
}

# _plan_inject_blocked <plan_json> <blocked_extra_json>
# Inject additional pre-blocked features into plan JSON.
# Prints the updated plan JSON to stdout.
_plan_inject_blocked() {
    local plan_json="$1"
    local blocked_extra="$2"

    if [[ "$blocked_extra" == "[]" ]]; then
        echo "$plan_json"
        return 0
    fi

    echo "$plan_json" | jq \
        --argjson extra "$blocked_extra" \
        '.blocked += $extra | .summary.blocked = (.blocked | length)'
}

# ── Apply pipeline ────────────────────────────────────────────────────────────

# orchestrator_apply <profile_file>
# Full apply pipeline. Entry point called by cmd/apply.sh.
#
# Pipeline:
#   load policy → state_init → read_profile → resolve_dependencies
#   → planner_run → executor_run → print_summary
orchestrator_apply() {
    local profile_file="$1"

    if [[ -z "$profile_file" ]]; then
        log_error "orchestrator_apply: profile file is required"
        return 1
    fi

    if [[ ! -f "$profile_file" ]]; then
        log_error "Profile file not found: $profile_file"
        return 1
    fi

    # Load backend policy (non-fatal if policies dir is absent)
    backend_registry_load_policy

    # Initialise (or migrate) state
    state_init

    # Parse profile
    local -a _apply_features
    read_profile "$profile_file" _apply_features || return 1

    # Build Feature Index: scans all registered sources, enriches with metadata
    local _apply_index
    feature_index_build _apply_index || return 1

    # Filter desired features: separates valid from spec_version-blocked
    local -a _apply_valid_features
    local _apply_sv_blocked
    feature_index_filter "$_apply_index" _apply_features _apply_valid_features _apply_sv_blocked || return 1

    # Resolve feature metadata from index (no file I/O) + topological sort
    read_feature_metadata "$_apply_index" _apply_valid_features || return 1

    local -a _apply_sorted
    resolve_dependencies _apply_valid_features _apply_sorted || return 1

    # Compile raw DesiredResourceGraph (assigns stable resource IDs only)
    local _apply_drg
    _apply_drg=$(feature_compiler_run "$_apply_index" _apply_sorted) || return 1

    # Resolve desired_backend per resource via PolicyResolver
    local _apply_rrg
    _apply_rrg=$(policy_resolver_run "$_apply_drg") || return 1

    # Plan: pure computation of what needs to happen
    local plan_json
    plan_json=$(planner_run "$_apply_rrg" _apply_sorted "$profile_file") || return 1

    # Inject spec_version-blocked features into plan output
    plan_json=$(_plan_inject_blocked "$plan_json" "$_apply_sv_blocked")

    # Execute: impure — calls scripts, commits state
    executor_run "$plan_json" || return 1

    print_summary
}
