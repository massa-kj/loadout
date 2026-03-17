#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Unit tests: feature_index
#
# Tests that feature_index_build scans sources and produces valid Feature Index
# JSON, and that feature_index_filter separates valid from blocked features.
#
# Run directly: bash tests/unit/test_feature_index.sh
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
export SUPPORTED_FEATURE_SPEC_VERSION="1"

TMPDIR_FEATURES="$LOADOUT_ROOT/features"
mkdir -p "$TMPDIR_FEATURES" "$LOADOUT_ROOT/backends"
mkdir -p "$LOADOUT_CONFIG_HOME/features" "$LOADOUT_CONFIG_HOME/backends"
mkdir -p "$LOADOUT_DATA_HOME/sources"

# Empty sources.yaml (no external sources; only core is registered by default)
cat > "$LOADOUT_SOURCES_FILE" <<'EOF'
sources: []
EOF

source "$REPO_ROOT/core/lib/source_registry.sh"
source "$REPO_ROOT/core/lib/feature_index.sh"

# ── Fixtures ──────────────────────────────────────────────────────────────────

# alpha — no deps
mkdir -p "$TMPDIR_FEATURES/alpha"
cat > "$TMPDIR_FEATURES/alpha/feature.yaml" <<'EOF'
spec_version: 1
mode: script
description: alpha feature
depends: []
EOF

# beta — depends on bare name "alpha" (should normalize to core/alpha)
mkdir -p "$TMPDIR_FEATURES/beta"
cat > "$TMPDIR_FEATURES/beta/feature.yaml" <<'EOF'
spec_version: 1
mode: script
description: beta feature
depends:
  - alpha
EOF

# future — unsupported spec_version (should be blocked)
mkdir -p "$TMPDIR_FEATURES/future"
cat > "$TMPDIR_FEATURES/future/feature.yaml" <<'EOF'
spec_version: 999
mode: script
description: future feature with unknown spec
depends: []
EOF

# nodeps — minimal valid feature, no provides/requires
mkdir -p "$TMPDIR_FEATURES/nodeps"
cat > "$TMPDIR_FEATURES/nodeps/feature.yaml" <<'EOF'
spec_version: 1
mode: script
description: minimal feature
EOF

# withcap — provides and requires capabilities
mkdir -p "$TMPDIR_FEATURES/withcap"
cat > "$TMPDIR_FEATURES/withcap/feature.yaml" <<'EOF'
spec_version: 1
mode: script
description: feature with capabilities
depends: []
provides:
  - name: my_cap
requires:
  - name: other_cap
EOF

# nofile — a directory without feature.yaml (should be skipped)
mkdir -p "$TMPDIR_FEATURES/nofile"

# decl_base — declarative feature with resources only in base feature.yaml
mkdir -p "$TMPDIR_FEATURES/decl_base"
cat > "$TMPDIR_FEATURES/decl_base/feature.yaml" <<'EOF'
spec_version: 1
mode: declarative
description: declarative, base resources only
resources:
  - kind: package
    id: pkg:base-tool
    name: base-tool
EOF

# decl_plat — declarative feature with platform override (feature.linux.yaml replaces base)
mkdir -p "$TMPDIR_FEATURES/decl_plat"
cat > "$TMPDIR_FEATURES/decl_plat/feature.yaml" <<'EOF'
spec_version: 1
mode: declarative
description: declarative, platform override
resources:
  - kind: package
    id: pkg:base-tool
    name: base-tool
EOF
cat > "$TMPDIR_FEATURES/decl_plat/feature.linux.yaml" <<'EOF'
resources:
  - kind: package
    id: pkg:linux-tool
    name: linux-tool
EOF

# decl_empty_base — declarative feature with no base resources, platform provides them
mkdir -p "$TMPDIR_FEATURES/decl_empty_base"
cat > "$TMPDIR_FEATURES/decl_empty_base/feature.yaml" <<'EOF'
spec_version: 1
mode: declarative
description: declarative, no base resources
EOF
cat > "$TMPDIR_FEATURES/decl_empty_base/feature.linux.yaml" <<'EOF'
resources:
  - kind: package
    id: pkg:linux-only
    name: linux-only
