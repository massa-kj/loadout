#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Module: compiler (FeatureCompiler)
#
# Responsibility:
#   Compile the raw DesiredResourceGraph from Feature Index + resolved feature
#   order. Assigns stable resource IDs; does NOT resolve backends or read
#   profile/policy. That is PolicyResolver's job.
#
# Public API (Stable):
#   feature_compiler_run <feature_index_json> <sorted_features_nameref>
#   Prints raw DesiredResourceGraph JSON to stdout.
#
# Contract:
#   - mode:script     → entry with empty resources array
#   - mode:declarative → resources expanded with stable IDs (no desired_backend)
#   - Declarative validation: error if no resources, error if install.sh present
#   - PolicyResolver is responsible for adding desired_backend to each resource
#
# JSON schema: see docs/specs/data/desired_resource_graph.md
# -----------------------------------------------------------------------------

# This library expects env.sh, logger.sh, and source_registry.sh to be sourced
# by the caller. backend_registry is NOT needed by this module.

# _compiler_resource_id <kind> <resource_json>
# Derive a stable resource id from a resource JSON object.
# Uses .id if present; otherwise auto-generates from kind+name/path.
_compiler_resource_id() {
    local kind="$1"
    local resource_json="$2"

    local existing_id
    existing_id=$(printf '%s' "$resource_json" | jq -r '.id // empty')
    if [[ -n "$existing_id" && "$existing_id" != "null" ]]; then
        echo "$existing_id"
        return 0
    fi

    case "$kind" in
        package)
            local name
            name=$(printf '%s' "$resource_json" | jq -r '.name // empty')
            echo "package:${name}"
            ;;
        runtime)
            local name
            name=$(printf '%s' "$resource_json" | jq -r '.name // empty')
            echo "runtime:${name}"
            ;;
        fs)
            # Feature.yaml uses 'target' for the deployment path; fall back to 'path'.
            local target
            target=$(printf '%s' "$resource_json" | jq -r '(.target // .path) // empty')
            # Normalize ~ to $HOME for stable id generation
            target="${target/#\~/$HOME}"
            echo "fs:$(basename "$target")"
            ;;
        *)
            echo "${kind}:unknown"
            ;;
    esac
}

# _compiler_resolve_resource <canonical_id> <resource_json>
# Add a stable id to a resource JSON object. Does not resolve backends.
# Prints updated resource JSON to stdout.
_compiler_resolve_resource() {
    local canonical_id="$1"
    local resource_json="$2"

    local kind
    kind=$(printf '%s' "$resource_json" | jq -r '.kind // empty')
    if [[ -z "$kind" || "$kind" == "null" ]]; then
        log_error "feature_compiler_run: resource missing 'kind' in $canonical_id"
        return 1
    fi

    local resource_id
    resource_id=$(_compiler_resource_id "$kind" "$resource_json") || return 1

    # Add id field; all other fields are passed through unchanged.
    # PolicyResolver will add desired_backend in a subsequent step.
    case "$kind" in
        package|runtime|fs)
            printf '%s' "$resource_json" | jq --arg id "$resource_id" '. + {"id": $id}'
            ;;
        *)
            log_error "feature_compiler_run: unknown resource kind '$kind' in $canonical_id"
            return 1
            ;;
    esac
}

# feature_compiler_run <feature_index_json> <sorted_features_nameref>
# Compile raw DesiredResourceGraph from Feature Index and resolved feature order.
# Prints DesiredResourceGraph JSON to stdout.
#
# For mode:script features: produces an entry with an empty resources array.
# For mode:declarative features: expands resources with stable IDs.
# Call policy_resolver_run on the output to add desired_backend per resource.
feature_compiler_run() {
    local feature_index_json="$1"
    local -n _fcr_sorted="$2"

    local features_json="{}"

    local canonical_id
    for canonical_id in "${_fcr_sorted[@]}"; do
        local entry
        entry=$(printf '%s' "$feature_index_json" \
            | jq -r --arg id "$canonical_id" '.features[$id] // "null"')

        if [[ "$entry" == "null" ]]; then
            log_error "feature_compiler_run: feature not found in index: $canonical_id"
            return 1
        fi

        local mode
        mode=$(printf '%s' "$entry" | jq -r '.mode')

        local resources_json="[]"

        if [[ "$mode" == "declarative" ]]; then
            # ── Declarative validation ─────────────────────────────────────
            local source_dir
            source_dir=$(printf '%s' "$entry" | jq -r '.source_dir')

            # Must not have install.sh or uninstall.sh
            if [[ -f "$source_dir/install.sh" || -f "$source_dir/uninstall.sh" ]]; then
                log_error "feature_compiler_run: declarative feature must not have install.sh/uninstall.sh: $canonical_id"
                return 1
            fi

            # spec must exist with at least one resource
            local spec_resources
            spec_resources=$(printf '%s' "$entry" | jq '.spec.resources // []')
            local res_count
            res_count=$(printf '%s' "$spec_resources" | jq 'length')
            if [[ "$res_count" -eq 0 ]]; then
                log_error "feature_compiler_run: declarative feature has no resources defined: $canonical_id"
                return 1
            fi

            # ── Expand resources with desired_backend ──────────────────────
            while IFS= read -r resource_json; do
                local resolved
                resolved=$(_compiler_resolve_resource "$canonical_id" "$resource_json") || return 1
                resources_json=$(printf '%s' "$resources_json" \
                    | jq --argjson r "$resolved" '. + [$r]')
            done < <(printf '%s' "$spec_resources" | jq -c '.[]')
        fi

        features_json=$(printf '%s' "$features_json" | jq \
            --arg id  "$canonical_id" \
            --argjson res "$resources_json" \
            '.[$id] = {"resources": $res}')
    done

    printf '{"schema_version": 1, "features": %s}\n' "$features_json"
}
