#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Module: feature_index
#
# Responsibility:
#   Build the Feature Index by scanning all registered sources.
#   Produces a JSON Feature Index consumed by Resolver and FeatureCompiler.
#
# Public API (Stable):
#   feature_index_build <output_var>
#   feature_index_filter <feature_index_json_var> <desired_nameref> <valid_out> <blocked_json_out>
#
# JSON schema: see docs/specs/data/feature_index.md
# -----------------------------------------------------------------------------

# This library expects env.sh, logger.sh, and source_registry.sh to be sourced
# by the caller.

# Lazily source source_registry if not already loaded
if [[ "$(type -t canonical_id_normalize)" != "function" ]]; then
    # shellcheck source=core/lib/source_registry.sh
    source "${LOADOUT_ROOT}/core/lib/source_registry.sh"
fi

# _feature_index_platform_yaml <feature_dir>
# Print the platform-specific feature file path, or empty string if none.
_feature_index_platform_yaml() {
    local feature_dir="$1"
    if [[ "$LOADOUT_PLATFORM" == "wsl" ]]; then
        if [[ -f "$feature_dir/feature.wsl.yaml" ]]; then
            echo "$feature_dir/feature.wsl.yaml"
            return 0
        elif [[ -f "$feature_dir/feature.linux.yaml" ]]; then
            echo "$feature_dir/feature.linux.yaml"
            return 0
        fi
    elif [[ -n "${LOADOUT_PLATFORM:-}" ]]; then
        if [[ -f "$feature_dir/feature.${LOADOUT_PLATFORM}.yaml" ]]; then
            echo "$feature_dir/feature.${LOADOUT_PLATFORM}.yaml"
            return 0
        fi
    fi
    echo ""
}

# _feature_index_parse_entry <canonical_id> <source_id> <feature_dir> <feature_yaml>
# Emit a JSON object for a single feature entry.
# Returns 1 on fatal parse error; blocked features emit an entry with blocked:true.
_feature_index_parse_entry() {
    local canonical_id="$1"
    local source_id="$2"
    local feature_dir="$3"
    local feature_yaml="$4"

    # ── spec_version, mode, description ─────────────────────────────────────
    local spec_version mode description
    spec_version=$(yq eval '.spec_version // 1' "$feature_yaml" 2>/dev/null)
    mode=$(yq eval '.mode // "declarative"' "$feature_yaml" 2>/dev/null)
    description=$(yq eval '.description // ""' "$feature_yaml" 2>/dev/null)
    [[ "$description" == "null" ]] && description=""

    # ── platform override ────────────────────────────────────────────────────
    local platform_yaml
    platform_yaml=$(_feature_index_platform_yaml "$feature_dir")

    # ── spec_version check ───────────────────────────────────────────────────
    local max_ver="${SUPPORTED_FEATURE_SPEC_VERSION:-1}"
    local blocked="false"
    local blocked_reason="null"
    if [[ "$spec_version" -gt "$max_ver" ]]; then
        blocked="true"
        blocked_reason="\"unsupported spec_version: ${spec_version} (max: ${max_ver})\""
    fi

    # ── depends (base + platform, appended) ─────────────────────────────────
    local raw_depends_json
    raw_depends_json=$(yq eval -o=json '.depends // []' "$feature_yaml" 2>/dev/null)
    [[ -z "$raw_depends_json" || "$raw_depends_json" == "null" ]] && raw_depends_json="[]"

    if [[ -n "$platform_yaml" ]]; then
        local plat_deps
        plat_deps=$(yq eval -o=json '.depends // []' "$platform_yaml" 2>/dev/null)
        [[ -z "$plat_deps" || "$plat_deps" == "null" ]] && plat_deps="[]"
        raw_depends_json=$(printf '[%s, %s]' "$raw_depends_json" "$plat_deps" | jq 'flatten')
    fi

    # Normalize each dep to canonical ID
    local norm_deps_json="[]"
    local raw_dep
    while IFS= read -r raw_dep; do
        [[ -z "$raw_dep" || "$raw_dep" == "null" ]] && continue
        local canonical_dep
        canonical_dep=$(canonical_id_normalize "$raw_dep" "$source_id") || {
            log_error "feature_index_build: invalid depends entry '$raw_dep' in $canonical_id"
            return 1
        }
        norm_deps_json=$(printf '%s' "$norm_deps_json" | jq --arg d "$canonical_dep" '. + [$d]')
    done < <(printf '%s' "$raw_depends_json" | jq -r '.[]')
    # Deduplicate
    norm_deps_json=$(printf '%s' "$norm_deps_json" | jq '[unique[]]')

    # ── provides ────────────────────────────────────────────────────────────
    local provides_json
    provides_json=$(yq eval -o=json '.provides // []' "$feature_yaml" 2>/dev/null)
    [[ -z "$provides_json" || "$provides_json" == "null" ]] && provides_json="[]"
    if [[ -n "$platform_yaml" ]]; then
        local plat_prov
        plat_prov=$(yq eval -o=json '.provides // []' "$platform_yaml" 2>/dev/null)
        [[ -z "$plat_prov" || "$plat_prov" == "null" ]] && plat_prov="[]"
        provides_json=$(printf '[%s, %s]' "$provides_json" "$plat_prov" \
            | jq 'flatten | unique_by(.name)')
    fi

    # ── requires ────────────────────────────────────────────────────────────
    local requires_json
    requires_json=$(yq eval -o=json '.requires // []' "$feature_yaml" 2>/dev/null)
    [[ -z "$requires_json" || "$requires_json" == "null" ]] && requires_json="[]"
    if [[ -n "$platform_yaml" ]]; then
        local plat_req
        plat_req=$(yq eval -o=json '.requires // []' "$platform_yaml" 2>/dev/null)
        [[ -z "$plat_req" || "$plat_req" == "null" ]] && plat_req="[]"
        requires_json=$(printf '[%s, %s]' "$requires_json" "$plat_req" \
            | jq 'flatten | unique_by(.name)')
    fi

    # ── spec (resources for declarative mode) ────────────────────────────────
    local spec_json="null"
    if [[ "$mode" == "declarative" ]]; then
        local resources_json
        resources_json=$(yq eval -o=json '.resources // []' "$feature_yaml" 2>/dev/null)
        [[ -z "$resources_json" || "$resources_json" == "null" ]] && resources_json="[]"
        # Platform override replaces base resources if non-empty
        if [[ -n "$platform_yaml" ]]; then
            local plat_res
            plat_res=$(yq eval -o=json '.resources // []' "$platform_yaml" 2>/dev/null)
            [[ -z "$plat_res" || "$plat_res" == "null" ]] && plat_res="[]"
            if [[ "$plat_res" != "[]" ]]; then
                resources_json="$plat_res"
            fi
        fi
        spec_json=$(printf '{"resources": %s}' "$resources_json")
    fi

    # ── emit JSON ────────────────────────────────────────────────────────────
    jq -n \
        --argjson sv "$spec_version" \
        --arg     mode "$mode" \
        --arg     desc "$description" \
        --arg     dir  "$feature_dir" \
        --argjson blocked   "$blocked" \
        --argjson br        "$blocked_reason" \
        --argjson deps      "$norm_deps_json" \
        --argjson prov      "$provides_json" \
        --argjson reqs      "$requires_json" \
        --argjson spec      "$spec_json" \
        '{
            spec_version: $sv,
            mode: $mode,
            description: $desc,
            source_dir: $dir,
            blocked: $blocked,
            blocked_reason: $br,
            dep: {
                depends: $deps,
                provides: $prov,
                requires: $reqs
            },
            spec: $spec
        }'
}

