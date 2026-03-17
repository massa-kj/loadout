#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Unit tests: planner (Planner)
#
# Tests that planner_run produces the correct plan JSON for every
# classification case:
#   create, destroy, noop (script), noop (identical), replace, replace_backend,
#   strengthen, blocked (desired unknown kind), blocked (state unknown kind),
#   runtime version mismatch → replace, runtime version match → noop
#
# DRG fixtures use the RRG format (desired_backend present in resources).
# Version comparison uses a profile YAML file passed to planner_run.
#
# Run directly: bash tests/unit/test_planner.sh
# Exit code 0 = all pass, 1 = one or more failures.
# 
# 
# Cover all classification cases in the decision table:
# 
# * `create` — feature not in state
# * `destroy` — feature in state but not in profile
# * `noop` — feature in state and spec-compatible (script mode; identical resources)
# * `replace` — resource field mismatch (name, path, entry_type, op)
# * `replace_backend` — `desired_backend` mismatch only
# * `strengthen` — current state is a strict subset of spec resources, all compatible
# * `blocked` — unsupported resource kind, missing dependency, missing capability provider
# * Runtime version cases — profile version vs. state version
# -----------------------------------------------------------------------------

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

source "$(dirname "${BASH_SOURCE[0]}")/helpers.sh"

# ── Setup: tmp directory for profile YAML files ───────────────────────────────

TMPDIR_PLANNER="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_PLANNER"' EXIT

# ── Stubs ─────────────────────────────────────────────────────────────────────────────────────

# Stub log_* already provided via helpers.sh.
# We set _STATE_JSON directly so state_load is never called.
state_load() { return 0; }

# state_has_feature: check _STATE_JSON for the feature.
state_has_feature() {
    local feature="$1"
    local count
    count=$(printf '%s' "$_STATE_JSON" | jq --arg f "$feature" '.features | has($f)' 2>/dev/null || echo "false")
    [[ "$count" == "true" ]]
}

source "$REPO_ROOT/core/lib/planner.sh"

# ── Test helpers ──────────────────────────────────────────────────────────────

# _make_drg <features_json_object>
# Wrap features map into a DRG document.
_make_drg() {
    local features="$1"
    printf '{"schema_version":1,"features":%s}' "$features"
}

# _make_empty_state
# Empty state (no features installed).
_make_empty_state() {
    printf '{"version":3,"features":{}}'
}

# _make_state <features_json_object>
# Wrap features map into a state v3 document.
_make_state() {
    local features="$1"
    printf '{"version":3,"features":%s}' "$features"
}

# _assert_op <test_name> <feature> <expected_op> <plan_json>
# Assert that the plan contains an action with the given operation for the feature.
_assert_op() {
    local name="$1" feature="$2" expected_op="$3" plan_json="$4"
    local actual_op
    actual_op=$(printf '%s' "$plan_json" | jq -r \
        --arg f "$feature" \
        '.actions[] | select(.feature == $f) | .operation' 2>/dev/null || true)
    _assert_eq "$name" "$expected_op" "$actual_op"
}

# _assert_noop <test_name> <feature> <plan_json>
# Assert that the feature appears in plan.noops (not in actions).
_assert_noop() {
    local name="$1" feature="$2" plan_json="$3"
    local in_noops
    in_noops=$(printf '%s' "$plan_json" | jq -r \
        --arg f "$feature" \
        '[.noops[] | select(.feature == $f)] | length' 2>/dev/null || echo "0")
    _assert_eq "$name" "1" "$in_noops"
}

# _assert_blocked <test_name> <feature> <plan_json>
_assert_blocked() {
    local name="$1" feature="$2" plan_json="$3"
    local in_blocked
    in_blocked=$(printf '%s' "$plan_json" | jq -r \
        --arg f "$feature" \
        '[.blocked[] | select(.feature == $f)] | length' 2>/dev/null || echo "0")
    _assert_eq "$name" "1" "$in_blocked"
}

# _assert_summary_field <test_name> <field> <expected_count> <plan_json>
_assert_summary_field() {
    local name="$1" field="$2" expected="$3" plan_json="$4"
    local actual
    actual=$(printf '%s' "$plan_json" | jq -r ".summary.$field // 0")
    _assert_eq "$name" "$expected" "$actual"
}

