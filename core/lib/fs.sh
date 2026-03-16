#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Module: fs
#
# Responsibility:
#   Provide file system operations for feature installation.
#
# Public API (Stable):
#   ensure_dir <path>
#   backup_file <target>
#   backup_dir <target>
#   link_file <feature> <src> <dst>
#   link_dir <feature> <src> <dst>
#   remove_tracked_files <feature>
# -----------------------------------------------------------------------------

# This library expects core/lib/logger.sh and core/lib/state.sh to be sourced by the caller.

# ensure_dir <path>
# Create directory if it does not exist.
ensure_dir() {
    local path="$1"

    if [[ -z "$path" ]]; then
        log_error "ensure_dir: path is required"
        return 1
    fi

    if [[ ! -d "$path" ]]; then
        mkdir -p "$path"
    fi
}

# backup_file <target>
# Backup existing file with timestamp if it exists and is not a symlink.
backup_file() {
    local target="$1"

    if [[ -z "$target" ]]; then
        log_error "backup_file: target is required"
        return 1
    fi

    if [[ -f "$target" ]] && [[ ! -L "$target" ]]; then
        local backup_path="${target}.backup"
        if [[ -e "$backup_path" ]]; then
            backup_path="${target}.backup.$(date +%Y%m%d%H%M%S)"
        fi
        log_warn "Backing up existing $target to $backup_path"
        mv "$target" "$backup_path"
    fi
}

# backup_dir <target>
# Backup existing directory with timestamp if it exists and is not a symlink.
backup_dir() {
    local target="$1"

    if [[ -z "$target" ]]; then
        log_error "backup_dir: target is required"
        return 1
    fi

    if [[ -d "$target" ]] && [[ ! -L "$target" ]]; then
        local backup_path="${target}.backup.$(date +%Y%m%d%H%M%S)"
        log_warn "Backing up existing directory $target to $backup_path"
        mv "$target" "$backup_path"
    fi
}

# link_file <feature> <src> <dst>
# Link a file to dst and register to state.
# Attempts symbolic link; falls back to copy if not supported.
link_file() {
    local feature="$1"
    local src="$2"
    local dst="$3"

    if [[ -z "$feature" ]] || [[ -z "$src" ]] || [[ -z "$dst" ]]; then
        log_error "link_file: feature, src, and dst are required"
        return 1
    fi

    if [[ ! -f "$src" ]]; then
        log_error "link_file: source file not found: $src"
        return 1
    fi

    _fs_ensure_parent_dir "$dst" || return 1
    _fs_ensure_not_conflicting "$dst" || return 1

    if ! _fs_try_symlink "$src" "$dst"; then
        # Fallback to copy
        cp -f "$src" "$dst" || { log_error "link_file: copy failed: $dst"; return 1; }
    fi

    state_add_file "$feature" "$dst"
    log_success "Linked $dst"
}

# link_dir <feature> <src> <dst>
# Link a directory to dst and register to state.
# Attempts symbolic link; falls back to copy if not supported.
link_dir() {
    local feature="$1"
    local src="$2"
    local dst="$3"

    if [[ -z "$feature" ]] || [[ -z "$src" ]] || [[ -z "$dst" ]]; then
        log_error "link_dir: feature, src, and dst are required"
        return 1
    fi

    if [[ ! -d "$src" ]]; then
        log_error "link_dir: source directory not found: $src"
        return 1
    fi

    _fs_ensure_parent_dir "$dst" || return 1
    _fs_ensure_not_conflicting "$dst" || return 1

    if ! _fs_try_symlink "$src" "$dst"; then
        # Fallback to copy
        cp -rf "$src" "$dst" || { log_error "link_dir: copy failed: $dst"; return 1; }
    fi

    state_add_file "$feature" "$dst"
    log_success "Linked $dst"
}

# remove_tracked_files <feature>
# Remove all files tracked by a feature from state.
remove_tracked_files() {
    local feature="$1"

    if [[ -z "$feature" ]]; then
        log_error "remove_tracked_files: feature is required"
        return 1
    fi

    log_info "Removing configuration files..."
    while IFS= read -r file; do
        if [[ -z "$file" ]]; then continue; fi

        if [[ ! -e "$file" ]] && [[ ! -L "$file" ]]; then
            log_info "Path does not exist, skipping: $file"
            continue
        fi

        log_info "Removing: $file"
        rm -rf "$file"
    done < <(state_get_files "$feature")
}

# -----------------------------------------------------------------------------
# Internal helpers
# -----------------------------------------------------------------------------

# _fs_ensure_parent_dir <path>
# Create parent directory of path if it does not exist.
_fs_ensure_parent_dir() {
    local parent
    parent="$(dirname "$1")"
    if [[ ! -d "$parent" ]]; then
        mkdir -p "$parent"
    fi
}

# _fs_ensure_not_conflicting <path>
# Fail if path exists and is not managed by state.
# If it is managed, remove it so the caller can replace it.
_fs_ensure_not_conflicting() {
    local path="$1"

    if [[ -e "$path" ]] || [[ -L "$path" ]]; then
        if state_has_file "$path"; then
            rm -rf "$path"
        else
            log_error "Path exists and is not managed: $path"
            return 1
        fi
    fi
}

# _fs_try_symlink <src> <dst>
# Attempt to create a symbolic link. Returns 0 on success, 1 on failure.
_fs_try_symlink() {
    local src="$1"
    local dst="$2"

    ln -s "$src" "$dst" 2>/dev/null
}
