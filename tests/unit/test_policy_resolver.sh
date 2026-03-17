#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Unit tests: policy_resolver (PolicyResolver)
#
# Tests that policy_resolver_run correctly adds desired_backend to
# package/runtime resources, while leaving fs and unknown-kind resources
# unchanged.
#
# Run directly: bash tests/unit/test_policy_resolver.sh
# Exit code 0 = all pass, 1 = one or more failures.
# -----------------------------------------------------------------------------

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

source "$(dirname "${BASH_SOURCE[0]}")/helpers.sh"

# ── Stubs ─────────────────────────────────────────────────────────────────────

# Stub resolve_backend_for: routes by kind+name to a predictable backend string.
resolve_backend_for() {
    local kind="$1"
    local name="$2"
    case "${kind}:${name}" in
        package:ripgrep)  echo "brew"   ;;
        package:fzf)      echo "apt"    ;;
        runtime:node)     echo "mise"   ;;
        runtime:python)   echo "mise"   ;;
        *)                echo "unknown" ;;
    esac
    return 0
}

source "$REPO_ROOT/core/lib/policy_resolver.sh"

# ── Test helpers ──────────────────────────────────────────────────────────────

# _make_drg <features_json_object>
# Wrap features map into a raw DRG document.
_make_drg() {
    local features="$1"
    printf '{"schema_version":1,"features":%s}' "$features"
}

# ── Test cases ────────────────────────────────────────────────────────────────

# ---------------------------------------------------------------------------
# 1. package resource → desired_backend added
# ---------------------------------------------------------------------------
echo
echo "── package: desired_backend added ──────────────────────────────"

drg=$(_make_drg '{"core/rg":{"resources":[{"kind":"package","name":"ripgrep","id":"package:ripgrep"}]}}')
rrg=$(policy_resolver_run "$drg")

backend=$(printf '%s' "$rrg" | jq -r '.features["core/rg"].resources[0].desired_backend // "absent"')
_assert_eq "package ripgrep → desired_backend=brew" "brew" "$backend"

# Original fields preserved
id=$(printf '%s' "$rrg" | jq -r '.features["core/rg"].resources[0].id // "absent"')
_assert_eq "package ripgrep id preserved" "package:ripgrep" "$id"

# schema_version preserved
sv=$(printf '%s' "$rrg" | jq -r '.schema_version // "absent"')
_assert_eq "schema_version preserved" "1" "$sv"

# ---------------------------------------------------------------------------
# 2. runtime resource → desired_backend added
# ---------------------------------------------------------------------------
echo
echo "── runtime: desired_backend added ──────────────────────────────"

drg=$(_make_drg '{"tools/node":{"resources":[{"kind":"runtime","name":"node","id":"runtime:node"}]}}')
rrg=$(policy_resolver_run "$drg")

backend=$(printf '%s' "$rrg" | jq -r '.features["tools/node"].resources[0].desired_backend // "absent"')
_assert_eq "runtime node → desired_backend=mise" "mise" "$backend"

# ---------------------------------------------------------------------------
# 3. fs resource → passes through without desired_backend
# ---------------------------------------------------------------------------
echo
echo "── fs: passes through without desired_backend ──────────────────"

drg=$(_make_drg '{"user/git":{"resources":[{"kind":"fs","target":"~/.gitconfig","id":"fs:gitconfig"}]}}')
rrg=$(policy_resolver_run "$drg")

backend=$(printf '%s' "$rrg" | jq -r '.features["user/git"].resources[0].desired_backend // "absent"')
_assert_eq "fs resource: no desired_backend" "absent" "$backend"

# Original target field preserved
target=$(printf '%s' "$rrg" | jq -r '.features["user/git"].resources[0].target // "absent"')
_assert_eq "fs resource target preserved" "~/.gitconfig" "$target"

# ---------------------------------------------------------------------------
# 4. multiple features, mixed kinds
# ---------------------------------------------------------------------------
echo
echo "── multiple features mixed kinds ───────────────────────────────"

drg=$(_make_drg '{
    "core/rg":    {"resources":[{"kind":"package","name":"ripgrep","id":"package:ripgrep"}]},
    "tools/node": {"resources":[{"kind":"runtime","name":"node","id":"runtime:node"}]},
    "user/git":   {"resources":[{"kind":"fs","target":"~/.gitconfig","id":"fs:gitconfig"}]}
}')
rrg=$(policy_resolver_run "$drg")

key_count=$(printf '%s' "$rrg" | jq '.features | keys | length')
_assert_eq "all 3 features preserved" "3" "$key_count"

rg_backend=$(printf '%s' "$rrg" | jq -r '.features["core/rg"].resources[0].desired_backend // "absent"')
_assert_eq "mixed: ripgrep backend" "brew" "$rg_backend"

node_backend=$(printf '%s' "$rrg" | jq -r '.features["tools/node"].resources[0].desired_backend // "absent"')
_assert_eq "mixed: node backend" "mise" "$node_backend"

git_backend=$(printf '%s' "$rrg" | jq -r '.features["user/git"].resources[0].desired_backend // "absent"')
_assert_eq "mixed: fs has no backend" "absent" "$git_backend"

# ---------------------------------------------------------------------------
# 5. unknown resource kind → passes through (Planner will block)
# ---------------------------------------------------------------------------
echo
echo "── unknown kind: passes through ────────────────────────────────"

drg=$(_make_drg '{"user/legacy":{"resources":[{"kind":"registry","name":"foo","id":"registry:foo"}]}}')
rrg=$(policy_resolver_run "$drg")

backend=$(printf '%s' "$rrg" | jq -r '.features["user/legacy"].resources[0].desired_backend // "absent"')
_assert_eq "unknown kind: no desired_backend added" "absent" "$backend"

kind=$(printf '%s' "$rrg" | jq -r '.features["user/legacy"].resources[0].kind')
_assert_eq "unknown kind: kind preserved" "registry" "$kind"

# ---------------------------------------------------------------------------
# 6. empty DRG (no features)
# ---------------------------------------------------------------------------
echo
echo "── empty DRG ───────────────────────────────────────────────────"

drg=$(_make_drg '{}')
rrg=$(policy_resolver_run "$drg")

key_count=$(printf '%s' "$rrg" | jq '.features | keys | length')
_assert_eq "empty DRG: 0 features" "0" "$key_count"

# ---------------------------------------------------------------------------
# 7. feature with empty resources (script feature)
# ---------------------------------------------------------------------------
echo
echo "── script feature (empty resources) ────────────────────────────"

drg=$(_make_drg '{"core/bash":{"resources":[]}}')
rrg=$(policy_resolver_run "$drg")

res_count=$(printf '%s' "$rrg" | jq '.features["core/bash"].resources | length')
_assert_eq "script feature: 0 resources" "0" "$res_count"

# ── Summary ───────────────────────────────────────────────────────────────────

echo
_print_summary
