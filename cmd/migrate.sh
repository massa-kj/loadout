#!/usr/bin/env bash
# cmd/migrate.sh — CLI entry point for the migrate command.
#
# Migrate the loadout state file to the latest schema version (v3).
#
# Supported migration paths:
#   v1 → v2 → v3   (via existing _migrate_v1_to_v2 + state_migrate_v2_to_v3)
#   v2 → v3
#   v3              (already current; nothing to do)
#
# This command reads the state file DIRECTLY (bypassing state_load, which
# rejects v1/v2) so that migration can proceed even on outdated state.
#
# Flags:
#   --dry-run     Show what would change without committing.
#   --profiles    Also normalize feature keys in profiles/*.yaml (bare → core/<name>).
#
# Usage:
#   loadout migrate
#   loadout migrate --dry-run
#   loadout migrate --profiles

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOADOUT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
export LOADOUT_ROOT

# ── Library loading ───────────────────────────────────────────────────────────

source "$LOADOUT_ROOT/core/lib/env.sh"
source "$LOADOUT_ROOT/core/lib/logger.sh"
source "$LOADOUT_ROOT/core/lib/state.sh"

# ── Platform guard ────────────────────────────────────────────────────────────

case "$LOADOUT_PLATFORM" in
    linux|wsl) ;;
    windows)
        log_error "On Windows, use loadout.ps1 instead."
        exit 1
        ;;
    *)
        log_error "Unknown platform: $LOADOUT_PLATFORM"
        exit 1
        ;;
esac

# ── Argument parsing ──────────────────────────────────────────────────────────

DRY_RUN=false
MIGRATE_PROFILES=false

for arg in "$@"; do
    case "$arg" in
        --dry-run)    DRY_RUN=true ;;
        --profiles)   MIGRATE_PROFILES=true ;;
        -h|--help)
            cat <<EOF
Usage: loadout migrate [--dry-run] [--profiles]

Migrate the loadout state file to schema v3.

