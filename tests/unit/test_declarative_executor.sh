#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Unit tests: declarative_executor
#
# Tests that declarative_executor_run correctly installs/uninstalls resources
# for mode:declarative features across all plan operations.
#
# Test coverage:
#   1.  create:   package resource → backend_call dispatched, state recorded
#   2.  create:   fs resource      → symlink created, state recorded
#   3.  create:   fs resource (copy op) → file copied, state recorded
#   4.  create:   fs resource (fallback source) → basename convention applied
#   5.  destroy:  → _executor_remove_resources called, feature removed from state
#   6.  replace:  → remove_resources + fresh install of all spec resources
#   7.  replace_backend: same as replace
#   8.  strengthen: only add_resources installed; other spec resources skipped
#   9.  strengthen: empty add_resources → noop
#   10. create:   runtime resource → install_runtime dispatched, state recorded
#
# Run directly: bash tests/unit/test_declarative_executor.sh
# Exit code 0 = all pass, 1 = one or more failures.
# 
# 
# Cover all plan operations across all resource kinds:
# 
# | Operation | Resources |
# |---|---|
# | `create` | package, runtime, fs (link), fs (copy), fs (source fallback) |
# | `destroy` | removes state resources, removes feature from state |
# | `replace` | two-phase destroy + create |
# | `replace_backend` | same as replace |
# | `strengthen` | installs only `add_resources` list; leaves other resources untouched |
# | `strengthen` (empty) | noop — no install calls, no state patch |
# 
# Tests must use stubs for `backend_call` / `Backend-Call`, `state_patch_*` / `State-Patch*`,
# and `_executor_remove_resources` / `_Executor-RemoveResources`.
# Tests must NOT call real backends or write real state.
# 
# -----------------------------------------------------------------------------

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

source "$(dirname "${BASH_SOURCE[0]}")/helpers.sh"

TMPDIR_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_ROOT"' EXIT

_BACKEND_CALL_LOG="$TMPDIR_ROOT/backend_calls.log"
touch "$_BACKEND_CALL_LOG"

# ── Stub: _executor_* functions ───────────────────────────────────────────────
# declarative_executor.sh requires these to be defined in scope.

# Override per-test: TEST_FEATURE_DIR and TEST_FEATURE_YAML
TEST_FEATURE_DIR=""

_executor_feature_dir()                   { echo "$TEST_FEATURE_DIR"; }
_executor_resolve_feature_file()          { echo "$TEST_FEATURE_DIR/feature.yaml"; }
_executor_resolve_platform_feature_file() { echo ""; }

_REMOVED_RESOURCES=()
_executor_remove_resources() {
    _REMOVED_RESOURCES+=("$1")
}

# ── Stub: backend functions ───────────────────────────────────────────────────

_BACKEND_UNINSTALLS=()

resolve_backend_for() {
    echo "stub_backend"
}

load_backend() { return 0; }

backend_call() {
    local op="$1"; shift
    case "$op" in
        package_exists)    return 1 ;;
        install_package)   echo "pkg:$1" >> "$_BACKEND_CALL_LOG"; return 0 ;;
        uninstall_package) _BACKEND_UNINSTALLS+=("pkg:$1"); return 0 ;;
        runtime_exists)    return 1 ;;
        install_runtime)   echo "rt:$1@$2" >> "$_BACKEND_CALL_LOG"; echo "$2"; return 0 ;;
        uninstall_runtime) _BACKEND_UNINSTALLS+=("rt:$1@$2"); return 0 ;;
    esac
}

# ── Stub: state functions ─────────────────────────────────────────────────────

_PATCH_BEGIN_COUNT=0
_PATCH_FINALIZE_COUNT=0
_PATCH_RESOURCES=()   # "feature:resource_id" pairs recorded by add_resource
_PATCH_REMOVED=()     # features removed via remove_feature