# ── Test cases ────────────────────────────────────────────────────────────────

# ---------------------------------------------------------------------------
# 1. create: feature in DRG, not in state
# ---------------------------------------------------------------------------
echo
echo "── create ─────────────────────────────────────────────────────"

_STATE_JSON=$(_make_empty_state)
drg=$(_make_drg '{"core/git":{"resources":[{"kind":"package","name":"git","id":"pkg:git","desired_backend":"brew"}]}}')
declare -a _sorted=("core/git")
plan=$(planner_run "$drg" _sorted)

_assert_op     "create: action=create"               "core/git" "create" "$plan"
_assert_summary_field "create: summary.create=1"     "create"   "1"      "$plan"
_assert_summary_field "create: summary.destroy=0"    "destroy"  "0"      "$plan"
unset _sorted

# ---------------------------------------------------------------------------
# 2. destroy: feature in state, not in DRG desired
# ---------------------------------------------------------------------------
echo
echo "── destroy ────────────────────────────────────────────────────"

_STATE_JSON=$(_make_state '{"core/old":{"resources":[{"kind":"package","id":"pkg:old","backend":"brew","package":{"name":"old","version":null}}]}}')
drg=$(_make_drg '{}')
declare -a _sorted=()
plan=$(planner_run "$drg" _sorted)

_assert_op     "destroy: action=destroy"              "core/old" "destroy" "$plan"
_assert_summary_field "destroy: summary.destroy=1"   "destroy"  "1"       "$plan"
unset _sorted

# ---------------------------------------------------------------------------
# 3. noop (script): feature in both, desired_resource_count=0
# ---------------------------------------------------------------------------
echo
echo "── noop (script feature) ──────────────────────────────────────"

_STATE_JSON=$(_make_state '{"core/bash":{"resources":[]}}')
drg=$(_make_drg '{"core/bash":{"resources":[]}}')
declare -a _sorted=("core/bash")
plan=$(planner_run "$drg" _sorted)

_assert_noop   "noop(script): shows as noop"          "core/bash" "$plan"
_assert_summary_field "noop(script): summary.noop=1"  "noop"      "1"      "$plan"
unset _sorted

# ---------------------------------------------------------------------------
# 4. noop (declarative identical): same package resources in desired and state
# ---------------------------------------------------------------------------
echo
echo "── noop (declarative identical) ───────────────────────────────"

_STATE_JSON=$(_make_state '{"core/git":{"resources":[{"kind":"package","id":"pkg:git","backend":"brew","package":{"name":"git","version":null}}]}}')
drg=$(_make_drg '{"core/git":{"resources":[{"kind":"package","name":"git","id":"pkg:git","desired_backend":"brew"}]}}')
declare -a _sorted=("core/git")
plan=$(planner_run "$drg" _sorted)

_assert_noop   "noop(decl): shows as noop"             "core/git" "$plan"
_assert_summary_field "noop(decl): summary.noop=1"     "noop"     "1"     "$plan"
unset _sorted

# ---------------------------------------------------------------------------
# 5. replace: incompatible fs resource (target path change)
# ---------------------------------------------------------------------------
echo
echo "── replace (incompatible fs) ──────────────────────────────────"

_STATE_JSON=$(_make_state '{"user/git":{"resources":[{"kind":"fs","id":"fs:gitconfig","backend":"fs","fs":{"path":"/home/user/.gitconfig","entry_type":"file","op":"link"}}]}}')
drg=$(_make_drg '{"user/git":{"resources":[{"kind":"fs","name":"gitconfig","id":"fs:gitconfig","target":"/home/user/.config/git/config","entry_type":"file","op":"link","desired_backend":"fs"}]}}')
declare -a _sorted=("user/git")
plan=$(planner_run "$drg" _sorted)

_assert_op     "replace(fs path): action=replace"     "user/git" "replace" "$plan"
_assert_summary_field "replace(fs path): summary=1"   "replace"  "1"       "$plan"
unset _sorted

# ---------------------------------------------------------------------------
# 6. replace_backend: same package key, different backend
# ---------------------------------------------------------------------------
echo
echo "── replace_backend ────────────────────────────────────────────"