# feature_index_build <output_var>
# Build the Feature Index by scanning all registered sources (1 level deep).
# Populates output_var with a JSON string conforming to feature_index.md schema.
# Features with unsupported spec_version are included with blocked:true.
feature_index_build() {
    local -n _fib_out="$1"

    source_registry_load || return 1

    local source_ids=()
    source_registry_list_sources source_ids || return 1

    local features_json="{}"

    local source_id
    for source_id in "${source_ids[@]}"; do
        local feature_dir_root
        feature_dir_root=$(source_registry_get_feature_dir "$source_id" 2>/dev/null) || continue
        [[ -d "$feature_dir_root" ]] || continue

        # Scan 1 level deep — no recursive scan
        local feature_dir
        while IFS= read -r -d '' feature_dir; do
            [[ -d "$feature_dir" ]] || continue
            local feature_name
            feature_name=$(basename "$feature_dir")
            local feature_yaml="$feature_dir/feature.yaml"
            [[ -f "$feature_yaml" ]] || continue  # skip dirs without feature.yaml

            local canonical_id="${source_id}/${feature_name}"

            local entry
            entry=$(_feature_index_parse_entry \
                "$canonical_id" "$source_id" "$feature_dir" "$feature_yaml") || return 1

            features_json=$(printf '%s' "$features_json" \
                | jq --arg id "$canonical_id" --argjson e "$entry" '.[$id] = $e')
        done < <(find "$feature_dir_root" -mindepth 1 -maxdepth 1 -type d -print0 \
                    | sort -z)
    done

    _fib_out=$(printf '{"schema_version": 1, "features": %s}' "$features_json")
}

# feature_index_filter <feature_index_json> <desired_nameref> <valid_out> <blocked_json_out>
# For each feature in desired_nameref, check the Feature Index:
#   - Not present in index → error (feature unknown)
#   - blocked:true → adds to blocked_json_out
#   - blocked:false → adds to valid_out
feature_index_filter() {
    local feature_index_json="$1"
    local -n _fif_desired="$2"
    local -n _fif_valid="$3"
    local -n _fif_blocked_json="$4"

    _fif_valid=()
    _fif_blocked_json="[]"

    local feature
    for feature in "${_fif_desired[@]}"; do
        local entry
        entry=$(printf '%s' "$feature_index_json" \
            | jq -r --arg id "$feature" '.features[$id] // "null"')

        if [[ "$entry" == "null" ]]; then
            log_error "feature_index_filter: feature not found in index: $feature"
            return 1
        fi

        local blocked reason
        blocked=$(printf '%s' "$entry" | jq -r '.blocked')
        reason=$(printf '%s' "$entry"  | jq -r '.blocked_reason // ""')

        if [[ "$blocked" == "true" ]]; then
            log_warn "Blocked: $feature — $reason"
            _fif_blocked_json=$(printf '%s' "$_fif_blocked_json" \
                | jq --arg f "$feature" --arg r "$reason" '. + [{"feature": $f, "reason": $r}]')
        else
            _fif_valid+=("$feature")
        fi
    done
}