state_patch_begin()          { _PATCH_BEGIN_COUNT=$(( _PATCH_BEGIN_COUNT + 1 )); }
state_patch_finalize()       { _PATCH_FINALIZE_COUNT=$(( _PATCH_FINALIZE_COUNT + 1 )); }
state_patch_add_resource()   { _PATCH_RESOURCES+=("$1:$(printf '%s' "$2" | jq -r '.id')"); }
state_patch_remove_feature() { _PATCH_REMOVED+=("$1"); }
state_has_file()             { return 1; }  # nothing managed initially

# ── Source module under test ──────────────────────────────────────────────────

source "$REPO_ROOT/core/lib/declarative_executor.sh"

# ── Reset helpers ─────────────────────────────────────────────────────────────

_reset() {
    > "$_BACKEND_CALL_LOG"
    _BACKEND_UNINSTALLS=()
    _REMOVED_RESOURCES=()
    _PATCH_BEGIN_COUNT=0
    _PATCH_FINALIZE_COUNT=0
    _PATCH_RESOURCES=()
    _PATCH_REMOVED=()
}

# _assert_logged <test_name> <expected_entry>
# Passes when the expected entry appears in the backend call log.
_assert_logged() {
    local name="$1" expected="$2"
    if grep -qxF "$expected" "$_BACKEND_CALL_LOG" 2>/dev/null; then
        echo "  PASS  $name"
        (( _PASS++ )) || true
    else
        echo "  FAIL  $name"
        echo "        expected '$expected' in log"
        echo "        log contents: $(cat "$_BACKEND_CALL_LOG" | tr '\n' ' ')"
        (( _FAIL++ )) || true
    fi
}

# _assert_not_logged <test_name> <not_expected_entry>
_assert_not_logged() {
    local name="$1" not_expected="$2"
    if grep -qxF "$not_expected" "$_BACKEND_CALL_LOG" 2>/dev/null; then
        echo "  FAIL  $name"
        echo "        unexpected '$not_expected' found in log"
        (( _FAIL++ )) || true
    else
        echo "  PASS  $name"
        (( _PASS++ )) || true
    fi
}

# _make_feature_yaml <resources_yaml>
# Write a feature.yaml into $TEST_FEATURE_DIR.
_make_feature_yaml() {
    local resources_yaml="$1"
    cat > "$TEST_FEATURE_DIR/feature.yaml" <<EOF
spec_version: 1
mode: declarative
description: test feature
depends: []

resources:
${resources_yaml}
EOF
}

# _assert_contains_item <test_name> <expected_item> <array_name>
# Passes when <expected_item> appears in the named bash array.
_assert_contains_item() {
    local name="$1" expected="$2"
    shift 2
    local item found=0
    for item in "$@"; do
        if [[ "$item" == "$expected" ]]; then
            found=1
            break
        fi
    done
    if [[ "$found" == "1" ]]; then
        echo "  PASS  $name"
        (( _PASS++ )) || true
    else
        echo "  FAIL  $name"
        echo "        expected item: '$expected'"
        echo "        array contents: $*"
        (( _FAIL++ )) || true
    fi
}

# ── Fixtures ──────────────────────────────────────────────────────────────────

FS_SOURCE_DIR="$TMPDIR_ROOT/feat_fs/files"
mkdir -p "$FS_SOURCE_DIR"
echo "test content" > "$FS_SOURCE_DIR/marker"
echo "test content" > "$FS_SOURCE_DIR/copyfile"

# ── Test 1: create – package resource ────────────────────────────────────────

echo ""
echo "── create: package resource ─────────────────────────────────────────────"

TEST_FEATURE_DIR="$TMPDIR_ROOT/feat_pkg"
mkdir -p "$TEST_FEATURE_DIR"
_make_feature_yaml '  - kind: package
    id: package:jq
    name: jq'

_reset
declarative_executor_run "core/test" "create" "{}"

_assert_eq      "create:pkg: patch began once"            "1"                "$_PATCH_BEGIN_COUNT"
_assert_eq      "create:pkg: patch finalized once"        "1"                "$_PATCH_FINALIZE_COUNT"
_assert_logged  "create:pkg: install_package jq"          "pkg:jq"
_assert_contains_item "create:pkg: state recorded jq"     "core/test:package:jq" "${_PATCH_RESOURCES[@]:-}"