_STATE_JSON=$(_make_state '{"core/node":{"resources":[{"kind":"package","id":"pkg:nodejs","backend":"brew","package":{"name":"nodejs","version":null}}]}}')
drg=$(_make_drg '{"core/node":{"resources":[{"kind":"package","name":"nodejs","id":"pkg:nodejs","desired_backend":"apt"}]}}')
declare -a _sorted=("core/node")
plan=$(planner_run "$drg" _sorted)

_assert_op     "replace_backend: action=replace_backend"    "core/node" "replace_backend" "$plan"
_assert_summary_field "replace_backend: summary=1"          "replace_backend" "1"          "$plan"
unset _sorted

# ---------------------------------------------------------------------------
# 7. strengthen: state ⊂ desired, all common compatible, desired has extras
# ---------------------------------------------------------------------------
echo
echo "── strengthen ─────────────────────────────────────────────────"

_STATE_JSON=$(_make_state '{"core/dev":{"resources":[{"kind":"package","id":"pkg:git","backend":"brew","package":{"name":"git","version":null}}]}}')
drg=$(_make_drg '{"core/dev":{"resources":[
    {"kind":"package","name":"git",  "id":"pkg:git",  "desired_backend":"brew"},
    {"kind":"package","name":"curl", "id":"pkg:curl", "desired_backend":"brew"}
]}}')
declare -a _sorted=("core/dev")
plan=$(planner_run "$drg" _sorted)

_assert_op     "strengthen: action=strengthen"             "core/dev" "strengthen" "$plan"
_assert_summary_field "strengthen: summary=1"              "strengthen" "1"         "$plan"
# Verify add_resources contains curl
add_count=$(printf '%s' "$plan" | jq '[.actions[] | select(.feature=="core/dev") | .details.add_resources[]] | length')
_assert_eq     "strengthen: add_resources has 1 entry"    "1" "$add_count"
unset _sorted

# ---------------------------------------------------------------------------
# 8. strengthen → replace when state has resource NOT in desired (s_only > 0)
# ---------------------------------------------------------------------------
echo
echo "── strengthen boundary (s_only → replace) ─────────────────────"

# State has both git and curl; desired only has git (curl removed) → replace, NOT strengthen
_STATE_JSON=$(_make_state '{"core/dev":{"resources":[
    {"kind":"package","id":"pkg:git", "backend":"brew","package":{"name":"git", "version":null}},
    {"kind":"package","id":"pkg:curl","backend":"brew","package":{"name":"curl","version":null}}
]}}')
drg=$(_make_drg '{"core/dev":{"resources":[
    {"kind":"package","name":"git","id":"pkg:git","desired_backend":"brew"}
]}}')
declare -a _sorted=("core/dev")
plan=$(planner_run "$drg" _sorted)

_assert_op     "strengthen-boundary: s_only→replace" "core/dev" "replace" "$plan"
unset _sorted

# ---------------------------------------------------------------------------
# 9. blocked: unknown resource kind in desired
# ---------------------------------------------------------------------------
echo
echo "── blocked (unknown kind in desired) ──────────────────────────"

_STATE_JSON=$(_make_empty_state)
drg=$(_make_drg '{"user/legacy":{"resources":[{"kind":"registry","name":"foo","id":"reg:foo","desired_backend":"winreg"}]}}')
declare -a _sorted=("user/legacy")
plan=$(planner_run "$drg" _sorted)

_assert_blocked        "blocked(desired unknown): in blocked list"   "user/legacy" "$plan"
_assert_summary_field  "blocked(desired unknown): summary.blocked=1" "blocked" "1" "$plan"
unset _sorted

# ---------------------------------------------------------------------------
# 10. blocked: unknown resource kind in state
# ---------------------------------------------------------------------------
echo
echo "── blocked (unknown kind in state) ────────────────────────────"

_STATE_JSON=$(_make_state '{"user/legacy":{"resources":[{"kind":"registry","id":"reg:foo","backend":"winreg","registry":{"name":"foo"}}]}}')
drg=$(_make_drg '{"user/legacy":{"resources":[{"kind":"package","name":"git","id":"pkg:git","desired_backend":"brew"}]}}')
declare -a _sorted=("user/legacy")
plan=$(planner_run "$drg" _sorted)

