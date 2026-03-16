#!/usr/bin/env bash
# cmd/plan.sh — CLI entry point for the plan command.
#
# Runs the full planning pipeline (profile → diff → classify → decide)
# without executing any changes. State is never modified.
#
# Usage:
#   loadout plan <profile.yaml> [--verbose]
#
# Output:
#   Actions that would be taken, colored by operation type.
#   noop entries are hidden unless --verbose is specified.
#
# Exit codes:
#   0  — plan printed (may be all-noop)
#   1  — error (profile not found, resolver failure, etc.)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOADOUT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
export LOADOUT_ROOT

# ── Library loading ───────────────────────────────────────────────────────────

source "$LOADOUT_ROOT/core/lib/env.sh"
source "$LOADOUT_ROOT/core/lib/logger.sh"
source "$LOADOUT_ROOT/core/lib/state.sh"
source "$LOADOUT_ROOT/core/lib/runner.sh"
source "$LOADOUT_ROOT/core/lib/source_registry.sh"
source "$LOADOUT_ROOT/core/lib/feature_index.sh"
source "$LOADOUT_ROOT/core/lib/compiler.sh"
source "$LOADOUT_ROOT/core/lib/resolver.sh"
source "$LOADOUT_ROOT/core/lib/orchestrator.sh"

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

# ── Usage ─────────────────────────────────────────────────────────────────────

_usage() {
    cat <<EOF
Usage: loadout plan <profile.yaml> [--verbose]

Show what 'apply' would do without making any changes.

Arguments:
  profile.yaml    Path to the profile file

Options:
  --verbose       Also list noop (already up-to-date) features

Exit codes:
  0  Plan displayed successfully
  1  Error

Examples:
  loadout plan profiles/wsl.yaml
  loadout plan profiles/wsl.yaml --verbose
EOF
    exit 1
}

# ── Argument parsing ──────────────────────────────────────────────────────────

PROFILE_FILE=""
VERBOSE=false

for arg in "$@"; do
    case "$arg" in
        --verbose|-v) VERBOSE=true ;;
        --help|-h)    _usage ;;
        -*)           log_error "Unknown option: $arg"; _usage ;;
        *)            PROFILE_FILE="$arg" ;;
    esac
done

if [[ -z "$PROFILE_FILE" ]]; then
    _usage
fi

# ── Plan formatter ────────────────────────────────────────────────────────────

