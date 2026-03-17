#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Unit tests: compiler (FeatureCompiler)
#
# Tests that feature_compiler_run produces correct raw DesiredResourceGraph JSON
# for both script and declarative features, and enforces declarative invariants.
# The compiler only assigns stable resource IDs; desired_backend is NOT present
# in the output (PolicyResolver adds that in a subsequent step).
#
# Run directly: bash tests/unit/test_compiler.sh
# Exit code 0 = all pass, 1 = one or more failures.
# 
# 
# Cover:
# * package resource compiles without `desired_backend` (raw DRG output)
# * runtime resource compiles without `desired_backend`
# * `fs` resource has no `desired_backend`
# * Platform override replaces base resources when non-empty
# 
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

cat > "$LOADOUT_SOURCES_FILE" <<'EOF'
sources: []
EOF

source "$REPO_ROOT/core/lib/source_registry.sh"

source "$REPO_ROOT/core/lib/compiler.sh"

# ── Helpers ───────────────────────────────────────────────────────────────────

# _make_script_entry <canonical_id> <source_dir>
# Emit a Feature Index entry with mode:script.
_make_script_entry() {
    local id="$1"
    local dir="$2"
    jq -n \
        --arg dir "$dir" \
        '{spec_version: 1, mode: "script", description: "test", source_dir: $dir,
          blocked: false, blocked_reason: null,
          dep: {depends: [], provides: [], requires: []},
          spec: null}'
}

# _make_declarative_entry <canonical_id> <source_dir> <resources_json>
# Emit a Feature Index entry with mode:declarative.
_make_declarative_entry() {
    local id="$1"
    local dir="$2"
    local resources="$3"
    jq -n \
        --arg dir "$dir" \
        --argjson res "$resources" \
        '{spec_version: 1, mode: "declarative", description: "test", source_dir: $dir,
          blocked: false, blocked_reason: null,
          dep: {depends: [], provides: [], requires: []},
          spec: {resources: $res}}'
}

# _make_index <entries_json_object>
# Wrap features object into a Feature Index document.
_make_index() {
    local features="$1"
    printf '{"schema_version": 1, "features": %s}' "$features"
}

# ── Fixtures ──────────────────────────────────────────────────────────────────

# Script feature dir (no install.sh needed for declarative tests)
SCRIPT_DIR="$TMPDIR_ROOT/repo/features/scriptfeat"
mkdir -p "$SCRIPT_DIR"

# Declarative feature dir (no install.sh/uninstall.sh)
DECL_DIR="$TMPDIR_ROOT/repo/features/declfeat"
mkdir -p "$DECL_DIR"

# Declarative-but-has-install dir (should be rejected)
BAD_DECL_DIR="$TMPDIR_ROOT/repo/features/baddecl"
mkdir -p "$BAD_DECL_DIR"
touch "$BAD_DECL_DIR/install.sh"

# ── Test: mode:script → empty resources ──────────────────────────────────────

echo "feature_compiler_run: mode:script produces empty resources"

_script_entry=$(_make_script_entry "core/scriptfeat" "$SCRIPT_DIR")
_script_index=$(_make_index "$(jq -n \
    --argjson e "$_script_entry" '{"core/scriptfeat": $e}')")

_sorted_script=("core/scriptfeat")
_drg_script=$(feature_compiler_run "$_script_index" _sorted_script 2>/dev/null)

sv=$(printf '%s' "$_drg_script" | jq -r '.schema_version')
_assert_eq "DRG schema_version is 1" "1" "$sv"

res_count=$(printf '%s' "$_drg_script" \
    | jq '.features["core/scriptfeat"].resources | length')
_assert_eq "script feature has 0 resources" "0" "$res_count"

# ── Test: mode:declarative → resources expanded with stable id (no desired_backend) ──

echo "feature_compiler_run: mode:declarative expands resources with stable id"

_decl_resources='[{"kind": "package", "name": "ripgrep"}]'
_decl_entry=$(_make_declarative_entry "core/declfeat" "$DECL_DIR" "$_decl_resources")
_decl_index=$(_make_index "$(jq -n \
    --argjson e "$_decl_entry" '{"core/declfeat": $e}')")

