#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Module: source_registry
#
# Responsibility:
#   Canonical ID utilities for feature and backend identification.
#   A canonical ID is a string of the form "<source_id>/<name>".
#
#   This module provides Phase 1 foundations only.
#   Source loading (sources.yaml, allow lists) is Phase 5.
#
# Reserved source IDs (may not be defined in sources.yaml):
#   core     — built-in features shipped with this repository
#   user     — local user overrides
#   official — reserved for future use
#
# Public API (Stable):
#   canonical_id_normalize <name> <default_source_id>
#   canonical_id_parse     <canonical_id> <out_source_var> <out_name_var>
#   canonical_id_validate  <canonical_id>
#   source_registry_load [sources_yaml]
#   source_registry_get_feature_dir <source_id>
#   source_registry_get_backend_dir <source_id>
#   source_registry_is_allowed <source_id> <feature_name>
#   source_registry_is_backend_allowed <source_id> <backend_name>
# -----------------------------------------------------------------------------

# List of source IDs that cannot be defined by the user in sources.yaml.
# Used for validation in Phase 5; exposed here as a single source of truth.
readonly CANONICAL_ID_RESERVED_SOURCES="core user official"

declare -g _SR_LOADED="false"
declare -g -A _SR_SOURCE_TYPES=()
declare -g -A _SR_FEATURE_DIRS=()
declare -g -A _SR_BACKEND_DIRS=()
declare -g -A _SR_ALLOW_FEATURES_MODE=()
declare -g -A _SR_ALLOW_BACKENDS_MODE=()
declare -g -A _SR_ALLOW_FEATURES_LIST=()
declare -g -A _SR_ALLOW_BACKENDS_LIST=()

