#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Module: executor
#
# Responsibility:
#   IMPURE executor. Receives a plan JSON object from planner and executes it.
#   Manages all package/runtime/file installations and state commits.
#
# Public API:
#   executor_run <plan_json>
#
# Execution contract:
#   - Blocked features in plan.blocked are reported and skipped.
#   - Actions are executed in plan.actions order (destroy → replace → create).
#   - For each action, executor reads feature.yaml to determine packages/runtimes/files.
#   - On any failure: abort immediately with non-zero exit.
#     Partial execution is left in place; state reflects what succeeded.
#
# State commit model (Phase 4):
#   Executor owns all state writes:
#     install:  state_patch_begin → packages/runtimes/files → state_patch_finalize
#               → run install.sh (for secondary pkgs: npm/uv etc.)
#     destroy:  _executor_remove_resources (fs rm + backend uninstall)
#               → run uninstall.sh (for secondary pkg cleanup)
#               → state_patch_begin → state_patch_remove_feature → state_patch_finalize
#
#   Feature scripts must NOT call state_remove_feature, state_add_package
#   (except for secondary packages like npm:/uv:), install_package, install_runtime.
#
# feature.yaml package/runtime/files schema:
#   packages:
#     - tmux                    # string: name only, managed=true
#     - name: neovim
#       managed: false          # do not uninstall on feature remove
#   runtimes:
#     - name: node              # version from LOADOUT_FEATURE_CONFIG_VERSION
#     - name: rust-analyzer
#       version: "2025-05-26"   # fixed version
#   files:
#     - src: .tmux.conf         # path relative to feature/files/
#       target: ~/.tmux.conf    # ~ expanded
#       op: link                # link (symlink, fallback copy) or copy
# -----------------------------------------------------------------------------

# This library expects env.sh, logger.sh, state.sh, backend_registry.sh to be
# sourced by the caller (orchestrator).

if [[ "$(type -t source_registry_get_feature_dir)" != "function" ]]; then
    # shellcheck source=core/lib/source_registry.sh
    source "${LOADOUT_ROOT}/core/lib/source_registry.sh"
fi

if [[ "$(type -t declarative_executor_run)" != "function" ]]; then
    # shellcheck source=core/lib/declarative_executor.sh
    source "${LOADOUT_ROOT}/core/lib/declarative_executor.sh"
fi

# ── Meta helpers ──────────────────────────────────────────────────────────────

# _executor_feature_dir <feature>
# Resolve the concrete feature directory from the source registry.
_executor_feature_dir() {
    local feature="$1"
    local source_id feat_name
    canonical_id_parse "$feature" source_id feat_name || return 1

    local feature_root
    feature_root=$(source_registry_get_feature_dir "$source_id") || return 1
    echo "${feature_root}/${feat_name}"
}

# _executor_resolve_feature_file <feature>
# Print the base feature.yaml path for a feature. Fails if not found.
# <feature> may be a canonical ID ("core/git") or bare name; both are handled.
_executor_resolve_feature_file() {
    local feature="$1"
    local feature_dir
    feature_dir=$(_executor_feature_dir "$feature") || return 1
    local ffile="$feature_dir/feature.yaml"
    if [[ ! -f "$ffile" ]]; then
        log_error "_executor_resolve_feature_file: feature.yaml not found for: $feature"
        return 1
    fi
    echo "$ffile"
}

