#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Module: resolver
#
# Responsibility:
#   Resolve feature dependencies and perform topological sorting.
#   Supports capability-based dependencies via requires/provides fields.
#
# Public API (Stable):
#   resolve_dependencies <desired_features> <output_array>
#   read_feature_metadata <feature_index_json> <features_nameref>
#
# Input/output format:
#   All feature identifiers are canonical IDs of the form "<source_id>/<name>".
#   Dependency data is read exclusively from the Feature Index (feature_index.sh).
#   Resolver does NOT read feature.yaml or any other file directly.
# -----------------------------------------------------------------------------

# This library expects core/env.sh, core/lib/logger.sh, and
# core/lib/source_registry.sh to be sourced by the caller.

# Lazily source source_registry if not already loaded
if [[ "$(type -t canonical_id_normalize)" != "function" ]]; then
    # shellcheck source=core/lib/source_registry.sh
    source "${LOADOUT_ROOT}/core/lib/source_registry.sh"
fi

if [[ "$(type -t source_registry_load)" != "function" ]]; then
    # shellcheck source=core/lib/source_registry.sh
    source "${LOADOUT_ROOT}/core/lib/source_registry.sh"
fi

# Global variables for dependency graph
declare -g -A _RESOLVER_FEATURE_DEPS  # feature -> "dep1 dep2 ..."
declare -g -A _RESOLVER_VISITED
declare -g -A _RESOLVER_IN_STACK
declare -g -a _RESOLVER_SORTED

# Capability maps built by read_feature_metadata
declare -g -A _RESOLVER_PROVIDES  # capability -> "feature1 feature2 ..."
declare -g -A _RESOLVER_REQUIRES  # feature -> "cap1 cap2 ..."