EOF

# ── Tests: feature_index_build ────────────────────────────────────────────────

echo "feature_index_build: schema_version field"

_fi_index=""
feature_index_build _fi_index 2>/dev/null

sv=$(printf '%s' "$_fi_index" | jq -r '.schema_version')
_assert_eq "schema_version is 1" "1" "$sv"

# ── Test: well-formed entry for core/alpha ────────────────────────────────────

echo "feature_index_build: core/alpha entry"

alpha_entry=$(printf '%s' "$_fi_index" | jq -r '.features["core/alpha"] // "null"')
_assert_eq "core/alpha entry is present" "false" \
    "$(printf '%s' "$alpha_entry" | jq -r 'if . == "null" then "null" else "false" end')"

mode=$(printf '%s' "$alpha_entry" | jq -r '.mode')
_assert_eq "core/alpha mode is script" "script" "$mode"

blocked=$(printf '%s' "$alpha_entry" | jq -r '.blocked')
_assert_eq "core/alpha is not blocked" "false" "$blocked"

desc=$(printf '%s' "$alpha_entry" | jq -r '.description')
_assert_eq "core/alpha description" "alpha feature" "$desc"

# ── Test: bare dep "alpha" normalized to "core/alpha" ─────────────────────────

echo "feature_index_build: bare dep normalization"

beta_entry=$(printf '%s' "$_fi_index" | jq -r '.features["core/beta"] // "null"')
dep_0=$(printf '%s' "$beta_entry" | jq -r '.dep.depends[0] // "null"')
_assert_eq "beta dep 'alpha' normalized to core/alpha" "core/alpha" "$dep_0"

# ── Test: unsupported spec_version is blocked ─────────────────────────────────

echo "feature_index_build: unsupported spec_version blocked"

future_entry=$(printf '%s' "$_fi_index" | jq -r '.features["core/future"] // "null"')
fut_blocked=$(printf '%s' "$future_entry" | jq -r '.blocked')
_assert_eq "core/future is blocked" "true" "$fut_blocked"

fut_reason=$(printf '%s' "$future_entry" | jq -r '.blocked_reason // ""')
if [[ "$fut_reason" == *"unsupported spec_version"* ]]; then
    echo "  PASS  blocked_reason mentions unsupported spec_version"
    (( _PASS++ )) || true
else
    echo "  FAIL  blocked_reason should mention 'unsupported spec_version'; got: '$fut_reason'"
    (( _FAIL++ )) || true
fi

# ── Test: directory without feature.yaml is skipped ──────────────────────────

echo "feature_index_build: skips dirs without feature.yaml"

nofile_entry=$(printf '%s' "$_fi_index" | jq -r '.features["core/nofile"] // "null"')
_assert_eq "core/nofile absent from index" "null" "$nofile_entry"

# ── Test: source_dir field is set correctly ───────────────────────────────────

echo "feature_index_build: source_dir field"

alpha_dir=$(printf '%s' "$alpha_entry" | jq -r '.source_dir')
_assert_eq "core/alpha source_dir" "$TMPDIR_FEATURES/alpha" "$alpha_dir"

# ── Test: provides/requires arrays are populated ──────────────────────────────

echo "feature_index_build: provides and requires"

cap_entry=$(printf '%s' "$_fi_index" | jq -r '.features["core/withcap"] // "null"')
provides_name=$(printf '%s' "$cap_entry" | jq -r '.dep.provides[0].name // "null"')
_assert_eq "withcap provides my_cap" "my_cap" "$provides_name"

requires_name=$(printf '%s' "$cap_entry" | jq -r '.dep.requires[0].name // "null"')
_assert_eq "withcap requires other_cap" "other_cap" "$requires_name"

# ── Tests: declarative feature spec.resources ─────────────────────────────────

echo "feature_index_build: declarative feature — base resources"

