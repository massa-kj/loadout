#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Module: state
#
# Responsibility:
#   Manage state file (v3) with atomic writes, migration, and patch operations.
#
# Stable Public API:
#   state_load
#   state_validate <mode>                     mode = load | execute
#   state_commit_atomic <json>
#   state_query_feature <feature>
#   state_query_resources <feature>
#   state_patch_begin
#   state_patch_add_resource <feature> <resource_json>
#   state_patch_remove_feature <feature>
#   state_patch_finalize
#   state_migrate_v2_to_v3                    — called by cmd/migrate.sh
#
# Compat API:
#   state_init                                — keep (used by scripts)
#   state_has_feature <feature>               — keep
#   state_list_features                       — keep
#   state_get_files <feature>                 — keep
#   state_has_file <path>                     — keep
#   state_add_file <feature> <path>           — keep (git gitconfig complex merge)
#   state_get_runtime <feature> <key>         — keep (read-only)
# -----------------------------------------------------------------------------

# ── Private state ─────────────────────────────────────────────────────────────

# In-memory cache of the authoritative state (JSON string).
declare -g _STATE_JSON=""

# Working copy for patch operations.
declare -g _STATE_PATCH_JSON=""

# ── Internal helpers ──────────────────────────────────────────────────────────

# _state_ensure_loaded
# Guard: run state_load if cache is empty.
_state_ensure_loaded() {
    if [[ -z "$_STATE_JSON" ]]; then
        state_load || return 1
    fi
}

