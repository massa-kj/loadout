#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Unit tests: resolver (Phase 2)
#
# Tests that resolve_dependencies outputs canonical IDs.
# Tests depend normalization (bare name -> same-source canonical ID).
# Run directly: bash tests/unit/test_resolver.sh
# Exit code 0 = all pass, 1 = one or more failures.
# -----------------------------------------------------------------------------

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

source "$(dirname "${BASH_SOURCE[0]}")/helpers.sh"

TMPDIR_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_ROOT"' EXIT

export LOADOUT_ROOT="$TMPDIR_ROOT/repo"
export LOADOUT_PLATFORM="linux"
export LOADOUT_CONFIG_HOME="$TMPDIR_ROOT/config/loadout"
export LOADOUT_DATA_HOME="$TMPDIR_ROOT/data/loadout"
export LOADOUT_SOURCES_FILE="$LOADOUT_CONFIG_HOME/sources.yaml"

mkdir -p "$LOADOUT_ROOT/features" "$LOADOUT_ROOT/backends"
mkdir -p "$LOADOUT_CONFIG_HOME/features" "$LOADOUT_CONFIG_HOME/backends"
mkdir -p "$LOADOUT_DATA_HOME/sources"

source "$REPO_ROOT/core/lib/source_registry.sh"
source "$REPO_ROOT/core/lib/feature_index.sh"
source "$REPO_ROOT/core/lib/resolver.sh"

# ── Setup: temp feature directory ────────────────────────────────────────────

TMPDIR_FEATURES="$LOADOUT_ROOT/features"
mkdir -p "$TMPDIR_FEATURES"

cat > "$LOADOUT_SOURCES_FILE" <<'EOF'
sources:
    - id: ext
        type: git
        url: https://example.invalid/ext.git
        commit: abcdef
        allow:
            features:
                - extfeat
            backends: "*"
EOF

mkdir -p "$LOADOUT_DATA_HOME/sources/ext/features/extfeat"
cat > "$LOADOUT_DATA_HOME/sources/ext/features/extfeat/feature.yaml" <<'EOF'
spec_version: 1
mode: script
description: external feature
depends: []
EOF

mkdir -p "$LOADOUT_CONFIG_HOME/features/myfeat"
cat > "$LOADOUT_CONFIG_HOME/features/myfeat/feature.yaml" <<'EOF'
spec_version: 1
mode: script
description: user feature
depends: []
EOF

# Create minimal feature.yaml files for test features.
# alpha  – no deps
# beta   – depends: [alpha] (bare name -> core/alpha)
# gamma  – depends: [beta]  (bare name -> core/beta); also depends on user/delta explicitly
# user/delta is a user-source feature (not in this source; only tests normalization)

mkdir -p "$TMPDIR_FEATURES/alpha"
cat > "$TMPDIR_FEATURES/alpha/feature.yaml" <<'EOF'
spec_version: 1
mode: script
description: alpha feature
depends: []
EOF

mkdir -p "$TMPDIR_FEATURES/beta"
cat > "$TMPDIR_FEATURES/beta/feature.yaml" <<'EOF'
spec_version: 1
mode: script
description: beta feature
depends:
  - alpha
EOF

mkdir -p "$TMPDIR_FEATURES/gamma"
cat > "$TMPDIR_FEATURES/gamma/feature.yaml" <<'EOF'
spec_version: 1
mode: script
description: gamma feature
depends:
  - beta
EOF

mkdir -p "$TMPDIR_FEATURES/delta"
cat > "$TMPDIR_FEATURES/delta/feature.yaml" <<'EOF'
spec_version: 1
mode: script
description: delta feature (simulates user-source)
depends: []
EOF

mkdir -p "$TMPDIR_FEATURES/provider"
cat > "$TMPDIR_FEATURES/provider/feature.yaml" <<'EOF'
spec_version: 1
mode: script
description: provides a capability
depends: []
provides:
  - name: my_capability
EOF

mkdir -p "$TMPDIR_FEATURES/consumer"
cat > "$TMPDIR_FEATURES/consumer/feature.yaml" <<'EOF'
spec_version: 1
mode: script
description: requires a capability
depends: []
requires:
  - name: my_capability
EOF

# ── Test: resolve_dependencies outputs canonical IDs ─────────────────────────