_assert_blocked        "blocked(state unknown): in blocked list"   "user/legacy" "$plan"
_assert_summary_field  "blocked(state unknown): summary.blocked=1" "blocked" "1" "$plan"
unset _sorted

# ---------------------------------------------------------------------------
# 11. runtime: version mismatch → replace  (version read from profile file)
# ---------------------------------------------------------------------------
echo
echo "── runtime version mismatch → replace ─────────────────────────"

_PROFILE_V11="$TMPDIR_PLANNER/profile_v11.yaml"
cat > "$_PROFILE_V11" <<'EOF'
features:
  tools/node:
    version: "20.0.0"
EOF

_STATE_JSON=$(_make_state '{"tools/node":{"resources":[{"kind":"runtime","id":"rt:node","backend":"mise","runtime":{"name":"node","version":"18.0.0"}}]}}')
drg=$(_make_drg '{"tools/node":{"resources":[{"kind":"runtime","name":"node","id":"rt:node","desired_backend":"mise"}]}}')
declare -a _sorted=("tools/node")
plan=$(planner_run "$drg" _sorted "$_PROFILE_V11")

_assert_op     "runtime version mismatch: action=replace" "tools/node" "replace" "$plan"
unset _sorted

# ---------------------------------------------------------------------------
# 12. runtime: version match → noop  (version read from profile file)
# ---------------------------------------------------------------------------
echo
echo "── runtime version match → noop ───────────────────────────────"

_PROFILE_V12="$TMPDIR_PLANNER/profile_v12.yaml"
cat > "$_PROFILE_V12" <<'EOF'
features:
  tools/node:
    version: "20.0.0"
EOF

_STATE_JSON=$(_make_state '{"tools/node":{"resources":[{"kind":"runtime","id":"rt:node","backend":"mise","runtime":{"name":"node","version":"20.0.0"}}]}}')
drg=$(_make_drg '{"tools/node":{"resources":[{"kind":"runtime","name":"node","id":"rt:node","desired_backend":"mise"}]}}')
declare -a _sorted=("tools/node")
plan=$(planner_run "$drg" _sorted "$_PROFILE_V12")

_assert_noop   "runtime version match: noop"              "tools/node" "$plan"
unset _sorted

# ---------------------------------------------------------------------------
# 13. runtime: no version in profile → noop (no version constraint)
# ---------------------------------------------------------------------------
echo
echo "── runtime no version in profile → noop ───────────────────────"

_STATE_JSON=$(_make_state '{"tools/python":{"resources":[{"kind":"runtime","id":"rt:python","backend":"mise","runtime":{"name":"python","version":"3.11.0"}}]}}')
drg=$(_make_drg '{"tools/python":{"resources":[{"kind":"runtime","name":"python","id":"rt:python","desired_backend":"mise"}]}}')
declare -a _sorted=("tools/python")
plan=$(planner_run "$drg" _sorted)

_assert_noop   "runtime no version constraint: noop"      "tools/python" "$plan"
unset _sorted

# ---------------------------------------------------------------------------
# 14. mixed: create + destroy + noop in a single plan
# ---------------------------------------------------------------------------
echo
echo "── mixed (create + destroy + noop) ────────────────────────────"

_STATE_JSON=$(_make_state '{
    "core/old": {"resources":[{"kind":"package","id":"pkg:old","backend":"brew","package":{"name":"old","version":null}}]},
    "core/bash": {"resources":[]}
}')
drg=$(_make_drg '{
    "core/new":  {"resources":[{"kind":"package","name":"new","id":"pkg:new","desired_backend":"brew"}]},
    "core/bash": {"resources":[]}
}')
declare -a _sorted=("core/new" "core/bash")
plan=$(planner_run "$drg" _sorted)

_assert_op     "mixed: core/new=create"      "core/new"  "create"  "$plan"
_assert_op     "mixed: core/old=destroy"     "core/old"  "destroy" "$plan"
_assert_noop   "mixed: core/bash=noop"       "core/bash"           "$plan"
_assert_summary_field "mixed: summary.create=1"  "create"  "1" "$plan"
_assert_summary_field "mixed: summary.destroy=1" "destroy" "1" "$plan"
_assert_summary_field "mixed: summary.noop=1"    "noop"    "1" "$plan"
unset _sorted

# ── Summary ───────────────────────────────────────────────────────────────────

echo
_print_summary