# _executor_resolve_platform_feature_file <feature>
# Print the platform-specific feature.yaml path, or empty string if none exists.
# Priority: feature.wsl.yaml > feature.linux.yaml (for wsl), feature.linux.yaml (for linux),
#           feature.<platform>.yaml (for others).
_executor_resolve_platform_feature_file() {
    local feature="$1"
    local dir
    dir=$(_executor_feature_dir "$feature") || return 1

    if [[ "$LOADOUT_PLATFORM" == "wsl" ]]; then
        if [[ -f "$dir/feature.wsl.yaml" ]]; then echo "$dir/feature.wsl.yaml"; return; fi
        if [[ -f "$dir/feature.linux.yaml" ]]; then echo "$dir/feature.linux.yaml"; return; fi
    elif [[ "$LOADOUT_PLATFORM" == "linux" ]]; then
        if [[ -f "$dir/feature.linux.yaml" ]]; then echo "$dir/feature.linux.yaml"; return; fi
    else
        if [[ -f "$dir/feature.${LOADOUT_PLATFORM}.yaml" ]]; then
            echo "$dir/feature.${LOADOUT_PLATFORM}.yaml"; return
        fi
    fi
    echo ""
}

# _executor_feature_mode <feature>
# Return the mode of a feature by reading feature.yaml.
# Returns "declarative" if the file is not found or mode is not declared.
_executor_feature_mode() {
    local feature="$1"
    local feature_dir
    feature_dir=$(_executor_feature_dir "$feature") 2>/dev/null || { echo "declarative"; return 0; }
    local ffile="$feature_dir/feature.yaml"
    [[ ! -f "$ffile" ]] && { echo "declarative"; return 0; }
    local mode
    mode=$(yq eval '.mode // "declarative"' "$ffile" 2>/dev/null)
    echo "${mode:-declarative}"
}

# _executor_get_pkgs_from_meta <feature_file>
# Print package names (one per line) from a feature.yaml file.
# Supports string form ("tmux") and mapping form ({name: tmux, managed: false}).
_executor_get_pkgs_from_meta() {
    local meta_file="$1"
    [[ -z "$meta_file" ]] && return 0
    yq eval '.packages // [] | .[] | .name // .' \
        "$meta_file" 2>/dev/null
}

# _executor_pkg_managed <feature> <pkg_name>
# Return 0 if the package should be uninstalled on feature remove, 1 if managed:false.
# Checks both base and platform meta files.
_executor_pkg_managed() {
    local feature="$1"
    local pkg="$2"

    local meta_file
    meta_file=$(_executor_resolve_feature_file "$feature") || return 0

    local platform_meta
    platform_meta=$(_executor_resolve_platform_feature_file "$feature")

    # Check base + platform meta files for managed:false
    for f in "$meta_file" "$platform_meta"; do
        [[ -z "$f" ]] && continue
        local flag
        # yq v4: select item matching by name or string value, read .managed field
        # Note: do NOT use '.managed // true' — yq v4 treats false as falsy in //
        flag=$(yq eval \
            ".packages // [] | .[] | select((.name // .) == \"$pkg\") | .managed" \
            "$f" 2>/dev/null | head -n1)
        [[ "$flag" == "false" ]] && return 1
    done
    return 0
}

# _executor_get_runtimes_json <feature_file>
# Print runtimes array as JSON from a feature.yaml file. Returns [] if none.
_executor_get_runtimes_json() {
    local meta_file="$1"
    [[ -z "$meta_file" ]] && echo "[]" && return 0
    local result
    result=$(yq eval -o=json '.runtimes // []' "$meta_file" 2>/dev/null)
    echo "${result:-[]}"
}

# _executor_get_files_json <feature_file>
# Print files array as JSON from a feature.yaml file. Returns [] if none.
_executor_get_files_json() {
    local meta_file="$1"
    [[ -z "$meta_file" ]] && echo "[]" && return 0
    local result
    result=$(yq eval -o=json '.files // []' "$meta_file" 2>/dev/null)
    echo "${result:-[]}"
}

# ── Resource operations ───────────────────────────────────────────────────────

