#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Module: policy_resolver (PolicyResolver)
#
# Responsibility:
#   Convert a raw DesiredResourceGraph (compiler output) into a Resolved
#   Resource Graph (RRG) by adding desired_backend to each package/runtime
#   resource according to the active backend policy.
#
# Public API (Stable):
#   policy_resolver_run <drg_json>  → RRG JSON (stdout)
#
# Contract:
#   - package/runtime resources receive desired_backend via resolve_backend_for
#   - fs resources pass through unchanged (no backend applies)
#   - unknown resource kinds pass through unchanged (Planner will block them)
#   - top-level DRG fields (schema_version, etc.) are preserved
#
# This module requires backend_registry.sh (and its loaded policy) to be
# available before calling policy_resolver_run.
# -----------------------------------------------------------------------------

# policy_resolver_run <drg_json>
# Add desired_backend to each package/runtime resource in the raw DRG.
# Prints Resolved Resource Graph JSON to stdout.
policy_resolver_run() {
    local drg_json="$1"

    if [[ -z "$drg_json" ]]; then
        log_error "policy_resolver_run: drg_json is required"
        return 1
    fi

    # Collect feature IDs from the DRG
    local -a feature_ids
    mapfile -t feature_ids < <(printf '%s' "$drg_json" | jq -r '.features | keys[]')

    local features_json="{}"

    local canonical_id
    for canonical_id in "${feature_ids[@]}"; do
        local resources_json="[]"

        while IFS= read -r resource_json; do
            local kind
            kind=$(printf '%s' "$resource_json" | jq -r '.kind // empty')

            local resolved_resource
            case "$kind" in
                package)
                    local name desired_backend
                    name=$(printf '%s' "$resource_json" | jq -r '.name // empty')
                    desired_backend=$(resolve_backend_for "package" "$name" 2>/dev/null) \
                        || desired_backend="unknown"
                    resolved_resource=$(printf '%s' "$resource_json" | jq \
                        --arg backend "$desired_backend" \
                        '. + {"desired_backend": $backend}')
                    ;;
                runtime)
                    local name desired_backend
                    name=$(printf '%s' "$resource_json" | jq -r '.name // empty')
                    desired_backend=$(resolve_backend_for "runtime" "$name" 2>/dev/null) \
                        || desired_backend="unknown"
                    resolved_resource=$(printf '%s' "$resource_json" | jq \
                        --arg backend "$desired_backend" \
                        '. + {"desired_backend": $backend}')
                    ;;
                fs|*)
                    # fs resources have no backend.
                    # Unknown kinds pass through; Planner will classify them as blocked.
                    resolved_resource="$resource_json"
                    ;;
            esac

            resources_json=$(printf '%s' "$resources_json" \
                | jq --argjson r "$resolved_resource" '. + [$r]')
        done < <(printf '%s' "$drg_json" \
            | jq -c --arg f "$canonical_id" '.features[$f].resources // [] | .[]')

        features_json=$(printf '%s' "$features_json" | jq \
            --arg id  "$canonical_id" \
            --argjson res "$resources_json" \
            '.[$id] = {"resources": $res}')
    done

    # Overwrite features in the DRG; preserve schema_version and other top-level fields.
    printf '%s' "$drg_json" | jq --argjson features "$features_json" '. + {"features": $features}'
}