# canonical_id_normalize <name> <default_source_id>
#
# Normalize a feature/backend name to a canonical ID.
# If <name> is already a canonical ID ("source/name"), it is returned as-is.
# If <name> is a bare name, <default_source_id>/<name> is produced.
#
# Outputs the canonical ID to stdout.
# Returns 1 if the resulting canonical ID would be invalid.
#
# Examples:
#   canonical_id_normalize "git"         "core"   -> "core/git"
#   canonical_id_normalize "user/myfeat" "core"   -> "user/myfeat"
#   canonical_id_normalize "repo-a/foo"  "core"   -> "repo-a/foo"
canonical_id_normalize() {
    local name="$1"
    local default_source="$2"

    if [[ -z "$name" ]]; then
        log_error "canonical_id_normalize: name is required"
        return 1
    fi
    if [[ -z "$default_source" ]]; then
        log_error "canonical_id_normalize: default_source_id is required"
        return 1
    fi

    local result
    # Already contains a slash → treat as canonical; validate before returning
    if [[ "$name" == */* ]]; then
        result="$name"
    else
        # Bare name → prepend default source
        result="${default_source}/${name}"
    fi

    if ! canonical_id_validate "$result"; then
        return 1
    fi

    echo "$result"
}

# canonical_id_parse <canonical_id> <out_source_var> <out_name_var>
#
# Parse a canonical ID into its source_id and name components.
# Writes results to the caller-supplied variable names (nameref).
# Returns 1 if <canonical_id> is not a valid canonical ID.
#
# Example:
#   local src name
#   canonical_id_parse "core/git" src name
#   # src="core", name="git"
canonical_id_parse() {
    local canonical_id="$1"
    local -n _cip_source_out=$2
    local -n _cip_name_out=$3

    if ! canonical_id_validate "$canonical_id"; then
        return 1
    fi

    _cip_source_out="${canonical_id%%/*}"
    _cip_name_out="${canonical_id#*/}"
}

# canonical_id_validate <canonical_id>
#
# Validate that <canonical_id> is a well-formed canonical ID.
# A valid canonical ID:
#   - is non-empty
#   - contains exactly one "/" separator
#   - has a non-empty source_id part (left of "/")
#   - has a non-empty name part (right of "/")
#   - neither part contains a "/"
#
# Returns 0 if valid, 1 if invalid.
# Does NOT check whether the source_id is reserved — that is source_registry_load's job.
canonical_id_validate() {
    local id="$1"

    # Must be non-empty
    if [[ -z "$id" ]]; then
        return 1
    fi

    # Must contain exactly one "/"
    local slash_count
    slash_count=$(echo "$id" | tr -cd '/' | wc -c)
    if [[ "$slash_count" -ne 1 ]]; then
        return 1
    fi

    local source_part="${id%%/*}"
    local name_part="${id#*/}"

    # Both parts must be non-empty
    if [[ -z "$source_part" ]] || [[ -z "$name_part" ]]; then
        return 1
    fi

    return 0
}

# _source_registry_reserved <source_id>
_source_registry_reserved() {
    local source_id="$1"
    [[ " $CANONICAL_ID_RESERVED_SOURCES " == *" $source_id "* ]]
}

# _source_registry_register_source <source_id> <type> <feature_dir> <backend_dir>
_source_registry_register_source() {
    local source_id="$1"
    local source_type="$2"
    local feature_dir="$3"
    local backend_dir="$4"

    _SR_SOURCE_TYPES["$source_id"]="$source_type"
    _SR_FEATURE_DIRS["$source_id"]="$feature_dir"
    _SR_BACKEND_DIRS["$source_id"]="$backend_dir"

    if [[ "$source_id" == "core" || "$source_id" == "user" ]]; then
        _SR_ALLOW_FEATURES_MODE["$source_id"]="all"
        _SR_ALLOW_BACKENDS_MODE["$source_id"]="all"
    else
        _SR_ALLOW_FEATURES_MODE["$source_id"]="none"
        _SR_ALLOW_BACKENDS_MODE["$source_id"]="none"
    fi
    _SR_ALLOW_FEATURES_LIST["$source_id"]=""
    _SR_ALLOW_BACKENDS_LIST["$source_id"]=""
}

# _source_registry_load_implicit
_source_registry_load_implicit() {
    _source_registry_register_source \
        "core" \
        "implicit" \
        "${LOADOUT_ROOT}/features" \
        "${LOADOUT_ROOT}/backends"

    _source_registry_register_source \
        "user" \
        "implicit" \
        "${LOADOUT_CONFIG_HOME}/features" \
        "${LOADOUT_CONFIG_HOME}/backends"
}

# _source_registry_extract_allow <sources_yaml> <source_id> <kind>
_source_registry_extract_allow() {
    local sources_yaml="$1"
    local source_id="$2"
    local kind="$3"

    local allow_root
    allow_root=$(yq eval ".sources[] | select(.id == \"${source_id}\") | .allow" "$sources_yaml" 2>/dev/null | head -n1 || true)
    if [[ "$allow_root" == "*" ]]; then
        echo "all|"
        return 0
    fi

    local allow_kind
    allow_kind=$(yq eval ".sources[] | select(.id == \"${source_id}\") | .allow.${kind}" "$sources_yaml" 2>/dev/null | head -n1 || true)
    if [[ "$allow_kind" == "*" ]]; then
        echo "all|"
        return 0
    fi

    local items=()
    mapfile -t items < <(yq eval ".sources[] | select(.id == \"${source_id}\") | .allow.${kind}[]" "$sources_yaml" 2>/dev/null || true)
    if [[ ${#items[@]} -eq 0 ]]; then
        echo "none|"
        return 0
    fi

    local unique_items=()
    mapfile -t unique_items < <(printf '%s\n' "${items[@]}" | grep -v '^$' | sort -u)
    echo "list|${unique_items[*]}"
}

# source_registry_load [sources_yaml]
# Load implicit sources and optional external sources from sources.yaml.
source_registry_load() {
    local sources_yaml="${1:-${LOADOUT_SOURCES_FILE:-}}"

    _SR_SOURCE_TYPES=()
    _SR_FEATURE_DIRS=()
    _SR_BACKEND_DIRS=()
    _SR_ALLOW_FEATURES_MODE=()
    _SR_ALLOW_BACKENDS_MODE=()
    _SR_ALLOW_FEATURES_LIST=()
    _SR_ALLOW_BACKENDS_LIST=()

    _source_registry_load_implicit

    if [[ -z "$sources_yaml" || ! -f "$sources_yaml" ]]; then
        _SR_LOADED="true"
        return 0
    fi

    local source_ids=()
    mapfile -t source_ids < <(yq eval '.sources[].id' "$sources_yaml" 2>/dev/null || true)

    local source_id
    for source_id in "${source_ids[@]}"; do
        [[ -z "$source_id" || "$source_id" == "null" ]] && continue

        if _source_registry_reserved "$source_id"; then
            log_error "source_registry_load: reserved source id may not be defined in sources.yaml: $source_id"
            return 1
        fi

        local source_type
        source_type=$(yq eval ".sources[] | select(.id == \"${source_id}\") | .type" "$sources_yaml" 2>/dev/null | head -n1 || true)
        if [[ "$source_type" != "git" ]]; then
            log_error "source_registry_load: unsupported source type for '$source_id': ${source_type:-<missing>}"
            return 1
        fi

        local feature_dir="${LOADOUT_DATA_HOME}/sources/${source_id}/features"
        local backend_dir="${LOADOUT_DATA_HOME}/sources/${source_id}/backends"
        _source_registry_register_source "$source_id" "$source_type" "$feature_dir" "$backend_dir"

        local feature_allow backend_allow
        feature_allow=$(_source_registry_extract_allow "$sources_yaml" "$source_id" "features")
        backend_allow=$(_source_registry_extract_allow "$sources_yaml" "$source_id" "backends")

        _SR_ALLOW_FEATURES_MODE["$source_id"]="${feature_allow%%|*}"
        _SR_ALLOW_FEATURES_LIST["$source_id"]="${feature_allow#*|}"
        _SR_ALLOW_BACKENDS_MODE["$source_id"]="${backend_allow%%|*}"
        _SR_ALLOW_BACKENDS_LIST["$source_id"]="${backend_allow#*|}"
    done

    _SR_LOADED="true"
    return 0
}

# _source_registry_ensure_loaded
_source_registry_ensure_loaded() {
    if [[ "${_SR_LOADED:-false}" != "true" ]]; then
        source_registry_load || return 1
    fi
}

# source_registry_get_feature_dir <source_id>
source_registry_get_feature_dir() {
    local source_id="$1"
    [[ -z "$source_id" ]] && return 1
    _source_registry_ensure_loaded || return 1
    if [[ -z "${_SR_FEATURE_DIRS[$source_id]:-}" ]]; then
        log_error "source_registry_get_feature_dir: unknown source id: $source_id"
        return 1
    fi
    echo "${_SR_FEATURE_DIRS[$source_id]}"
}

# source_registry_get_backend_dir <source_id>
source_registry_get_backend_dir() {
    local source_id="$1"
    [[ -z "$source_id" ]] && return 1
    _source_registry_ensure_loaded || return 1
    if [[ -z "${_SR_BACKEND_DIRS[$source_id]:-}" ]]; then
        log_error "source_registry_get_backend_dir: unknown source id: $source_id"
        return 1
    fi
    echo "${_SR_BACKEND_DIRS[$source_id]}"
}

# source_registry_list_sources <output_nameref>
# Populate output_nameref with all registered source IDs.
source_registry_list_sources() {
    local -n _srl_out="$1"
    _source_registry_ensure_loaded || return 1
    _srl_out=()
    local id
    for id in "${!_SR_SOURCE_TYPES[@]}"; do
        _srl_out+=("$id")
    done
    # Sort for deterministic order (core first, then rest)
    mapfile -t _srl_out < <(printf '%s\n' "${_srl_out[@]}" | sort)
}

# _source_registry_is_allowed_kind <source_id> <name> <kind>
_source_registry_is_allowed_kind() {
    local source_id="$1"
    local name="$2"
    local kind="$3"

    _source_registry_ensure_loaded || return 1

    if [[ -z "${_SR_SOURCE_TYPES[$source_id]:-}" ]]; then
        return 1
    fi

    if [[ "$source_id" == "core" || "$source_id" == "user" ]]; then
        return 0
    fi

    local mode list
    if [[ "$kind" == "feature" ]]; then
        mode="${_SR_ALLOW_FEATURES_MODE[$source_id]:-none}"
        list="${_SR_ALLOW_FEATURES_LIST[$source_id]:-}"
    else
        mode="${_SR_ALLOW_BACKENDS_MODE[$source_id]:-none}"
        list="${_SR_ALLOW_BACKENDS_LIST[$source_id]:-}"
    fi

    case "$mode" in
        all) return 0 ;;
        list)
            [[ " $list " == *" $name "* ]]
            return
            ;;
        *) return 1 ;;
    esac
}

# source_registry_is_allowed <source_id> <feature_name>
source_registry_is_allowed() {
    _source_registry_is_allowed_kind "$1" "$2" "feature"
}

# source_registry_is_backend_allowed <source_id> <backend_name>
source_registry_is_backend_allowed() {
    _source_registry_is_allowed_kind "$1" "$2" "backend"
}