_sorted_decl=("core/declfeat")
_drg_decl=$(feature_compiler_run "$_decl_index" _sorted_decl 2>/dev/null)

res_count_decl=$(printf '%s' "$_drg_decl" \
    | jq '.features["core/declfeat"].resources | length')
_assert_eq "declarative feature has 1 resource" "1" "$res_count_decl"

res_id=$(printf '%s' "$_drg_decl" \
    | jq -r '.features["core/declfeat"].resources[0].id // "null"')
_assert_eq "resource id is package:ripgrep" "package:ripgrep" "$res_id"

# Compiler must NOT embed desired_backend (that is PolicyResolver's job)
res_backend=$(printf '%s' "$_drg_decl" \
    | jq -r '.features["core/declfeat"].resources[0].desired_backend // "absent"')
_assert_eq "compiler does not embed desired_backend" "absent" "$res_backend"

# ── Test: mode:declarative with no resources → error ─────────────────────────

echo "feature_compiler_run: declarative with no resources → error"

_empty_decl_entry=$(_make_declarative_entry "core/emptydecl" "$DECL_DIR" '[]')
_empty_decl_index=$(_make_index "$(jq -n \
    --argjson e "$_empty_decl_entry" '{"core/emptydecl": $e}')")

_sorted_empty=("core/emptydecl")
_assert_return1 \
    "declarative with no resources causes error" \
    feature_compiler_run "$_empty_decl_index" _sorted_empty

# ── Test: mode:declarative with install.sh present → error ───────────────────

echo "feature_compiler_run: declarative with install.sh → error"

_bad_decl_resources='[{"kind": "package", "name": "something"}]'
_bad_decl_entry=$(_make_declarative_entry "core/baddecl" "$BAD_DECL_DIR" "$_bad_decl_resources")
_bad_decl_index=$(_make_index "$(jq -n \
    --argjson e "$_bad_decl_entry" '{"core/baddecl": $e}')")

_sorted_bad=("core/baddecl")
_assert_return1 \
    "declarative with install.sh causes error" \
    feature_compiler_run "$_bad_decl_index" _sorted_bad

# ── Test: multiple features in DRG ───────────────────────────────────────────

echo "feature_compiler_run: multiple features in DRG"

_multi_features_json=$(jq -n \
    --argjson s "$_script_entry" \
    --argjson d "$_decl_entry" \
    '{"core/scriptfeat": $s, "core/declfeat": $d}')
_multi_index=$(_make_index "$_multi_features_json")

_sorted_multi=("core/scriptfeat" "core/declfeat")
_drg_multi=$(feature_compiler_run "$_multi_index" _sorted_multi 2>/dev/null)

key_count=$(printf '%s' "$_drg_multi" | jq '.features | keys | length')
_assert_eq "DRG has 2 feature entries" "2" "$key_count"

# ── Test: fs resource gets id but no desired_backend ─────────────────────────

echo "feature_compiler_run: fs resource gets id only"

_fs_resources='[{"kind": "fs", "path": "/etc/myapp/config"}]'
_fs_decl_entry=$(_make_declarative_entry "core/fsfeature" "$DECL_DIR" "$_fs_resources")
_fs_index=$(_make_index "$(jq -n \
    --argjson e "$_fs_decl_entry" '{"core/fsfeature": $e}')")

_sorted_fs=("core/fsfeature")
_drg_fs=$(feature_compiler_run "$_fs_index" _sorted_fs 2>/dev/null)

fs_id=$(printf '%s' "$_drg_fs" \
    | jq -r '.features["core/fsfeature"].resources[0].id // "null"')
_assert_eq "fs resource id is fs:config" "fs:config" "$fs_id"

fs_backend=$(printf '%s' "$_drg_fs" \
    | jq -r '.features["core/fsfeature"].resources[0].desired_backend // "absent"')
_assert_eq "fs resource has no desired_backend" "absent" "$fs_backend"

# ── Summary ────────────────────────────────────────────────────────────────────

_print_summary
