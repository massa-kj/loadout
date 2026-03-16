#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Module: declarative_executor
#
# Responsibility:
#   Execute plan actions for mode:declarative features.
#   Reads resources from feature.yaml spec.resources and installs/uninstalls
#   each resource through the appropriate backend or fs operation.
#
# Public API:
#   declarative_executor_run <feature> <operation> <details_json>
#
# Operations:
#   create          — install all spec.resources; record in state
#   destroy         — uninstall all state resources; remove feature from state
#   replace         — destroy (resources + state) then install all spec.resources
#   replace_backend — same as replace
#   strengthen      — install only resources listed in details.add_resources
#
# Design notes:
#   - Resources are read directly from feature.yaml .resources at execute time.
#     This mirrors how the script executor reads packages/runtimes/files fields.
#   - desired_backend is re-resolved via resolve_backend_for (same as script executor).
#   - fs source paths use the explicit 'source' field in the resource (relative to
#     feature_dir). Fallback: files/<basename(path)> by convention.
#   - strengthen uses state_patch_begin from current state so existing resources
#     are preserved; only the add_resources list is freshly installed.
#   - This module is sourced by executor.sh and must NOT source executor.sh
#     (circular dependency). All _executor_* helpers are provided by executor.sh.
# -----------------------------------------------------------------------------

# This module expects env.sh, logger.sh, state.sh, backend_registry.sh, and
# executor.sh (for _executor_feature_dir / _executor_remove_resources etc.)
# to be sourced by the caller (executor.sh).

# ── Spec resource reading ─────────────────────────────────────────────────────

# _de_spec_resources <feature>
# Read .resources from feature.yaml (and platform override if present).
# Platform-specific override replaces (not appends) base resources when non-empty.
# Outputs JSON array to stdout.
_de_spec_resources() {
    local feature="$1"

    local meta_file platform_meta
    meta_file=$(_executor_resolve_feature_file "$feature") || return 1
    platform_meta=$(_executor_resolve_platform_feature_file "$feature")

    # Platform override replaces base if it declares a non-empty resources list
    if [[ -n "$platform_meta" ]]; then
        local plat_res
        plat_res=$(yq eval -o=json '.resources // []' "$platform_meta" 2>/dev/null)
        [[ -z "$plat_res" || "$plat_res" == "null" ]] && plat_res="[]"
        if [[ "$plat_res" != "[]" ]]; then
            echo "$plat_res"
            return 0
        fi
    fi

    local base_res
    base_res=$(yq eval -o=json '.resources // []' "$meta_file" 2>/dev/null)
    echo "${base_res:-[]}"
}

# ── Resource installation ─────────────────────────────────────────────────────

# _de_install_resource <feature> <feature_dir> <resource_json>
# Install a single resource and record it in the active state patch.
_de_install_resource() {
    local feature="$1"
    local feature_dir="$2"
    local resource_json="$3"

    local kind
    kind=$(printf '%s' "$resource_json" | jq -r '.kind')

    case "$kind" in
        package)
            local name backend
            name=$(printf '%s' "$resource_json" | jq -r '.name')
            backend=$(resolve_backend_for "package" "$name") || return 1
            load_backend "$backend" || return 1

            if backend_call "package_exists" "$name" 2>/dev/null; then
                log_info "    package already installed: $name"
            else
                log_info "    installing package: $name"
                backend_call "install_package" "$name" || {
                    log_error "declarative_executor: failed to install package: $name"
                    return 1
                }
            fi

            local rid
            rid=$(printf '%s' "$resource_json" | jq -r '.id // ("pkg:" + .name)')
            local state_res
            state_res=$(jq -n \
                --arg id  "$rid" \
                --arg name "$name" \
                --arg backend "$backend" \
                '{kind: "package", id: $id, backend: $backend,
                  package: {name: $name, version: null}}')
            state_patch_add_resource "$feature" "$state_res" || return 1
            ;;

        runtime)
            local name backend rt_version actual_version
            name=$(printf '%s' "$resource_json" | jq -r '.name')
            backend=$(resolve_backend_for "runtime" "$name") || return 1
            load_backend "$backend" || return 1

            # Version from resource field (set in feature.yaml or via plan details).
            # Profile-based versioning passes the version through the plan action's
            # details.config_version if the orchestrator injects it (future extension).
            rt_version=$(printf '%s' "$resource_json" | jq -r '.version // ""')
            [[ -z "$rt_version" ]] && rt_version="latest"

            actual_version="$rt_version"
            if backend_call "runtime_exists" "$name" "$rt_version" 2>/dev/null; then
                log_info "    runtime already installed: $name@$rt_version"
            else
                log_info "    installing runtime: $name@$rt_version"
                actual_version=$(backend_call "install_runtime" "$name" "$rt_version") || {
                    log_error "declarative_executor: failed to install runtime: $name@$rt_version"
                    return 1
                }
                [[ -z "$actual_version" ]] && actual_version="$rt_version"
            fi

            local rid
            rid=$(printf '%s' "$resource_json" | jq -r '.id // ("rt:" + .name)')
            local state_res
            state_res=$(jq -n \
                --arg id      "$rid" \
                --arg name    "$name" \
                --arg ver     "$actual_version" \
                --arg backend "$backend" \
                '{kind: "runtime", id: $id, backend: $backend,
                  runtime: {name: $name, version: $ver}}')
            state_patch_add_resource "$feature" "$state_res" || return 1
            ;;

        fs)
            local path op source_rel src parent
            path=$(printf '%s' "$resource_json" | jq -r '.path // .target // empty')
            op=$(printf '%s' "$resource_json" | jq -r '.op // "link"')
            source_rel=$(printf '%s' "$resource_json" | jq -r '.source // empty')

            if [[ -z "$path" ]]; then
                log_error "declarative_executor: fs resource missing 'path' field"
                return 1
            fi

            # Expand ~ in target path
            path="${path/#\~/$HOME}"

            # Resolve source file:
            #   explicit: feature_dir/<source_rel>
            #   fallback: feature_dir/files/<basename(path)>
            if [[ -n "$source_rel" ]]; then
                src="$feature_dir/$source_rel"
            else
                src="$feature_dir/files/$(basename "$path")"
            fi

            if [[ ! -e "$src" ]]; then
                log_error "declarative_executor: fs source not found: $src"
                return 1
            fi

            # Ensure parent directory
            parent="$(dirname "$path")"
            [[ ! -d "$parent" ]] && mkdir -p "$parent"

            # Handle existing path: only remove if managed by loadout
            if [[ -e "$path" ]] || [[ -L "$path" ]]; then
                if state_has_file "$path"; then
                    rm -rf "$path"
                else
                    log_error "declarative_executor: path exists and is not managed: $path"
                    return 1
                fi
            fi

            # Deploy
            if [[ "$op" == "link" ]]; then
                if ln -s "$src" "$path" 2>/dev/null; then
                    log_success "  Linked $path"
                else
                    cp -r "$src" "$path" || {
                        log_error "declarative_executor: copy fallback failed: $path"
                        return 1
                    }
                    log_success "  Copied $path (link fallback)"
                fi
            else
                cp -r "$src" "$path" || {
                    log_error "declarative_executor: copy failed: $path"
                    return 1
                }
                log_success "  Copied $path"
            fi

            # Detect actual entry type from filesystem
            local actual_et actual_op
            if   [[ -L "$path" ]]; then actual_et="symlink"; actual_op="link"
            elif [[ -d "$path" ]]; then actual_et="dir";     actual_op="copy"
            else                        actual_et="file";    actual_op="copy"
            fi

            local rid
            rid=$(printf '%s' "$resource_json" | jq -r \
                --arg p "$path" \
                '.id // ("fs:" + ($p | split("/") | last))')
            local state_res
            state_res=$(jq -n \
                --arg id "$rid" --arg p "$path" \
                --arg et "$actual_et" --arg op "$actual_op" \
                '{kind: "fs", id: $id, backend: "fs",
                  fs: {path: $p, entry_type: $et, op: $op}}')
            state_patch_add_resource "$feature" "$state_res" || return 1
            ;;

        *)
            log_error "declarative_executor: unsupported resource kind: $kind"
            return 1
            ;;
    esac
}