Options:
  --dry-run     Show the migration diff without writing any changes.
  --profiles    Also normalize bare feature names in profiles/*.yaml to
                canonical IDs (e.g. "git" → "core/git").
EOF
            exit 0
            ;;
        *)
            log_error "Unknown argument: $arg"
            exit 1
            ;;
    esac
done

# ── Helpers ───────────────────────────────────────────────────────────────────

# _migrate_show_diff <before_json> <after_json>
# Print a human-readable diff of feature keys before and after migration.
_migrate_show_diff() {
    local before="$1"
    local after="$2"

    local before_keys after_keys
    before_keys=$(echo "$before" | jq -r '.features | keys[]' | sort)
    after_keys=$(echo "$after"  | jq -r '.features | keys[]' | sort)

    if [[ "$before_keys" == "$after_keys" ]]; then
        log_info "  No feature key changes."
        return
    fi

    log_info "  Feature key changes:"
    while IFS= read -r key; do
        [[ -n "$key" ]] && log_info "    - $key"
    done < <(comm -23 <(echo "$before_keys") <(echo "$after_keys"))
    while IFS= read -r key; do
        [[ -n "$key" ]] && log_info "    + $key"
    done < <(comm -13 <(echo "$before_keys") <(echo "$after_keys"))
}

# _migrate_profiles
# Rewrite bare feature names in profiles/*.yaml to canonical IDs.
# Requires yq (https://github.com/mikefarah/yq) to be installed.
_migrate_profiles() {
    local profiles_dir="${LOADOUT_PROFILES_DIR:-}"
    if [[ -z "$profiles_dir" ]]; then
        log_error "migrate --profiles: LOADOUT_PROFILES_DIR is not set"
        return 1
    fi
    if [[ ! -d "$profiles_dir" ]]; then
        log_warn "migrate: profiles directory not found: $profiles_dir (skipping)"
        return 0
    fi

    if ! command -v yq >/dev/null 2>&1; then
        log_error "migrate --profiles: 'yq' is required but not installed."
        log_error "  Install via: mise install yq  or  brew install yq"
        return 1
    fi

    local profile_file
    for profile_file in "$profiles_dir"/*.yaml; do
        [[ -f "$profile_file" ]] || continue
        log_info "  Processing profile: $profile_file"

        if "$DRY_RUN"; then
            # Show what would change without writing
            local orig_features normalized_features
            orig_features=$(yq -r '.features[].name // .features[]' "$profile_file" 2>/dev/null || true)
            log_info "    (dry-run) would normalize bare names to canonical IDs in: $(basename "$profile_file")"
        else
            # Rewrite: for each entry in .features[], if name doesn't contain "/",
            # prefix with "core/". The profile schema uses mixed forms depending
            # on whether it's a string list or object list.
            yq eval '
                .features = (.features | map(
                    if type == "string" then
                        if contains("/") then . else "core/" + . end
                    elif type == "object" then
                        if (.name | contains("/")) then . else .name = "core/" + .name end
                    else .
                    end
                ))
            ' -i "$profile_file"
            log_success "  Updated: $(basename "$profile_file")"
        fi
    done
}

# ── Main ──────────────────────────────────────────────────────────────────────

log_task "Migrating loadout state"

# 1. Resolve state path directly (do NOT call state_load / state_init)
if ! declare -F loadout_state_file_path >/dev/null 2>&1; then
    log_error "migrate: loadout_state_file_path is not available"
    exit 1
fi

STATE_PATH="$(loadout_state_file_path)"
LEGACY_STATE_PATH="$LOADOUT_ROOT/state/state.json"

state_dir="$(dirname "$STATE_PATH")"
mkdir -p "$state_dir"

# Legacy state physical move (copy + backup, keep original file).
if [[ ! -f "$STATE_PATH" && -f "$LEGACY_STATE_PATH" ]]; then
    timestamp="$(date +%Y%m%d_%H%M%S)"
    legacy_backup="${LEGACY_STATE_PATH}.bak.${timestamp}"

    cp "$LEGACY_STATE_PATH" "$legacy_backup"
    cp "$LEGACY_STATE_PATH" "$STATE_PATH"

    log_info "Legacy state copied: $LEGACY_STATE_PATH -> $STATE_PATH"
    log_info "Legacy backup created: $legacy_backup"
fi

if [[ ! -f "$STATE_PATH" ]]; then
    log_info "No state file found at $STATE_PATH — nothing to migrate."
    log_info "A fresh v3 state will be created on the next 'loadout apply'."
    exit 0
fi

if ! jq empty "$STATE_PATH" 2>/dev/null; then
    log_error "migrate: state file is not valid JSON: $STATE_PATH"
    exit 1
fi

_STATE_JSON="$(cat "$STATE_PATH")"

# 2. Check current version
CURRENT_VER=$(echo "$_STATE_JSON" | jq -r '.version // "unknown"')

log_info "Current state version: $CURRENT_VER"
log_info "Target  state version: 3"

if [[ "$CURRENT_VER" == "3" ]]; then
    log_success "State is already at v3 — nothing to migrate."
    if "$MIGRATE_PROFILES"; then
        log_task "Normalizing profile feature names"
        _migrate_profiles
    fi
    exit 0
fi

# 3. Chain v1 → v2 → v3 if needed
if [[ "$CURRENT_VER" == "1" ]]; then
    log_info "Migrating: v1 → v2..."

    v2_json=""
    if ! v2_json=$(_migrate_v1_to_v2 "$_STATE_JSON"); then
        log_error "migrate: v1 → v2 transformation failed"
        exit 1
    fi
    _STATE_JSON="$v2_json"
    CURRENT_VER="2"
    log_success "  v1 → v2 transformation complete."
fi

if [[ "$CURRENT_VER" != "2" ]]; then
    log_error "migrate: unexpected state version: $CURRENT_VER (expected 1 or 2)"
    exit 1
fi

# 4. Transform v2 → v3 (compute new JSON)
log_info "Migrating: v2 → v3..."

V3_JSON=$(_state_transform_v2_to_v3 "$_STATE_JSON")
if [[ -z "$V3_JSON" ]]; then
    log_error "migrate: v2 → v3 transformation returned empty output"
    exit 1
fi

# 5. Show diff
_migrate_show_diff "$_STATE_JSON" "$V3_JSON"

# 6. Handle --dry-run
if "$DRY_RUN"; then
    log_info ""
    log_info "[dry-run] No changes written."
    if "$MIGRATE_PROFILES"; then
        log_task "Normalizing profile feature names (dry-run)"
        _migrate_profiles
    fi
    exit 0
fi

# 7. Commit atomically via state_migrate_v2_to_v3
# The function uses _STATE_JSON (in-memory) for the source so we export our
# transformed-on-disk original. state_migrate_v2_to_v3 will re-derive v3 internally,
# so we just need _STATE_JSON to hold the v2 content to migrate from.
log_info "Writing migrated state..."
if ! state_migrate_v2_to_v3; then
    log_error "migrate: commit failed"
    exit 1
fi

log_success "State successfully migrated to v3."

# 8. Optional profile normalization
if "$MIGRATE_PROFILES"; then
    log_task "Normalizing profile feature names"
    _migrate_profiles
fi
