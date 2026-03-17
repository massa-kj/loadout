#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Unit tests: source_registry
#
# Tests canonical_id_normalize, canonical_id_parse, canonical_id_validate.
# Run directly: bash tests/unit/test_source_registry.sh
# Exit code 0 = all pass, 1 = one or more failures.
# -----------------------------------------------------------------------------

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

source "$(dirname "${BASH_SOURCE[0]}")/helpers.sh"

source "$REPO_ROOT/core/lib/source_registry.sh"

TMPDIR_SR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_SR"' EXIT

export LOADOUT_ROOT="$TMPDIR_SR/repo"
export LOADOUT_CONFIG_HOME="$TMPDIR_SR/config/loadout"
export LOADOUT_DATA_HOME="$TMPDIR_SR/data/loadout"
export LOADOUT_SOURCES_FILE="$LOADOUT_CONFIG_HOME/sources.yaml"

mkdir -p "$LOADOUT_ROOT/features" "$LOADOUT_ROOT/backends"
mkdir -p "$LOADOUT_CONFIG_HOME/features" "$LOADOUT_CONFIG_HOME/backends"
mkdir -p "$LOADOUT_DATA_HOME/sources"

# ── canonical_id_normalize ────────────────────────────────────────────────────

echo "canonical_id_normalize"

_assert_eq \
    "bare name with default=core -> core/<name>" \
    "core/git" \
    "$(canonical_id_normalize "git" "core")"

_assert_eq \
    "already canonical (user source) -> pass-through" \
    "user/myfeat" \
    "$(canonical_id_normalize "user/myfeat" "core")"

_assert_eq \
    "already canonical (external source) -> pass-through" \
    "repo-a/foo" \
    "$(canonical_id_normalize "repo-a/foo" "core")"

_assert_eq \
    "bare name with custom default source" \
    "user/tool" \
    "$(canonical_id_normalize "tool" "user")"

_assert_return1 \
    "empty name -> error" \
    canonical_id_normalize "" "core"

_assert_return1 \
    "empty default_source -> error" \
    canonical_id_normalize "git" ""

_assert_return1 \
    "nested path (a/b/c) -> invalid" \
    canonical_id_normalize "a/b/c" "core"

# ── canonical_id_validate ─────────────────────────────────────────────────────

echo "canonical_id_validate"

_assert_return0 \
    "core/git -> valid" \
    canonical_id_validate "core/git"

_assert_return0 \
    "user/myfeat -> valid" \
    canonical_id_validate "user/myfeat"

_assert_return0 \
    "external-source/tool -> valid" \
    canonical_id_validate "external-source/tool"

_assert_return1 \
    "bare name (no slash) -> invalid" \
    canonical_id_validate "git"

_assert_return1 \
    "empty string -> invalid" \
    canonical_id_validate ""

_assert_return1 \
    "nested path core/a/b -> invalid" \
    canonical_id_validate "core/a/b"

_assert_return1 \
    "leading slash (/name) -> invalid" \
    canonical_id_validate "/name"

_assert_return1 \
    "trailing slash (source/) -> invalid" \
    canonical_id_validate "source/"

# ── canonical_id_parse ────────────────────────────────────────────────────────

echo "canonical_id_parse"

src="" nm=""
canonical_id_parse "core/git" src nm
_assert_eq "core/git -> source_id=core"  "core" "$src"
_assert_eq "core/git -> name=git"        "git"  "$nm"

src="" nm=""
canonical_id_parse "user/myfeat" src nm
_assert_eq "user/myfeat -> source_id=user"    "user"    "$src"
_assert_eq "user/myfeat -> name=myfeat"       "myfeat"  "$nm"

src="" nm=""
canonical_id_parse "repo-a/foo" src nm
_assert_eq "repo-a/foo -> source_id=repo-a"   "repo-a"  "$src"
_assert_eq "repo-a/foo -> name=foo"           "foo"     "$nm"

_assert_return1 \
    "bare name passed to parse -> error" \
    canonical_id_parse "git" _dummy1 _dummy2

_assert_return1 \
    "empty string passed to parse -> error" \
    canonical_id_parse "" _dummy1 _dummy2

# ── reserved source ID constant ───────────────────────────────────────────────

echo "CANONICAL_ID_RESERVED_SOURCES"

_assert_return0 "core is in reserved list" \
    bash -c "source '$REPO_ROOT/core/lib/source_registry.sh' 2>/dev/null
             [[ \"\$CANONICAL_ID_RESERVED_SOURCES\" == *core* ]]"

_assert_return0 "user is in reserved list" \
    bash -c "source '$REPO_ROOT/core/lib/source_registry.sh' 2>/dev/null
             [[ \"\$CANONICAL_ID_RESERVED_SOURCES\" == *user* ]]"

_assert_return0 "official is in reserved list" \
    bash -c "source '$REPO_ROOT/core/lib/source_registry.sh' 2>/dev/null
             [[ \"\$CANONICAL_ID_RESERVED_SOURCES\" == *official* ]]"

# ── source_registry_load / allow list ────────────────────────────────────────

echo "source_registry_load"

cat > "$LOADOUT_SOURCES_FILE" <<'EOF'
sources:
  - id: ext
    type: git
    url: https://example.invalid/ext.git
    commit: abcdef
    allow:
      features:
        - node
      backends:
        - brew
  - id: allsrc
    type: git
    url: https://example.invalid/all.git
    commit: 123456
    allow: "*"
EOF

_assert_return0 \
    "source_registry_load accepts valid sources.yaml" \
    source_registry_load "$LOADOUT_SOURCES_FILE"

_assert_eq \
    "core feature dir is repo/features" \
    "$LOADOUT_ROOT/features" \
    "$(source_registry_get_feature_dir core)"

_assert_eq \
    "user feature dir is config/features" \
    "$LOADOUT_CONFIG_HOME/features" \
    "$(source_registry_get_feature_dir user)"

_assert_eq \
    "external feature dir is data/sources/<id>/features" \
    "$LOADOUT_DATA_HOME/sources/ext/features" \
    "$(source_registry_get_feature_dir ext)"

_assert_return0 \
    "core features are always allowed" \
    source_registry_is_allowed core git

_assert_return0 \
    "user features are always allowed" \
    source_registry_is_allowed user myfeat

_assert_return0 \
    "external allow-list permits configured feature" \
    source_registry_is_allowed ext node

_assert_return1 \
    "external allow-list denies unspecified feature" \
    source_registry_is_allowed ext python

_assert_return0 \
    "external allow * permits any feature" \
    source_registry_is_allowed allsrc anything

_assert_return0 \
    "external backend allow-list permits configured backend" \
    source_registry_is_backend_allowed ext brew

_assert_return1 \
    "external backend allow-list denies unspecified backend" \
    source_registry_is_backend_allowed ext mise

cat > "$LOADOUT_SOURCES_FILE" <<'EOF'
sources:
  - id: core
    type: git
    url: https://example.invalid/core.git
    commit: abcdef
    allow: "*"
EOF

_assert_return1 \
    "reserved source ids may not be defined in sources.yaml" \
    source_registry_load "$LOADOUT_SOURCES_FILE"

rm -f "$LOADOUT_SOURCES_FILE"

_assert_return0 \
    "source_registry_load succeeds without sources.yaml" \
    source_registry_load

_assert_return0 \
    "implicit core source works without sources.yaml" \
    source_registry_is_allowed core git

_assert_return0 \
    "implicit user source works without sources.yaml" \
    source_registry_is_allowed user myfeat

# ── Summary ───────────────────────────────────────────────────────────────────

_print_summary