# _executor_apply_packages <feature> <feature_file> [<platform_feature_file>]
# Install all packages declared in feature.yaml and add them to the active state patch.
# Skips packages where backend_package_exists returns true.
_executor_apply_packages() {
    local feature="$1"
    local meta_file="$2"
    local platform_meta_file="${3:-}"

    # Collect package names from base + platform meta (deduplicated)
    local -a all_pkgs=()
    while IFS= read -r pkg; do
        [[ -n "$pkg" ]] && all_pkgs+=("$pkg")
    done < <(_executor_get_pkgs_from_meta "$meta_file")

    while IFS= read -r pkg; do
        [[ -n "$pkg" ]] && all_pkgs+=("$pkg")
    done < <(_executor_get_pkgs_from_meta "$platform_meta_file")

    # Deduplicate (guard: printf '%s\n' on an empty array emits a blank line in bash,
    # which would pass "" as a package name to resolve_backend_for).
    ((${#all_pkgs[@]} == 0)) && return 0
    local -a unique_pkgs
    readarray -t unique_pkgs < <(printf '%s\n' "${all_pkgs[@]}" | grep -v '^[[:space:]]*$' | sort -u)
    ((${#unique_pkgs[@]} == 0)) && return 0

    for pkg in "${unique_pkgs[@]}"; do
        local backend
        backend=$(resolve_backend_for "package" "$pkg") || return 1
        load_backend "$backend" || return 1

        if backend_call "package_exists" "$pkg" 2>/dev/null; then
            log_info "    package already installed: $pkg"
        else
            log_info "    installing package: $pkg"
            backend_call "install_package" "$pkg" || {
                log_error "executor: failed to install package: $pkg"
                return 1
            }
        fi

        local resource
        resource=$(jq -n \
            --arg name "$pkg" \
            --arg backend "$backend" '
            {kind: "package", id: ("pkg:" + $name), backend: $backend,
             package: {name: $name, version: null}}
        ')
        state_patch_add_resource "$feature" "$resource" || return 1
    done
}

# _executor_apply_runtimes <feature> <feature_file> <platform_feature_file> <config_version>
# Install all runtimes declared in feature.yaml and add them to the active state patch.
# Runtime version resolution order:
#   1. Explicit version in feature.yaml ({name: rust-analyzer, version: "2025-05-26"})
#   2. config_version from profile (for the primary runtime, no explicit version)
#   3. "latest" as fallback
_executor_apply_runtimes() {
    local feature="$1"
    local meta_file="$2"
    local platform_meta_file="${3:-}"
    local config_version="${4:-}"

    # Merge runtimes from base + platform meta (deduplicated by name)
    local base_rts platform_rts merged_rts
    base_rts=$(_executor_get_runtimes_json "$meta_file")
    platform_rts=$(_executor_get_runtimes_json "$platform_meta_file")
    merged_rts=$(jq -n --argjson a "$base_rts" --argjson b "$platform_rts" \
        '($a + $b) | unique_by(.name)')

    local rt_count
    rt_count=$(echo "$merged_rts" | jq 'length')
    ((rt_count == 0)) && return 0

    local rt_idx
    for ((rt_idx = 0; rt_idx < rt_count; rt_idx++)); do
        local rt_entry rt_name rt_meta_ver rt_version
        rt_entry=$(echo "$merged_rts" | jq --argjson i "$rt_idx" '.[$i]')
        rt_name=$(echo "$rt_entry" | jq -r '.name')
        rt_meta_ver=$(echo "$rt_entry" | jq -r '.version // empty')

        # Version resolution
        if [[ -n "$rt_meta_ver" ]]; then
            rt_version="$rt_meta_ver"
        elif [[ -n "$config_version" ]]; then
            rt_version="$config_version"
        else
            rt_version="latest"
        fi

        local backend
        backend=$(resolve_backend_for "runtime" "$rt_name") || return 1
        load_backend "$backend" || return 1

        local actual_version="$rt_version"
        if backend_call "runtime_exists" "$rt_name" "$rt_version" 2>/dev/null; then
            log_info "    runtime already installed: $rt_name@$rt_version"
        else
            log_info "    installing runtime: $rt_name@$rt_version"
            actual_version=$(backend_call "install_runtime" "$rt_name" "$rt_version") || {
                log_error "executor: failed to install runtime: $rt_name@$rt_version"
                return 1
            }
            [[ -z "$actual_version" ]] && actual_version="$rt_version"
        fi

        local resource
        resource=$(jq -n \
            --arg name "$rt_name" \
            --arg ver "$actual_version" \
            --arg backend "$backend" '
            {kind: "runtime", id: ("rt:" + $name + "@" + $ver), backend: $backend,
             runtime: {name: $name, version: $ver}}
        ')
        state_patch_add_resource "$feature" "$resource" || return 1
    done
}

# _executor_deploy_files <feature> <feature_file> [<platform_feature_file>]
# Deploy files declared in feature.yaml and add fs resources to the active state patch.
# Supports op: link (symlink with copy fallback) and op: copy.
_executor_deploy_files() {
    local feature="$1"
    local feature_dir
    feature_dir=$(_executor_feature_dir "$feature") || return 1
    local meta_file="$2"
    local platform_meta_file="${3:-}"

    # Merge files from base + platform meta
    local base_files platform_files merged_files
    base_files=$(_executor_get_files_json "$meta_file")
    platform_files=$(_executor_get_files_json "$platform_meta_file")
    merged_files=$(jq -n --argjson a "$base_files" --argjson b "$platform_files" '$a + $b')

    local file_count
    file_count=$(echo "$merged_files" | jq 'length')
    ((file_count == 0)) && return 0

    local file_idx
    for ((file_idx = 0; file_idx < file_count; file_idx++)); do
        local entry src_rel op target
        entry=$(echo "$merged_files" | jq --argjson i "$file_idx" '.[$i]')
        src_rel=$(echo "$entry" | jq -r '.src')
        op=$(echo "$entry" | jq -r '.op // "link"')
        # Expand ~ in target path
        target=$(echo "$entry" | jq -r '.target' | sed "s|^~|$HOME|")

        local src="$feature_dir/files/$src_rel"
        if [[ ! -e "$src" ]]; then
            log_error "executor: file source not found: $src"
            return 1
        fi

        # Ensure parent directory exists
        local parent
        parent="$(dirname "$target")"
        [[ ! -d "$parent" ]] && mkdir -p "$parent"

        # Handle conflict: remove if managed, fail otherwise
        if [[ -e "$target" ]] || [[ -L "$target" ]]; then
            if state_has_file "$target"; then
                rm -rf "$target"
            else
                log_error "executor: path exists and is not managed: $target"
                return 1
            fi
        fi

        # Deploy
        if [[ "$op" == "link" ]]; then
            if ln -s "$src" "$target" 2>/dev/null; then
                log_success "  Linked $target"
            else
                cp -r "$src" "$target" || {
                    log_error "executor: copy fallback failed for: $target"
                    return 1
                }
                log_success "  Copied $target (link fallback)"
            fi
        else
            cp -r "$src" "$target" || {
                log_error "executor: copy failed for: $target"
                return 1
            }
            log_success "  Copied $target"
        fi

        # Detect actual entry_type from filesystem
        local entry_type actual_op
        if [[ -L "$target" ]]; then
            entry_type="symlink"; actual_op="link"
        elif [[ -d "$target" ]]; then
            entry_type="dir";     actual_op="copy"
        else
            entry_type="file";    actual_op="copy"
        fi

        local resource
        resource=$(jq -n \
            --arg path "$target" \
            --arg et "$entry_type" \
            --arg op "$actual_op" '
            {kind: "fs", id: ("fs:" + $path),
             fs: {path: $path, entry_type: $et, op: $op}}
        ')
        state_patch_add_resource "$feature" "$resource" || return 1
    done
}

# _executor_remove_resources <feature>
# Reverse of apply: reads current state and removes fs/runtime/package resources.
#
# Removal order (reverse of install): secondary-pkgs skipped, files → runtimes → packages
# Skip rules:
#   - Resources with backend="unknown" are NOT backend-uninstalled (legacy / pre-Phase4)
#   - Packages with managed:false in feature.yaml are NOT uninstalled
_executor_remove_resources() {
    local feature="$1"

    state_has_feature "$feature" || return 0

    local resources
    resources=$(state_query_resources "$feature")
    local rc
    rc=$(echo "$resources" | jq 'length')
    ((rc == 0)) && return 0

    # 1. Remove fs resources (files / dirs / symlinks)
    local resource_idx
    for ((resource_idx = 0; resource_idx < rc; resource_idx++)); do
        local res kind
        res=$(echo "$resources" | jq --argjson i "$resource_idx" '.[$i]')
        kind=$(echo "$res" | jq -r '.kind')
        [[ "$kind" != "fs" ]] && continue

        local path
        path=$(echo "$res" | jq -r '.fs.path')
        if [[ -e "$path" ]] || [[ -L "$path" ]]; then
            log_info "    removing: $path"
            rm -rf "$path"
        fi
    done

    # 2. Uninstall managed runtimes (backend != "unknown")
    for ((resource_idx = 0; resource_idx < rc; resource_idx++)); do
        local res kind backend
        res=$(echo "$resources" | jq --argjson i "$resource_idx" '.[$i]')
        kind=$(echo "$res" | jq -r '.kind')
        [[ "$kind" != "runtime" ]] && continue
        backend=$(echo "$res" | jq -r '.backend // "unknown"')
        [[ "$backend" == "unknown" ]] && continue

        local rt_name rt_ver
        rt_name=$(echo "$res" | jq -r '.runtime.name')
        rt_ver=$(echo "$res" | jq -r '.runtime.version // ""')
        log_info "    uninstalling runtime: $rt_name@$rt_ver"
        load_backend "$backend" || { log_warn "    backend load failed: $backend (skipping)"; continue; }
        backend_call "uninstall_runtime" "$rt_name" "$rt_ver" || \
            log_warn "    uninstall_runtime failed for $rt_name@$rt_ver (continuing)"
    done

    # 3. Uninstall managed packages (backend != "unknown", managed != false)
    for ((resource_idx = 0; resource_idx < rc; resource_idx++)); do
        local res kind backend
        res=$(echo "$resources" | jq --argjson i "$resource_idx" '.[$i]')
        kind=$(echo "$res" | jq -r '.kind')
        [[ "$kind" != "package" ]] && continue
        backend=$(echo "$res" | jq -r '.backend // "unknown"')
        [[ "$backend" == "unknown" ]] && continue

        local pkg_name
        pkg_name=$(echo "$res" | jq -r '.package.name')
        _executor_pkg_managed "$feature" "$pkg_name" || {
            log_info "    skipping unmanaged package: $pkg_name"
            continue
        }

        log_info "    uninstalling package: $pkg_name"
        load_backend "$backend" || { log_warn "    backend load failed: $backend (skipping)"; continue; }
        backend_call "uninstall_package" "$pkg_name" || \
            log_warn "    uninstall_package failed for $pkg_name (continuing)"
    done
}

# ── Feature operations ────────────────────────────────────────────────────────

# _executor_run_script <script_path> [env_vars...]
# Run a feature script as a subprocess.
# Extra arguments are exported as environment variables for the subprocess.
_executor_run_script() {
    local script="$1"
    shift

    if [[ ! -f "$script" ]]; then
        log_error "executor: script not found: $script"
        return 1
    fi

    local -a env_args=()
    for arg in "$@"; do
        env_args+=("$arg")
    done

    if [[ ${#env_args[@]} -gt 0 ]]; then
        env "${env_args[@]}" bash "$script"
    else
        bash "$script"
    fi
}

# _executor_install <feature> <config_version>
# Full install pipeline:
#   1. Resolve meta files
#   2. state_patch_begin
#   3. Install packages/runtimes/files from feature.yaml → state_patch
#   4. state_patch_finalize
#   5. Run install.sh script (for secondary pkg setup: npm/uv/bootstrap)
_executor_install() {
    local feature="$1"
    local config_version="${2:-}"
    local feature_dir
    feature_dir=$(_executor_feature_dir "$feature") || return 1

    log_info "Installing: $feature"

    local meta_file platform_meta
    meta_file=$(_executor_resolve_feature_file "$feature") || return 1
    platform_meta=$(_executor_resolve_platform_feature_file "$feature")

    # Begin patch accumulation
    state_patch_begin || return 1

    _executor_apply_packages "$feature" "$meta_file" "$platform_meta" || {
        log_error "executor: package installation failed for: $feature"
        return 1
    }

    _executor_apply_runtimes "$feature" "$meta_file" "$platform_meta" "$config_version" || {
        log_error "executor: runtime installation failed for: $feature"
        return 1
    }

    _executor_deploy_files "$feature" "$meta_file" "$platform_meta" || {
        log_error "executor: file deployment failed for: $feature"
        return 1
    }

    # Commit packages/runtimes/files to state; install.sh subprocess will
    # inherit the updated state and may further commit secondary packages.
    state_patch_finalize || return 1

    # Run install.sh for remaining setup (npm/uv packages, bootstrap logic, etc.)
    local script="$feature_dir/install.sh"
    if [[ -f "$script" ]]; then
        local -a env_args=()
        [[ -n "$config_version" ]] && env_args+=("LOADOUT_FEATURE_CONFIG_VERSION=$config_version")
        if ! _executor_run_script "$script" "${env_args[@]}"; then
            log_error "executor: install script failed for: $feature"
            return 1
        fi
    fi
}

# _executor_destroy <feature>
# Full destroy pipeline:
#   1. Remove resources tracked in state (fs + managed runtimes/packages)
#   2. Run uninstall.sh script (for secondary pkg cleanup: npm/uv etc.)
#   3. state_patch_begin → state_patch_remove_feature → state_patch_finalize
_executor_destroy() {
    local feature="$1"
    local feature_dir
    feature_dir=$(_executor_feature_dir "$feature") || return 1

    log_info "Destroying: $feature"

    # Remove resources tracked in state
    _executor_remove_resources "$feature" || return 1

    # Run uninstall.sh for remaining cleanup (npm/uv uninstall etc.)
    local script="$feature_dir/uninstall.sh"
    if [[ -f "$script" ]]; then
        if ! _executor_run_script "$script"; then
            log_error "executor: uninstall script failed for: $feature"
            return 1
        fi
    fi

    # Remove feature entry from state
    state_patch_begin || return 1
    state_patch_remove_feature "$feature" || return 1
    state_patch_finalize || return 1
}

# _executor_replace <feature> <config_version>
# Replace = destroy resources + uninstall script + state remove, then full install.
_executor_replace() {
    local feature="$1"
    local config_version="${2:-}"
    local feature_dir
    feature_dir=$(_executor_feature_dir "$feature") || return 1

    log_info "Replacing: $feature"

    # Destroy phase (resources + script)
    _executor_remove_resources "$feature" || return 1

    local uninstall_script="$feature_dir/uninstall.sh"
    if [[ -f "$uninstall_script" ]]; then
        if ! _executor_run_script "$uninstall_script"; then
            log_error "executor: uninstall script failed during replace for: $feature"
            return 1
        fi
    fi

    # Remove state entry so install can start fresh
    state_patch_begin || return 1
    state_patch_remove_feature "$feature" || return 1
    state_patch_finalize || return 1

    # Install phase (full feature.yaml + script)
    _executor_install "$feature" "$config_version" || return 1
}

# ── Plan reporting ────────────────────────────────────────────────────────────

# _executor_report_blocked <plan_json>
# Log blocked features (they are skipped, not aborted).
_executor_report_blocked() {
    local plan_json="$1"
    local count
    count=$(echo "$plan_json" | jq '.blocked | length')
    if [[ "$count" -gt 0 ]]; then
        log_warn "Skipping $count blocked feature(s):"
        while IFS= read -r line; do
            log_warn "  ⊘ $line"
        done < <(echo "$plan_json" | jq -r '.blocked[] | "\(.feature): \(.reason)"')
    fi
}

# _executor_report_summary <plan_json>
# Print plan action summary to log.
_executor_report_summary() {
    local plan_json="$1"
    local create destroy replace blocked noop
    create=$(echo "$plan_json" | jq '.summary.create')
    destroy=$(echo "$plan_json" | jq '.summary.destroy')
    replace=$(echo "$plan_json" | jq '.summary.replace')
    blocked=$(echo "$plan_json" | jq '.summary.blocked')
    noop=$(echo "$plan_json" | jq '.summary.noop')
    log_info "Plan: create=$create  destroy=$destroy  replace=$replace  noop=$noop  blocked=$blocked"
}

# ── Public API ────────────────────────────────────────────────────────────────

# executor_run <plan_json>
# Execute all actions in plan_json.
#
# Blocked features are reported and skipped.
# Any action failure causes immediate abort (non-zero exit).
executor_run() {
    local plan_json="$1"

    if [[ -z "$plan_json" ]]; then
        log_error "executor_run: plan_json is required"
        return 1
    fi

    # Validate plan is parseable
    if ! echo "$plan_json" | jq empty 2>/dev/null; then
        log_error "executor_run: plan_json is not valid JSON"
        return 1
    fi

    _executor_report_blocked "$plan_json"
    _executor_report_summary "$plan_json"

    local action_count
    action_count=$(echo "$plan_json" | jq '.actions | length')

    if [[ "$action_count" -eq 0 ]]; then
        log_info "Nothing to do."
        return 0
    fi

    log_task "Executing plan ($action_count actions)..."
    # Log action list for diagnostics
    log_info "Actions: $(echo "$plan_json" | jq -r '.actions[] | "\(.operation) \(.feature)"' | tr '\n' ',' | sed 's/,$//g')"

    local action_idx
    for ((action_idx = 0; action_idx < action_count; action_idx++)); do
        local action feature operation config_version action_details feature_mode
        action=$(echo "$plan_json" | jq --argjson i "$action_idx" '.actions[$i]')
        feature=$(echo "$action" | jq -r '.feature')
        operation=$(echo "$action" | jq -r '.operation')
        config_version=$(echo "$action" | jq -r '.details.config_version // empty')
        action_details=$(echo "$action" | jq -c '.details // {}')

        # Determine execution path: declarative features use declarative_executor_run
        feature_mode=$(_executor_feature_mode "$feature")

        case "$operation" in
            destroy)
                if [[ "$feature_mode" == "declarative" ]]; then
                    declarative_executor_run "$feature" "destroy" "$action_details" || return 1
                else
                    _executor_destroy "$feature" || return 1
                fi
                ;;
            create)
                if [[ "$feature_mode" == "declarative" ]]; then
                    declarative_executor_run "$feature" "create" "$action_details" || return 1
                else
                    _executor_install "$feature" "$config_version" || return 1
                fi
                ;;
            replace|replace_backend)
                if [[ "$feature_mode" == "declarative" ]]; then
                    declarative_executor_run "$feature" "$operation" "$action_details" || return 1
                else
                    _executor_replace "$feature" "$config_version" || return 1
                fi
                ;;
            strengthen)
                if [[ "$feature_mode" == "declarative" ]]; then
                    declarative_executor_run "$feature" "strengthen" "$action_details" || return 1
                else
                    log_error "executor: strengthen is not supported for script-mode features"
                    return 1
                fi
                ;;
            *)
                log_error "executor: unknown operation '$operation' for feature '$feature'"
                return 1
                ;;
        esac
    done

    return 0
}
