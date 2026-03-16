#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Module: repo
#
# Responsibility:
#   Provide repository-based tool installation utilities.
#   Manages cloning of source repositories and path resolution for
#   locally installed tools (not managed by a package manager).
#
# Convention:
#   Source repositories are cloned under ~/.local/src/<tool>
#   Tool binaries are placed under ~/.local/bin/<tool>
#
# Public API (Stable):
#   clone_repository <feature> <repo_url> <dest_path>
#   resolve_tool_path <tool_name>
#   is_tool_installed <tool_name>
# -----------------------------------------------------------------------------

# This library expects core/lib/logger.sh, core/lib/state.sh, and
# core/lib/runner.sh to be sourced by the caller.

# clone_repository <feature> <repo_url> <dest_path>
# Clone a git repository to dest_path, or pull if it already exists.
# Registers the destination directory to feature state for uninstall tracking.
clone_repository() {
    local feature="$1"
    local repo_url="$2"
    local dest_path="$3"

    if [[ -z "$feature" ]] || [[ -z "$repo_url" ]] || [[ -z "$dest_path" ]]; then
        log_error "clone_repository: feature, repo_url, and dest_path are required"
        return 1
    fi

    require_command git || return 1

    if [[ -d "$dest_path/.git" ]]; then
        log_info "Repository already cloned, pulling: $dest_path"
        git -C "$dest_path" pull --ff-only || {
            log_error "clone_repository: git pull failed: $dest_path"
            return 1
        }
    else
        local parent_dir
        parent_dir="$(dirname "$dest_path")"
        if [[ ! -d "$parent_dir" ]]; then
            mkdir -p "$parent_dir"
        fi

        log_info "Cloning $repo_url into $dest_path"
        git clone "$repo_url" "$dest_path" || {
            log_error "clone_repository: git clone failed: $repo_url"
            return 1
        }
    fi

    state_add_file "$feature" "$dest_path"
    log_success "Repository ready: $dest_path"
}

# resolve_tool_path <tool_name>
# Returns the canonical install path for a locally managed tool binary.
# Output: ~/.local/bin/<tool_name>
resolve_tool_path() {
    local tool_name="$1"

    if [[ -z "$tool_name" ]]; then
        log_error "resolve_tool_path: tool_name is required"
        return 1
    fi

    echo "${HOME}/.local/bin/${tool_name}"
}

# is_tool_installed <tool_name>
# Check if a tool exists at the local install path (~/.local/bin/<tool_name>).
# Returns 0 if installed, 1 otherwise.
is_tool_installed() {
    local tool_name="$1"

    if [[ -z "$tool_name" ]]; then
        log_error "is_tool_installed: tool_name is required"
        return 1
    fi

    local tool_path
    tool_path="$(resolve_tool_path "$tool_name")"
    [[ -x "$tool_path" ]]
}