# _plan_print <plan_json> <profile_file> <verbose>
# Format and print plan JSON to stdout.
_plan_print() {
    local plan_json="$1"
    local profile="$2"
    local verbose="$3"

    # Colour constants (stdout — not using log_ functions which go to stderr)
    local C_RESET='\033[0m'
    local C_GREEN='\033[0;32m'
    local C_YELLOW='\033[0;33m'
    local C_RED='\033[0;31m'
    local C_CYAN='\033[0;36m'
    local C_GRAY='\033[0;90m'
    local C_BOLD='\033[1m'

    # Disable colour when not a terminal or TERM is unset/dumb
    if [[ ! -t 1 ]] || [[ "${TERM:-}" == "dumb" ]]; then
        C_RESET="" C_GREEN="" C_YELLOW="" C_RED="" C_CYAN="" C_GRAY="" C_BOLD=""
    fi

    local actions blocked summary
    actions=$(echo "$plan_json" | jq -c '.actions[]' 2>/dev/null || true)
    blocked=$(echo "$plan_json" | jq -c '.blocked[]' 2>/dev/null || true)
    summary=$(echo "$plan_json" | jq -r '
        .summary |
        "create=\(.create)  destroy=\(.destroy)  replace=\(.replace)  strengthen=\(.strengthen // 0)  noop=\(.noop)  blocked=\(.blocked)"
    ')

    echo ""
    printf "${C_BOLD}Plan:${C_RESET} %s\n" "$profile"
    echo ""

    local has_output=false

    # Print active operations: destroy → replace → create (plan ordering)
    while IFS= read -r action; do
        [[ -z "$action" ]] && continue
        local op feature
        op=$(echo "$action" | jq -r '.operation')
        feature=$(echo "$action" | jq -r '.feature')

        case "$op" in
            destroy)
                printf "  ${C_RED}%-9s${C_RESET} %s\n" "destroy" "$feature"
                has_output=true
                ;;
            replace)
                local from to
                from=$(echo "$action" | jq -r '.details.from_version // ""')
                to=$(echo "$action"   | jq -r '.details.to_version   // ""')
                if [[ -n "$from" && -n "$to" ]]; then
                    printf "  ${C_YELLOW}%-16s${C_RESET} %-20s %s \u2192 %s\n" \
                        "replace" "$feature" "$from" "$to"
                else
                    printf "  ${C_YELLOW}%-16s${C_RESET} %s\n" "replace" "$feature"
                fi
                has_output=true
                ;;
            replace_backend)
                printf "  ${C_YELLOW}%-16s${C_RESET} %s\n" "replace_backend" "$feature"
                has_output=true
                ;;
            strengthen)
                local add_count
                add_count=$(echo "$action" | jq '.details.add_resources | length' 2>/dev/null || echo "0")
                printf "  ${C_CYAN}%-16s${C_RESET} %-20s (+%s resource(s))\n" \
                    "strengthen" "$feature" "$add_count"
                has_output=true
                ;;
            create)
                printf "  ${C_GREEN}%-16s${C_RESET} %s\n" "create" "$feature"
                has_output=true
                ;;
            destroy)
                printf "  ${C_RED}%-16s${C_RESET} %s\n" "destroy" "$feature"
                has_output=true
                ;;
        esac
    done <<< "$actions"

    # Print blocked entries
    while IFS= read -r item; do
        [[ -z "$item" ]] && continue
        local feat reason
        feat=$(echo "$item"   | jq -r '.feature')
        reason=$(echo "$item" | jq -r '.reason // ""')
        printf "  ${C_RED}${C_BOLD}%-16s${C_RESET} %-20s ${C_GRAY}%s${C_RESET}\n" \
            "blocked" "$feat" "$reason"
        has_output=true
    done <<< "$blocked"

    # Print noop entries when --verbose
    if [[ "$verbose" == "true" ]]; then
        while IFS= read -r item; do
            [[ -z "$item" ]] && continue
            local feat
            feat=$(echo "$item" | jq -r '.feature')
            printf "  ${C_GRAY}%-16s${C_RESET} %s\n" "noop" "$feat"
            has_output=true
        done <<< "$(echo "$plan_json" | jq -c '.noops[]' 2>/dev/null || true)"
    fi

    if [[ "$has_output" == "true" ]]; then
        echo ""
    fi

    printf "${C_BOLD}Summary:${C_RESET} %s\n" "$summary"
    echo ""

    # Simple guidance message
    local create destroy replace strengthen blocked_count
    create=$(echo "$plan_json"         | jq '.summary.create')
    destroy=$(echo "$plan_json"         | jq '.summary.destroy')
    replace=$(echo "$plan_json"         | jq '.summary.replace')
    strengthen=$(echo "$plan_json"      | jq '.summary.strengthen // 0')
    blocked_count=$(echo "$plan_json"   | jq '.summary.blocked')

    if [[ "$blocked_count" -gt 0 ]]; then
        printf "${C_RED}%d blocked feature(s) — run 'apply' to see details.${C_RESET}\n\n" \
            "$blocked_count"
    elif [[ "$((create + destroy + replace + strengthen))" -eq 0 ]]; then
        printf "${C_GRAY}Nothing to do.${C_RESET}\n\n"
    else
        printf "${C_BOLD}Run 'loadout apply %s' to apply these changes.${C_RESET}\n\n" \
            "$profile"
    fi
}

# ── Plan pipeline ─────────────────────────────────────────────────────────────

log_task "Planning profile: $PROFILE_FILE"

# Load backend policy (non-fatal if policies dir is absent)
backend_registry_load_policy

# Initialise (or migrate) state — read-only after this point
state_init

# Parse profile
declare -a _plan_features
read_profile "$PROFILE_FILE" _plan_features || exit 1

# Build Feature Index: scans all registered sources, enriches with metadata
_plan_index=""
feature_index_build _plan_index || exit 1

# Filter desired features: separates valid from spec_version-blocked
declare -a _plan_valid_features
_plan_sv_blocked=""
feature_index_filter "$_plan_index" _plan_features _plan_valid_features _plan_sv_blocked || exit 1

# Resolve feature metadata from index (no file I/O) + topological sort
read_feature_metadata "$_plan_index" _plan_valid_features || exit 1

declare -a _plan_sorted
resolve_dependencies _plan_valid_features _plan_sorted || exit 1

# Compile raw DesiredResourceGraph (assigns stable resource IDs only)
_plan_drg=""
_plan_drg=$(feature_compiler_run "$_plan_index" _plan_sorted) || exit 1

# Resolve desired_backend per resource via PolicyResolver
_plan_rrg=""
_plan_rrg=$(policy_resolver_run "$_plan_drg") || exit 1

# Plan: pure computation — no state writes
plan_json=$(planner_run "$_plan_rrg" _plan_sorted "$PROFILE_FILE") || exit 1

# Inject spec_version-blocked features into plan output
plan_json=$(_plan_inject_blocked "$plan_json" "$_plan_sv_blocked")

# Display
_plan_print "$plan_json" "$PROFILE_FILE" "$VERBOSE"