# read_feature_metadata <feature_index_json> <features_nameref>
# Populate resolver globals from the Feature Index.
# Reads dep fields exclusively from the Feature Index JSON — does NOT touch
# feature.yaml or any filesystem file.
#
# Populates:
#   _RESOLVER_FEATURE_DEPS  – canonical depends per feature
#   _RESOLVER_PROVIDES      – capability -> canonical features that provide it
#   _RESOLVER_REQUIRES      – canonical feature -> required capabilities
read_feature_metadata() {
    local feature_index_json="$1"
    local -n features="$2"

    _RESOLVER_FEATURE_DEPS=()
    _RESOLVER_PROVIDES=()
    _RESOLVER_REQUIRES=()

    log_info "Reading feature metadata..."
    local feature
    for feature in "${features[@]}"; do
        local entry
        entry=$(printf '%s' "$feature_index_json" \
            | jq -r --arg id "$feature" '.features[$id] // "null"')

        if [[ "$entry" == "null" ]]; then
            log_error "read_feature_metadata: feature not found in index: $feature"
            return 1
        fi

        # ── depends ─────────────────────────────────────────────────────────
        local dep_arr=()
        local dep
        while IFS= read -r dep; do
            [[ -z "$dep" || "$dep" == "null" ]] && continue
            dep_arr+=("$dep")
        done < <(printf '%s' "$entry" | jq -r '.dep.depends[]' 2>/dev/null || true)

        mapfile -t dep_arr < <(printf '%s\n' "${dep_arr[@]:-}" | sort -u | awk 'NF')
        _RESOLVER_FEATURE_DEPS["$feature"]="${dep_arr[*]:-}"

        # ── provides ────────────────────────────────────────────────────────
        local prov_arr=()
        local cap
        while IFS= read -r cap; do
            [[ -z "$cap" || "$cap" == "null" ]] && continue
            prov_arr+=("$cap")
            if [[ -n "${_RESOLVER_PROVIDES[$cap]:-}" ]]; then
                _RESOLVER_PROVIDES["$cap"]+=" $feature"
            else
                _RESOLVER_PROVIDES["$cap"]="$feature"
            fi
        done < <(printf '%s' "$entry" | jq -r '.dep.provides[].name' 2>/dev/null || true)

        # ── requires ────────────────────────────────────────────────────────
        local req_arr=()
        while IFS= read -r cap; do
            [[ -z "$cap" || "$cap" == "null" ]] && continue
            req_arr+=("$cap")
        done < <(printf '%s' "$entry" | jq -r '.dep.requires[].name' 2>/dev/null || true)

        mapfile -t req_arr < <(printf '%s\n' "${req_arr[@]:-}" | sort -u | awk 'NF')
        _RESOLVER_REQUIRES["$feature"]="${req_arr[*]:-}"

        # ── log ─────────────────────────────────────────────────────────────
        if [[ ${#dep_arr[@]} -gt 0 ]]; then
            log_info "  $feature depends on: ${dep_arr[*]}"
        fi
        if [[ ${#prov_arr[@]} -gt 0 ]]; then
            log_info "  $feature provides: ${prov_arr[*]}"
        fi
        if [[ ${#req_arr[@]} -gt 0 ]]; then
            log_info "  $feature requires capabilities: ${req_arr[*]}"
        fi
        if [[ ${#dep_arr[@]} -eq 0 && ${#prov_arr[@]} -eq 0 && ${#req_arr[@]} -eq 0 ]]; then
            log_info "  $feature has no dependencies"
        fi
    done

    return 0
}

# _resolver_inject_capability_deps <desired_features_nameref>
# For each feature with requires[], find providers in the desired features and
# inject them as implicit entries in _RESOLVER_FEATURE_DEPS.
# Errors if a required capability has no provider in the profile.
_resolver_inject_capability_deps() {
    local -n _inject_desired=$1

    for feature in "${_inject_desired[@]}"; do
        local caps=(${_RESOLVER_REQUIRES[$feature]:-})
        [[ ${#caps[@]} -eq 0 ]] && continue

        for cap in "${caps[@]}"; do
            # Find providers that are present in the desired feature set
            local all_providers=(${_RESOLVER_PROVIDES[$cap]:-})
            local found_providers=()

            for p in "${all_providers[@]}"; do
                if [[ " ${_inject_desired[*]} " =~ " ${p} " ]]; then
                    found_providers+=("$p")
                fi
            done

            if [[ ${#found_providers[@]} -eq 0 ]]; then
                log_error "Feature '$feature' requires capability '$cap'" \
                    "but no provider is present in the profile."
                log_error "  Known providers: ${all_providers[*]:-(none registered)}"
                return 1
            fi

            # Add each found provider as an implicit dependency (deduplicate)
            for p in "${found_providers[@]}"; do
                local existing="${_RESOLVER_FEATURE_DEPS[$feature]:-}"
                if [[ ! " $existing " =~ " $p " ]]; then
                    _RESOLVER_FEATURE_DEPS["$feature"]+="${existing:+ }$p"
                fi
            done

            log_info "  $feature: capability '$cap' provided by: ${found_providers[*]}"
        done
    done
}

# Depth-first search for topological sort
_topo_sort_dfs() {
    local feature="$1"
    shift
    local desired_features=("$@")

    # Check if already visited
    if [[ "${_RESOLVER_VISITED[$feature]:-}" == "true" ]]; then
        return 0
    fi

    # Check for cycle
    if [[ "${_RESOLVER_IN_STACK[$feature]:-}" == "true" ]]; then
        log_error "Circular dependency detected involving: $feature"
        return 1
    fi

    _RESOLVER_IN_STACK["$feature"]="true"

    # Visit dependencies first (includes capability-injected implicit deps)
    local deps=(${_RESOLVER_FEATURE_DEPS[$feature]:-})
    for dep in "${deps[@]}"; do
        # Check if dependency is in desired features
        if [[ ! " ${desired_features[*]} " =~ " ${dep} " ]]; then
            log_error "Dependency '$dep' (required by '$feature') is not in profile"
            return 1
        fi

        _topo_sort_dfs "$dep" "${desired_features[@]}" || return 1
    done

    _RESOLVER_IN_STACK["$feature"]="false"
    _RESOLVER_VISITED["$feature"]="true"
    _RESOLVER_SORTED+=("$feature")
}

# resolve_dependencies <desired_features> <output_array>
# Resolve capability dependencies and return topologically sorted feature list.
resolve_dependencies() {
    local -n desired_features=$1
    local -n output_array=$2

    _RESOLVER_VISITED=()
    _RESOLVER_IN_STACK=()
    _RESOLVER_SORTED=()

    log_info "Resolving dependencies..."

    # Inject implicit deps derived from requires/provides into _RESOLVER_FEATURE_DEPS
    _resolver_inject_capability_deps desired_features || return 1

    # Sort all features
    for feature in "${desired_features[@]}"; do
        _topo_sort_dfs "$feature" "${desired_features[@]}" || return 1
    done

    # Copy result to output array
    output_array=("${_RESOLVER_SORTED[@]}")

    log_success "Install order (canonical IDs): ${output_array[*]}"
    return 0
}