# _de_install_resources <feature> <resources_json>
# Install all resources in the given JSON array.
_de_install_resources() {
    local feature="$1"
    local resources_json="$2"

    local feature_dir
    feature_dir=$(_executor_feature_dir "$feature") || return 1

    local count i res
    count=$(printf '%s' "$resources_json" | jq 'length')
    for ((i = 0; i < count; i++)); do
        res=$(printf '%s' "$resources_json" | jq --argjson i "$i" '.[$i]')
        _de_install_resource "$feature" "$feature_dir" "$res" || return 1
    done
}

# ── Public API ────────────────────────────────────────────────────────────────

# declarative_executor_run <feature> <operation> <details_json>
# Execute a plan action for a mode:declarative feature.
declarative_executor_run() {
    local feature="$1"
    local operation="$2"
    # Note: cannot use "${3:-{}}" — bash expands ${var:-{}} as "${var}}" (trailing })
    local details_json="${3:-}"
    [[ -z "$details_json" ]] && details_json="{}"

    log_info "[$operation] $feature (declarative)"

    local spec_resources
    case "$operation" in
        create)
            spec_resources=$(_de_spec_resources "$feature") || return 1
            state_patch_begin || return 1
            _de_install_resources "$feature" "$spec_resources" || return 1
            state_patch_finalize || return 1
            ;;

        destroy)
            # Remove resources tracked in state, then remove the feature entry
            _executor_remove_resources "$feature" || return 1
            state_patch_begin || return 1
            state_patch_remove_feature "$feature" || return 1
            state_patch_finalize || return 1
            ;;

        replace|replace_backend)
            # Destroy phase: remove currently installed resources + clear state entry
            _executor_remove_resources "$feature" || return 1
            state_patch_begin || return 1
            state_patch_remove_feature "$feature" || return 1
            state_patch_finalize || return 1
            # Install phase: install all spec resources from scratch
            spec_resources=$(_de_spec_resources "$feature") || return 1
            state_patch_begin || return 1
            _de_install_resources "$feature" "$spec_resources" || return 1
            state_patch_finalize || return 1
            ;;

        strengthen)
            # Install only the resources in details.add_resources.
            # state_patch_begin starts from the current state, so existing
            # resources in the feature are preserved automatically.
            local add_ids
            add_ids=$(printf '%s' "$details_json" \
                | jq '[.add_resources // [] | .[].id]')

            spec_resources=$(_de_spec_resources "$feature") || return 1

            # Filter spec resources to only those listed in add_ids
            local add_resources
            add_resources=$(printf '%s' "$spec_resources" | jq \
                --argjson ids "$add_ids" \
                '[.[] | select(.id | IN($ids[]))]')

            local add_count
            add_count=$(printf '%s' "$add_resources" | jq 'length')
            if [[ "$add_count" -eq 0 ]]; then
                log_warn "declarative_executor: strengthen: no matching add_resources found (noop)"
                return 0
            fi

            state_patch_begin || return 1
            _de_install_resources "$feature" "$add_resources" || return 1
            state_patch_finalize || return 1
            ;;

        *)
            log_error "declarative_executor_run: unsupported operation: $operation"
            return 1
            ;;
    esac
}