decl_base_entry=$(printf '%s' "$_fi_index" | jq -r '.features["core/decl_base"] // "null"')
decl_base_res=$(printf '%s' "$decl_base_entry" | jq -r '.spec.resources[0].name // "null"')
_assert_eq "decl_base spec.resources[0].name is base-tool" "base-tool" "$decl_base_res"

echo "feature_index_build: declarative feature — platform override replaces base"

decl_plat_entry=$(printf '%s' "$_fi_index" | jq -r '.features["core/decl_plat"] // "null"')
decl_plat_count=$(printf '%s' "$decl_plat_entry" | jq -r '.spec.resources | length')
decl_plat_name=$(printf '%s' "$decl_plat_entry" | jq -r '.spec.resources[0].name // "null"')
_assert_eq "decl_plat has exactly 1 resource (platform replaces, not appends)" "1" "$decl_plat_count"
_assert_eq "decl_plat spec.resources[0].name is linux-tool (platform override)" "linux-tool" "$decl_plat_name"

echo "feature_index_build: declarative feature — empty base, platform provides resources"

decl_empty_entry=$(printf '%s' "$_fi_index" | jq -r '.features["core/decl_empty_base"] // "null"')
decl_empty_name=$(printf '%s' "$decl_empty_entry" | jq -r '.spec.resources[0].name // "null"')
_assert_eq "decl_empty_base spec.resources[0].name is linux-only" "linux-only" "$decl_empty_name"

# ── Tests: feature_index_filter ───────────────────────────────────────────────

echo "feature_index_filter: separates valid from blocked"

_fi2_desired=("core/alpha" "core/future")
_fi2_valid=()
_fi2_blocked="[]"
feature_index_filter "$_fi_index" _fi2_desired _fi2_valid _fi2_blocked 2>/dev/null

_assert_contains "valid array contains core/alpha" "core/alpha" "${_fi2_valid[*]}"

if [[ " ${_fi2_valid[*]} " == *" core/future "* ]]; then
    echo "  FAIL  core/future should not be in valid list"
    (( _FAIL++ )) || true
else
    echo "  PASS  core/future absent from valid list"
    (( _PASS++ )) || true
fi

blocked_feat=$(printf '%s' "$_fi2_blocked" | jq -r '.[0].feature // "null"')
_assert_eq "blocked array contains core/future" "core/future" "$blocked_feat"

# ── Test: feature_index_filter errors on unknown feature ─────────────────────

echo "feature_index_filter: error on unknown feature"

_fi3_desired=("core/doesnotexist")
_fi3_valid=()
_fi3_blocked="[]"
_assert_return1 \
    "unknown feature in filter causes error" \
    feature_index_filter "$_fi_index" _fi3_desired _fi3_valid _fi3_blocked

# ── Test: feature_index_build with real repo features ────────────────────────

echo "feature_index_build: real repo smoke test"

mkdir -p "$LOADOUT_ROOT/features"
cp -R "$REPO_ROOT/features/." "$LOADOUT_ROOT/features/"

_fi_real=""
feature_index_build _fi_real 2>/dev/null

git_entry=$(printf '%s' "$_fi_real" | jq -r '.features["core/git"] // "null"')
_assert_eq "real core/git is in index" "script" \
    "$(printf '%s' "$git_entry" | jq -r '.mode // "null"')"

bash_entry=$(printf '%s' "$_fi_real" | jq -r '.features["core/bash"] // "null"')
_assert_eq "real core/bash is in index" "declarative" \
    "$(printf '%s' "$bash_entry" | jq -r '.mode // "null"')"

# Verify core/git and core/bash are not blocked
git_blocked=$(printf '%s' "$_fi_real" \
    | jq '.features["core/git"] | .blocked')
_assert_eq "real core/git is not blocked" "false" "$git_blocked"

bash_blocked=$(printf '%s' "$_fi_real" \
    | jq '.features["core/bash"] | .blocked')
_assert_eq "real core/bash is not blocked" "false" "$bash_blocked"

# ── Summary ────────────────────────────────────────────────────────────────────

_print_summary