# ── Test 2: create – fs resource (link op) ────────────────────────────────────

echo ""
echo "── create: fs resource (link) ────────────────────────────────────────────"

FS_TARGET="$TMPDIR_ROOT/home/.config/loadout/marker"
TEST_FEATURE_DIR="$TMPDIR_ROOT/feat_fs"

_make_feature_yaml "  - kind: fs
    id: fs:test-marker
    source: files/marker
    path: ${TMPDIR_ROOT}/home/.config/loadout/marker
    entry_type: file
    op: link"

_reset
declarative_executor_run "core/testfs" "create" "{}"

_assert_eq             "create:fs: patch began once"                 "1"                   "$_PATCH_BEGIN_COUNT"
_assert_eq             "create:fs: patch finalized once"             "1"                   "$_PATCH_FINALIZE_COUNT"
_assert_contains_item  "create:fs: state recorded fs:test-marker"    "core/testfs:fs:test-marker" "${_PATCH_RESOURCES[@]:-}"
_assert_return0        "create:fs: symlink created"                  test -L "$FS_TARGET"

# ── Test 3: create – fs resource (copy op) ────────────────────────────────────

echo ""
echo "── create: fs resource (copy op) ────────────────────────────────────────"

FS_COPY_TARGET="$TMPDIR_ROOT/home/.config/loadout/copyfile"
TEST_FEATURE_DIR="$TMPDIR_ROOT/feat_fs_copy"
mkdir -p "$TEST_FEATURE_DIR/files"
echo "copy content" > "$TEST_FEATURE_DIR/files/copyfile"

_make_feature_yaml "  - kind: fs
    id: fs:test-copy
    source: files/copyfile
    path: ${TMPDIR_ROOT}/home/.config/loadout/copyfile
    entry_type: file
    op: copy"

_reset
declarative_executor_run "core/testfscopy" "create" "{}"

_assert_return0        "create:fs(copy): file created"              test -f "$FS_COPY_TARGET"
_assert_return1        "create:fs(copy): is not a symlink"         test -L "$FS_COPY_TARGET"
_assert_contains_item  "create:fs(copy): state recorded"            "core/testfscopy:fs:test-copy" "${_PATCH_RESOURCES[@]:-}"

# ── Test 4: create – fs resource (fallback source by convention) ──────────────

echo ""
echo "── create: fs resource (source fallback: files/<basename>) ──────────────"

FS_FALLBACK_TARGET="$TMPDIR_ROOT/home/.config/loadout/marker2"
TEST_FEATURE_DIR="$TMPDIR_ROOT/feat_fallback"
mkdir -p "$TEST_FEATURE_DIR/files"
echo "fallback content" > "$TEST_FEATURE_DIR/files/marker2"

_make_feature_yaml "  - kind: fs
    id: fs:fallback
    path: ${TMPDIR_ROOT}/home/.config/loadout/marker2
    entry_type: file
    op: link"

_reset
declarative_executor_run "core/testfallback" "create" "{}"

_assert_return0        "create:fs(fallback): symlink created"       test -L "$FS_FALLBACK_TARGET"
_assert_contains_item  "create:fs(fallback): state recorded"        "core/testfallback:fs:fallback" "${_PATCH_RESOURCES[@]:-}"

# ── Test 5: destroy ────────────────────────────────────────────────────────────

echo ""
echo "── destroy ──────────────────────────────────────────────────────────────"

TEST_FEATURE_DIR="$TMPDIR_ROOT/feat_pkg"

_reset
declarative_executor_run "core/test" "destroy" "{}"

_assert_eq             "destroy: patch began once"                   "1"            "$_PATCH_BEGIN_COUNT"
_assert_eq             "destroy: patch finalized once"               "1"            "$_PATCH_FINALIZE_COUNT"
_assert_contains_item  "destroy: _executor_remove_resources called"  "core/test"    "${_REMOVED_RESOURCES[@]:-}"
_assert_contains_item  "destroy: state_patch_remove_feature called"  "core/test"    "${_PATCH_REMOVED[@]:-}"