# _state_file_path
# Resolve authoritative state file path from env module.
_state_file_path() {
    if declare -F loadout_state_file_path >/dev/null 2>&1; then
        loadout_state_file_path
        return 0
    fi
    log_error "_state_file_path: loadout_state_file_path is not available"
    return 1
}

    # _state_normalize_feature_id <feature>
    # Normalize bare feature names to canonical IDs for compat APIs.
    _state_normalize_feature_id() {
        local feature="$1"

        if [[ -z "$feature" ]]; then
            log_error "_state_normalize_feature_id: feature name is required"
            return 1
        fi

        if declare -F canonical_id_normalize >/dev/null 2>&1; then
            canonical_id_normalize "$feature" "core"
            return $?
        fi

        if [[ "$feature" == */* ]]; then
            echo "$feature"
        else
            echo "core/$feature"
        fi
    }

# ── Stable Core API ───────────────────────────────────────────────────────────

# state_load
# Load state from disk into in-memory cache.
# Creates empty v3 state if the file does not exist.
state_load() {
    local path
    path="$(_state_file_path)" || return 1
    local dir
    dir="$(dirname "$path")"

    # Create state directory if missing
    if [[ ! -d "$dir" ]]; then
        mkdir -p "$dir"
    fi

    # Create empty v3 state if file does not exist
    if [[ ! -f "$path" ]]; then
        _STATE_JSON='{"version":3,"features":{}}'
        echo "$_STATE_JSON" > "$path"
        return 0
    fi

    # Validate JSON parsability
    if ! jq empty "$path" 2>/dev/null; then
        log_error "state_load: state file is not valid JSON: $path"
        return 1
    fi

    _STATE_JSON="$(cat "$path")"

    local ver
    ver=$(echo "$_STATE_JSON" | jq -r '.version // empty')

    if [[ "$ver" == "3" ]]; then
        return 0
    elif [[ "$ver" == "1" || "$ver" == "2" ]]; then
        log_error "state_load: state is at version ${ver}, which requires migration."
        log_error "  Run: loadout migrate"
        return 1
    else
        log_error "state_load: unknown state version: ${ver:-<missing>}"
        return 1
    fi
}

# state_validate <mode> [json]
# Validate structural invariants.
#   mode=load    – allow unknown resource kinds; check structural sanity.
#   mode=execute – additionally abort on features containing unknown kinds.
# If [json] is omitted, the in-memory cache is validated.
state_validate() {
    local mode="${1:-load}"
    local json="${2:-$_STATE_JSON}"

    if [[ -z "$json" ]]; then
        log_error "state_validate: no state loaded"
        return 1
    fi

    # 1. JSON validity
    if ! echo "$json" | jq empty 2>/dev/null; then
        log_error "state_validate: invalid JSON"
        return 1
    fi

    # 2. version MUST be 3
    local ver
    ver=$(echo "$json" | jq -r '.version')
    if [[ "$ver" != "3" ]]; then
        log_error "state_validate: version MUST be 3, got: $ver"
        return 1
    fi

    # 3. features MUST be an object
    if ! echo "$json" | jq -e '.features | type == "object"' >/dev/null 2>&1; then
        log_error "state_validate: .features must be an object"
        return 1
    fi

    # 4. Each feature MUST have a resources array; each resource MUST have kind and id
    local missing
    missing=$(echo "$json" | jq -r '
      .features | to_entries[] |
      . as $f |
      if (.value.resources | type) != "array" then
        "feature \(.key): resources must be an array"
      else
        (.value.resources[] |
          if (.kind == null or .kind == "") or (.id == null or .id == "") then
            "feature \($f.key): resource missing kind or id"
          else empty end)
      end
    ' 2>/dev/null)
    if [[ -n "$missing" ]]; then
        log_error "state_validate: $missing"
        return 1
    fi

    # 5. Within a feature: no duplicate resource.id
    local dup_res
    dup_res=$(echo "$json" | jq -r '
      .features | to_entries[] |
      . as $f |
      (.value.resources | map(.id) | group_by(.) | map(select(length > 1)) | .[]) |
      "feature \($f.key): duplicate resource id: \(.[0])"
    ' 2>/dev/null)
    if [[ -n "$dup_res" ]]; then
        log_error "state_validate: $dup_res"
        return 1
    fi

    # 6. Across all features: no duplicate fs.path
    local dup_path
    dup_path=$(echo "$json" | jq -r '
      [.features | to_entries[] | .value.resources[] |
        select(.kind == "fs") | .fs.path] |
      group_by(.) | map(select(length > 1)) | .[] | .[0] |
      "duplicate fs.path across features: \(.)"
    ' 2>/dev/null)
    if [[ -n "$dup_path" ]]; then
        log_error "state_validate: $dup_path"
        return 1
    fi

    # 7. All fs.path MUST be absolute
    local nonabs
    nonabs=$(echo "$json" | jq -r '
      .features | to_entries[] | .value.resources[] |
      select(.kind == "fs" and (.fs.path | startswith("/") | not)) |
      "fs.path not absolute: \(.fs.path)"
    ' 2>/dev/null)
    if [[ -n "$nonabs" ]]; then
        log_error "state_validate: $nonabs"
        return 1
    fi

    # mode=execute: abort on features containing unknown kinds
    if [[ "$mode" == "execute" ]]; then
        local unknown
        unknown=$(echo "$json" | jq -r '
          .features | to_entries[] |
          . as $f |
          (.value.resources // [])[] |
          select([.kind] | inside(["package","runtime","fs"]) | not) |
          "feature \($f.key): unknown kind: \(.kind)"
        ' 2>/dev/null)
        if [[ -n "$unknown" ]]; then
            log_error "state_validate(execute): $unknown"
            return 1
        fi
    fi

    return 0
}

# state_commit_atomic <json>
# Write a new state atomically: write to .tmp → validate → atomic rename.
state_commit_atomic() {
    local new_json="$1"

    local path
    path="$(_state_file_path)" || return 1
    local tmp="${path}.tmp"

    if [[ -z "$new_json" ]]; then
        log_error "state_commit_atomic: empty JSON provided"
        return 1
    fi

    # Write to tmp (pretty-print for readability)
    if ! echo "$new_json" | jq '.' > "$tmp" 2>/dev/null; then
        log_error "state_commit_atomic: failed to write tmp file"
        rm -f "$tmp"
        return 1
    fi

    # Validate before committing
    local tmp_content
    tmp_content="$(cat "$tmp")"
    if ! state_validate "load" "$tmp_content"; then
        log_error "state_commit_atomic: validation failed, aborting commit"
        rm -f "$tmp"
        return 1
    fi

    # Atomic rename
    if ! mv "$tmp" "$path"; then
        log_error "state_commit_atomic: atomic rename failed"
        rm -f "$tmp"
        return 1
    fi

    # Update in-memory cache
    _STATE_JSON="$tmp_content"

    return 0
}

# state_query_feature <feature>
# Output the feature entry JSON (compact), or nothing if not found.
state_query_feature() {
    local feature="$1"
    if [[ -z "$feature" ]]; then
        log_error "state_query_feature: feature name is required"
        return 1
    fi
        feature=$(_state_normalize_feature_id "$feature") || return 1
    _state_ensure_loaded || return 1
    echo "$_STATE_JSON" | jq -c --arg f "$feature" '.features[$f] // empty'
}

# state_query_resources <feature>
# Output the resources array JSON for a feature, or [] if not found.
state_query_resources() {
    local feature="$1"
    if [[ -z "$feature" ]]; then
        log_error "state_query_resources: feature name is required"
        return 1
    fi
        feature=$(_state_normalize_feature_id "$feature") || return 1
    _state_ensure_loaded || return 1
    echo "$_STATE_JSON" | jq -c --arg f "$feature" '.features[$f].resources // []'
}

# ── Patch Operations ──────────────────────────────────────────────────────────

# state_patch_begin
# Initialize a patch working copy from the current state cache.
state_patch_begin() {
    _state_ensure_loaded || return 1
    _STATE_PATCH_JSON="$_STATE_JSON"
}

# state_patch_add_resource <feature> <resource_json>
# Add (or replace by id) a resource in the patch working copy.
# Creates the feature entry if it does not exist.
state_patch_add_resource() {
    local feature="$1"
    local resource_json="$2"

    if [[ -z "$feature" ]] || [[ -z "$resource_json" ]]; then
        log_error "state_patch_add_resource: feature and resource_json are required"
        return 1
    fi
        feature=$(_state_normalize_feature_id "$feature") || return 1

    if [[ -z "$_STATE_PATCH_JSON" ]]; then
        log_error "state_patch_add_resource: no patch in progress; call state_patch_begin first"
        return 1
    fi

    _STATE_PATCH_JSON=$(echo "$_STATE_PATCH_JSON" | jq \
        --arg f "$feature" \
        --argjson res "$resource_json" '
        if .features[$f] == null then
            .features[$f] = {"resources": []}
        else . end |
        .features[$f].resources = (
            [.features[$f].resources[] | select(.id != $res.id)] + [$res]
        )
    ')
}

# state_patch_remove_feature <feature>
# Remove a feature entry from the patch working copy.
state_patch_remove_feature() {
    local feature="$1"

    if [[ -z "$feature" ]]; then
        log_error "state_patch_remove_feature: feature name is required"
        return 1
    fi
        feature=$(_state_normalize_feature_id "$feature") || return 1

    if [[ -z "$_STATE_PATCH_JSON" ]]; then
        log_error "state_patch_remove_feature: no patch in progress; call state_patch_begin first"
        return 1
    fi

    _STATE_PATCH_JSON=$(echo "$_STATE_PATCH_JSON" | jq --arg f "$feature" 'del(.features[$f])')
}

# state_patch_finalize
# Commit the patch working copy atomically and clear the buffer.
state_patch_finalize() {
    if [[ -z "$_STATE_PATCH_JSON" ]]; then
        log_error "state_patch_finalize: no patch in progress"
        return 1
    fi

    if ! state_commit_atomic "$_STATE_PATCH_JSON"; then
        _STATE_PATCH_JSON=""
        return 1
    fi

    _STATE_PATCH_JSON=""
    return 0
}

# ── Migration ─────────────────────────────────────────────────────────────────

# _state_transform_v2_to_v3 <v2_json>
# Pure transformation: v2 JSON (bare feature keys) → v3 JSON (canonical IDs).
# All bare names are prefixed with "core/". Already-canonical keys are unchanged.
# Outputs v3 JSON to stdout.
_state_transform_v2_to_v3() {
    local v2_json="$1"
    echo "$v2_json" | jq '
        {
            version: 3,
            features: (
                .features | to_entries | map(
                    if (.key | contains("/")) then .
                    else .key = "core/" + .key
                    end
                ) | from_entries
            )
        }
    '
}

# state_migrate_v2_to_v3
# Migrate v2 state (bare feature keys) to v3 (canonical IDs).
# Must be called with _STATE_JSON populated with valid v2 state.
# Performs: timestamped backup → transform → validate → atomic commit.
# Called exclusively by cmd/migrate.sh; NOT called automatically by state_load.
state_migrate_v2_to_v3() {
    local path
    path="$(_state_file_path)" || return 1
    local timestamp
    timestamp=$(date +%Y%m%d_%H%M%S)
    local backup="${path}.bak.${timestamp}"

    # Backup current file
    if [[ -f "$path" ]]; then
        cp "$path" "$backup" || {
            log_error "state_migrate_v2_to_v3: failed to create backup at $backup"
            return 1
        }
        log_info "state_migrate_v2_to_v3: backup created: $backup"
    fi

    # Transform
    local v3_json
    if ! v3_json=$(_state_transform_v2_to_v3 "$_STATE_JSON"); then
        log_error "state_migrate_v2_to_v3: transformation failed; restore from: $backup"
        return 1
    fi

    # Commit atomically (state_validate inside state_commit_atomic now checks v3)
    if ! state_commit_atomic "$v3_json"; then
        log_error "state_migrate_v2_to_v3: commit failed; restore from: $backup"
        return 1
    fi

    log_info "state_migrate_v2_to_v3: migration to v3 complete"
    return 0
}

# state_migrate (v1 → v2)
# Migrate the in-memory v1 state to v2: backup → transform → commit atomically.
# Called by cmd/migrate.sh for v1 state, before chaining into state_migrate_v2_to_v3.
state_migrate() {
    local path
    path="$(_state_file_path)" || return 1
    local backup="${path}.bak"

    # Backup current file
    if [[ -f "$path" ]]; then
        cp "$path" "$backup" || {
            log_error "state_migrate: failed to create backup at $backup"
            return 1
        }
        log_info "state_migrate: backup created: $backup"
    fi

    # Transform
    local v2_json
    if ! v2_json=$(_migrate_v1_to_v2 "$_STATE_JSON"); then
        log_error "state_migrate: transformation failed; restore from: $backup"
        return 1
    fi

    # Commit atomically
    if ! state_commit_atomic "$v2_json"; then
        log_error "state_migrate: commit failed; restore from: $backup"
        return 1
    fi

    log_info "state_migrate: migration to v2 complete"
    return 0
}
# Pure transformation: convert v1 schema to v2 resources format.
# Outputs v2 JSON to stdout.
#
# v1 → v2 mapping:
#   packages[]          → kind:package resources
#   files[]             → kind:fs resources  (entry_type/op inferred from filesystem)
#   runtime.version     → kind:runtime resource (runtime name = feature_id)
#   "{fid}@{rv}" entry  → skipped when runtime.version is set (captured by runtime resource)
_migrate_v1_to_v2() {
    local v1_json="$1"

    # ── Structural transformation (jq) ──────────────────────────────────────
    local intermediate
    intermediate=$(echo "$v1_json" | jq '
      {
        version: 2,
        features: (
          .features | to_entries | map(
            .key as $fid |
            (.value.runtime.version // null) as $rv |
            {
              key: $fid,
              value: {
                resources: (
                  # ── package resources ─────────────────────────────────────
                  # Skip entries matching "{feature_id}@{runtime_version}"
                  # because those are captured as kind:runtime below.
                  [
                    (.value.packages // [])[] |
                    . as $pkg |
                    if ($rv != null and $pkg == ($fid + "@" + $rv))
                    then empty
                    else {
                      kind: "package",
                      id: ("pkg:" + $pkg),
                      backend: "unknown",
                      package: { name: $pkg, version: null }
                    }
                    end
                  ] +

                  # ── fs resources ──────────────────────────────────────────
                  [
                    (.value.files // [])[] |
                    {
                      kind: "fs",
                      id: ("fs:" + .),
                      fs: {
                        path: .,
                        entry_type: "file",
                        op: "copy"
                      }
                    }
                  ] +

                  # ── runtime resource ──────────────────────────────────────
                  if $rv != null
                  then [{
                    kind: "runtime",
                    id: ("rt:" + $fid + "@" + $rv),
                    backend: "unknown",
                    runtime: { name: $fid, version: $rv }
                  }]
                  else []
                  end
                )
              }
            }
          ) | from_entries
        )
      }
    ') || return 1

    # ── Filesystem inspection pass ───────────────────────────────────────────
    # Refine entry_type and op by inspecting the real filesystem paths.
    local feature_id path entry_type op
    while IFS= read -r feature_id; do
        while IFS= read -r path; do
            if [[ -L "$path" ]]; then
                entry_type="symlink"
                op="link"
            elif [[ -d "$path" ]]; then
                entry_type="dir"
                op="copy"
            elif [[ -f "$path" ]]; then
                entry_type="file"
                op="copy"
            else
                # Path no longer exists; keep safe defaults
                entry_type="file"
                op="copy"
            fi

            intermediate=$(echo "$intermediate" | jq \
                --arg f "$feature_id" \
                --arg p "$path" \
                --arg et "$entry_type" \
                --arg op "$op" '
                .features[$f].resources = [
                    .features[$f].resources[] |
                    if .kind == "fs" and .fs.path == $p then
                        .fs.entry_type = $et | .fs.op = $op
                    else . end
                ]
            ') || return 1
        done < <(echo "$intermediate" | jq -r \
            --arg f "$feature_id" \
            '.features[$f].resources // [] | .[] | select(.kind == "fs") | .fs.path' \
            2>/dev/null)
    done < <(echo "$intermediate" | jq -r '.features | keys[]' 2>/dev/null)

    echo "$intermediate"
}

# ── Compat API ────────────────────────────────────────────────────────────────
# Compatibility API for Phase 4+ feature scripts.
#
# Status after Phase 4.5:
#   state_init, state_has_feature, state_list_features  → still used (keep)
#   state_get_files, state_has_file                     → still used (keep)
#   state_get_runtime                                   → kept for reads
#   state_add_file                                      → used by scripts for fs resources

# state_init
# Initialize or load state. Calls state_load (which auto-migrates v1 if needed).
state_init() {
    state_load
}

# state_has_feature <feature>
# Return 0 if the feature exists in state, 1 otherwise.
state_has_feature() {
    local feature="$1"
    if [[ -z "$feature" ]]; then
        log_error "state_has_feature: feature name is required"
        return 1
    fi
        feature=$(_state_normalize_feature_id "$feature") || return 1
    _state_ensure_loaded || return 1
    echo "$_STATE_JSON" | jq -e --arg f "$feature" '.features[$f] != null' >/dev/null 2>&1
}

# state_list_features
# Output all installed feature names (one per line).
state_list_features() {
    _state_ensure_loaded || return 1
    echo "$_STATE_JSON" | jq -r '.features | keys[]' 2>/dev/null
}

# state_get_files <feature>
# Output file paths tracked for a feature (one per line).
state_get_files() {
    local feature="$1"
    if [[ -z "$feature" ]]; then
        log_error "state_get_files: feature name is required"
        return 1
    fi
        feature=$(_state_normalize_feature_id "$feature") || return 1
    _state_ensure_loaded || return 1
    echo "$_STATE_JSON" | jq -r \
        --arg f "$feature" \
        '.features[$f].resources // [] | .[] | select(.kind == "fs") | .fs.path' \
        2>/dev/null
}

# state_feature_key_for — REMOVED in Phase 3 (state v3 uses canonical IDs directly).

# state_has_file <path>
# Return 0 if the path is tracked under any feature, 1 otherwise.
state_has_file() {
    local path="$1"
    if [[ -z "$path" ]]; then
        log_error "state_has_file: path is required"
        return 1
    fi
    _state_ensure_loaded || return 1
    echo "$_STATE_JSON" | jq -e \
        --arg p "$path" \
        '[.features | to_entries[] | .value.resources[] |
          select(.kind == "fs") | .fs.path] | index($p) != null' \
        >/dev/null 2>&1
}

# state_add_file <feature> <path>
# Register an fs resource for a feature and commit atomically.
# entry_type and op are inferred from the filesystem at call time.
# Idempotent: replaces any existing resource with the same id.
state_add_file() {
    local feature="$1"
    local path="$2"

    if [[ -z "$feature" ]] || [[ -z "$path" ]]; then
        log_error "state_add_file: feature and path are required"
        return 1
    fi
        feature=$(_state_normalize_feature_id "$feature") || return 1
    _state_ensure_loaded || return 1

    # Infer entry_type and op from the actual filesystem entry
    local entry_type op
    if [[ -L "$path" ]]; then
        entry_type="symlink"
        op="link"
    elif [[ -d "$path" ]]; then
        entry_type="dir"
        op="copy"
    elif [[ -f "$path" ]]; then
        entry_type="file"
        op="copy"
    else
        entry_type="file"
        op="copy"
    fi

    local resource
    resource=$(jq -n \
        --arg path "$path" \
        --arg et "$entry_type" \
        --arg op "$op" '
        {
            kind: "fs",
            id: ("fs:" + $path),
            fs: {
                path: $path,
                entry_type: $et,
                op: $op
            }
        }
    ')

    local new_json
    new_json=$(echo "$_STATE_JSON" | jq \
        --arg f "$feature" \
        --argjson res "$resource" '
        if .features[$f] == null then
            .features[$f] = {"resources": []}
        else . end |
        .features[$f].resources = (
            [.features[$f].resources[] | select(.id != $res.id)] + [$res]
        )
    ')
    state_commit_atomic "$new_json"
}

# state_get_runtime <feature> <key>
# Return the value for <key> from the runtime resource of a feature.
# Only key="version" is supported; returns the runtime.version string.
state_get_runtime() {
    local feature="$1"
    local key="$2"

    if [[ -z "$feature" ]] || [[ -z "$key" ]]; then
        log_error "state_get_runtime: feature and key are required"
        return 1
    fi
        feature=$(_state_normalize_feature_id "$feature") || return 1
    _state_ensure_loaded || return 1

    if [[ "$key" != "version" ]]; then
        return 0
    fi

    echo "$_STATE_JSON" | jq -r \
        --arg f "$feature" \
        '.features[$f].resources // [] |
         .[] | select(.kind == "runtime") | .runtime.version' \
        2>/dev/null | head -n1
}