echo "resolve_dependencies: canonical ID output"

declare -a test_features=("core/alpha" "core/beta" "core/gamma")
_test_index_1=""
feature_index_build _test_index_1 2>/dev/null
read_feature_metadata "$_test_index_1" test_features 2>/dev/null

declare -a sorted_output
resolve_dependencies test_features sorted_output 2>/dev/null

_assert_contains \
    "output contains core/alpha" \
    "core/alpha" \
    "${sorted_output[*]}"

_assert_contains \
    "output contains core/beta" \
    "core/beta" \
    "${sorted_output[*]}"

_assert_contains \
    "output contains core/gamma" \
    "core/gamma" \
    "${sorted_output[*]}"

# ── Test: dependency ordering is correct ─────────────────────────────────────

echo "resolve_dependencies: ordering"

# alpha should come before beta (beta depends on alpha)
_check_order() {
    local a="$1" b="$2" arr=("${@:3}")
    local idx_a=-1 idx_b=-1 i
    for (( i=0; i<${#arr[@]}; i++ )); do
        [[ "${arr[$i]}" == "$a" ]] && idx_a=$i
        [[ "${arr[$i]}" == "$b" ]] && idx_b=$i
    done
    [[ $idx_a -lt $idx_b ]]
}

if _check_order "core/alpha" "core/beta" "${sorted_output[@]}"; then
    echo "  PASS  core/alpha before core/beta"
    (( _PASS++ )) || true
else
    echo "  FAIL  core/alpha should come before core/beta"
    (( _FAIL++ )) || true
fi

if _check_order "core/beta" "core/gamma" "${sorted_output[@]}"; then
    echo "  PASS  core/beta before core/gamma"
    (( _PASS++ )) || true
else
    echo "  FAIL  core/beta should come before core/gamma"
    (( _FAIL++ )) || true
fi

# ── Test: read_feature_metadata normalizes bare deps ─────────────────────────

echo "read_feature_metadata: bare dep normalization"

declare -a single_feat=("core/beta")
_RESOLVER_FEATURE_DEPS=()
_RESOLVER_PROVIDES=()
_RESOLVER_REQUIRES=()
_test_index_2=""
feature_index_build _test_index_2 2>/dev/null
read_feature_metadata "$_test_index_2" single_feat 2>/dev/null

# "beta" depends on bare name "alpha" -> should be normalized to "core/alpha"
_assert_eq \
    "beta's dep normalized to core/alpha" \
    "core/alpha" \
    "${_RESOLVER_FEATURE_DEPS[core/beta]}"

# ── Test: capability-based deps use canonical IDs ────────────────────────────

echo "read_feature_metadata: canonical IDs in provides/requires"

declare -a cap_features=("core/provider" "core/consumer")
_RESOLVER_FEATURE_DEPS=()
_RESOLVER_PROVIDES=()
_RESOLVER_REQUIRES=()
_test_index_3=""
feature_index_build _test_index_3 2>/dev/null
read_feature_metadata "$_test_index_3" cap_features 2>/dev/null

_assert_eq \
    "provider stored as canonical ID in PROVIDES" \
    "core/provider" \
    "${_RESOLVER_PROVIDES[my_capability]}"

declare -a cap_sorted_output
resolve_dependencies cap_features cap_sorted_output 2>/dev/null

# consumer requires my_capability; provider provides it.
# In sorted output, provider should come before consumer.
if _check_order "core/provider" "core/consumer" "${cap_sorted_output[@]}"; then
    echo "  PASS  core/provider before core/consumer (capability dep)"
    (( _PASS++ )) || true
else
    echo "  FAIL  core/provider should come before core/consumer"
    (( _FAIL++ )) || true
fi

# ── Test: single feature (no deps) outputs canonical ID ──────────────────────

echo "resolve_dependencies: single feature"

declare -a single=("core/alpha")
_RESOLVER_FEATURE_DEPS=()
_RESOLVER_PROVIDES=()
_RESOLVER_REQUIRES=()
_test_index_4=""
feature_index_build _test_index_4 2>/dev/null
read_feature_metadata "$_test_index_4" single 2>/dev/null
declare -a single_sorted
resolve_dependencies single single_sorted 2>/dev/null

_assert_eq \
    "single feature output is canonical" \
    "core/alpha" \
    "${single_sorted[0]}"

# ── Test: missing dependency causes error ─────────────────────────────────────

echo "resolve_dependencies: missing dep → error"

mkdir -p "$TMPDIR_FEATURES/orphan"
cat > "$TMPDIR_FEATURES/orphan/feature.yaml" <<'EOF'
spec_version: 1
mode: script
description: depends on non-existent feature
depends:
  - nonexistent
EOF

declare -a orphan_features=("core/orphan")
_RESOLVER_FEATURE_DEPS=()
_RESOLVER_PROVIDES=()
_RESOLVER_REQUIRES=()
_test_index_5=""
feature_index_build _test_index_5 2>/dev/null
read_feature_metadata "$_test_index_5" orphan_features 2>/dev/null || true
declare -a orphan_sorted
_assert_return1 \
    "missing dep causes resolve_dependencies to fail" \
    resolve_dependencies orphan_features orphan_sorted

# ── Test: real repo features produce canonical IDs ────────────────────────────

echo "resolve_dependencies: real repo features (integration smoke)"

mkdir -p "$LOADOUT_ROOT/features"
cp -R "$REPO_ROOT/features/." "$LOADOUT_ROOT/features/"

declare -a real_features=("core/git" "core/bash")
_RESOLVER_FEATURE_DEPS=()
_RESOLVER_PROVIDES=()
_RESOLVER_REQUIRES=()
_test_index_6=""
feature_index_build _test_index_6 2>/dev/null
read_feature_metadata "$_test_index_6" real_features 2>/dev/null

declare -a real_sorted
resolve_dependencies real_features real_sorted 2>/dev/null

_assert_contains \
    "repo core/git in real output" \
    "core/git" \
    "${real_sorted[*]}"

_assert_contains \
    "repo core/bash in real output" \
    "core/bash" \
    "${real_sorted[*]}"

# Verify no bare names leaked into output
for id in "${real_sorted[@]}"; do
    if [[ "$id" != */* ]]; then
        echo "  FAIL  bare name leaked into output: '$id'"
        (( _FAIL++ )) || true
    fi
done
echo "  PASS  no bare names in output (${#real_sorted[@]} features checked)"
(( _PASS++ )) || true

# ── Test: disallowed external dependency causes error ────────────────────────

echo "read_feature_metadata: external allow-list enforcement"

mkdir -p "$TMPDIR_FEATURES/extparent"
cat > "$TMPDIR_FEATURES/extparent/feature.yaml" <<'EOF'
spec_version: 1
mode: script
description: depends on disallowed external feature
depends:
    - ext/blocked
EOF

declare -a ext_parent=("core/extparent")
_RESOLVER_FEATURE_DEPS=()
_RESOLVER_PROVIDES=()
_RESOLVER_REQUIRES=()

# Phase 2: allow-list enforcement moved to resolution time.
# ext/blocked is not in the desired feature set → resolve_dependencies fails.
_test_index_7=""
feature_index_build _test_index_7 2>/dev/null
read_feature_metadata "$_test_index_7" ext_parent 2>/dev/null
declare -a ext_parent_sorted
_assert_return1 \
        "disallowed external dependency is rejected" \
        resolve_dependencies ext_parent ext_parent_sorted

# ── Test: external and user sources resolve directories ──────────────────────

echo "read_feature_metadata: external/user sources"

declare -a multi_source_features=("ext/extfeat" "user/myfeat")
_RESOLVER_FEATURE_DEPS=()
_RESOLVER_PROVIDES=()
_RESOLVER_REQUIRES=()

_test_index_8=""
feature_index_build _test_index_8 2>/dev/null
read_feature_metadata "$_test_index_8" multi_source_features 2>/dev/null || true

declare -a multi_sorted
_assert_return0 \
    "allowed external and user features resolve successfully" \
    resolve_dependencies multi_source_features multi_sorted

_assert_contains \
    "resolved output contains ext/extfeat" \
    "ext/extfeat" \
    "${multi_sorted[*]}"

_assert_contains \
    "resolved output contains user/myfeat" \
    "user/myfeat" \
    "${multi_sorted[*]}"

# ── Summary ────────────────────────────────────────────────────────────────────

_print_summary