# ── Test 6: replace ────────────────────────────────────────────────────────────

echo ""
echo "── replace ──────────────────────────────────────────────────────────────"

TEST_FEATURE_DIR="$TMPDIR_ROOT/feat_pkg"

_reset
declarative_executor_run "core/test" "replace" "{}"

# Two patch cycles: remove + re-install
_assert_eq             "replace: patch began twice"                  "2"            "$_PATCH_BEGIN_COUNT"
_assert_eq             "replace: patch finalized twice"              "2"            "$_PATCH_FINALIZE_COUNT"
_assert_contains_item  "replace: remove_resources called"            "core/test"    "${_REMOVED_RESOURCES[@]:-}"
_assert_contains_item  "replace: state_patch_remove_feature called"  "core/test"    "${_PATCH_REMOVED[@]:-}"
_assert_logged         "replace: install_package jq re-run"          "pkg:jq"

# ── Test 7: replace_backend ────────────────────────────────────────────────────

echo ""
echo "── replace_backend ──────────────────────────────────────────────────────"

TEST_FEATURE_DIR="$TMPDIR_ROOT/feat_pkg"

_reset
declarative_executor_run "core/test" "replace_backend" "{}"

_assert_contains_item  "replace_backend: remove_resources called"   "core/test"    "${_REMOVED_RESOURCES[@]:-}"
_assert_logged         "replace_backend: install_package jq"        "pkg:jq"

# ── Test 8: strengthen – only add_resources installed ─────────────────────────

echo ""
echo "── strengthen: only add_resources installed ─────────────────────────────"

TEST_FEATURE_DIR="$TMPDIR_ROOT/feat_strengthen"
mkdir -p "$TEST_FEATURE_DIR"
_make_feature_yaml '  - kind: package
    id: package:git
    name: git
  - kind: package
    id: package:curl
    name: curl'

# details.add_resources: only curl; git is already installed (would be in state)
STRENGTHEN_DETAILS='{"add_resources":[{"kind":"package","id":"package:curl"}]}'

_reset
declarative_executor_run "core/teststrengthen" "strengthen" "$STRENGTHEN_DETAILS"

_assert_eq             "strengthen: patch began once"               "1"  "$_PATCH_BEGIN_COUNT"
_assert_eq             "strengthen: patch finalized once"           "1"  "$_PATCH_FINALIZE_COUNT"

# Only curl should be installed; git should NOT be in the install list
_assert_logged         "strengthen: curl installed"                   "pkg:curl"
_assert_not_logged     "strengthen: git NOT installed (already in state)" "pkg:git"

# ── Test 9: strengthen – empty add_resources is noop ─────────────────────────

echo ""
echo "── strengthen: empty add_resources → noop ───────────────────────────────"

TEST_FEATURE_DIR="$TMPDIR_ROOT/feat_strengthen"

STRENGTHEN_EMPTY='{"add_resources":[]}'

_reset
declarative_executor_run "core/teststrengthen" "strengthen" "$STRENGTHEN_EMPTY"

_assert_eq "strengthen(empty): no install calls"     "0" "$(wc -l < "$_BACKEND_CALL_LOG")"
_assert_eq "strengthen(empty): patch not begun"      "0" "$_PATCH_BEGIN_COUNT"

# ── Test 10: create – runtime resource ────────────────────────────────────────

echo ""
echo "── create: runtime resource ─────────────────────────────────────────────"

TEST_FEATURE_DIR="$TMPDIR_ROOT/feat_runtime"
mkdir -p "$TEST_FEATURE_DIR"
_make_feature_yaml '  - kind: runtime
    id: runtime:node
    name: node
    version: "20.0.0"'

_reset
declarative_executor_run "core/testrt" "create" "{}"

_assert_logged  "create:rt: install_runtime called"    "rt:node@20.0.0"
_assert_contains_item "create:rt: state recorded"      "core/testrt:runtime:node" "${_PATCH_RESOURCES[@]:-}"

# ── Summary ────────────────────────────────────────────────────────────────────

echo ""
_print_summary
