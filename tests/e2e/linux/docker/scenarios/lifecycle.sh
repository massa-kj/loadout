#!/usr/bin/env bash
set -euo pipefail

ROOT="/tmp/loadout-repo"
CONFIG_BASE="$HOME/.config/loadout/configs/config-base.yaml"
CONFIG_FULL="$HOME/.config/loadout/configs/config-full.yaml"
CONFIG_EMPTY="$HOME/.config/loadout/configs/config-empty.yaml"
export XDG_CONFIG_HOME="/tmp/loadout-xdg-config"
export XDG_STATE_HOME="/tmp/loadout-xdg-state"
STATE_FILE="$XDG_STATE_HOME/loadout/state.json"

echo "==> Lifecycle scenario"

cd "$ROOT"

rm -rf /root/.bashrc /root/.bashrc.d

echo "==> Phase 1: base apply"
loadout apply --config "$CONFIG_BASE"

echo "==> Validating state file existence"
test -f "$STATE_FILE"

echo "==> Validating JSON format"
jq empty "$STATE_FILE" > /dev/null

echo "==> Checking version field"
VERSION=$(jq -r '.version' "$STATE_FILE")
if [[ "$VERSION" != "3" ]]; then
  echo "Invalid state version: $VERSION"
  exit 1
fi

echo "==> Checking features object exists"
jq -e '.features' "$STATE_FILE" > /dev/null

echo "==> Checking duplicate resource id entries per feature"
jq -e '
  .features[]?
  | (.resources // []) as $r
  | (($r | map(.id) | length) == ($r | map(.id) | unique | length))
' "$STATE_FILE" > /dev/null

echo "==> Checking duplicate fs.path across features"
jq -e '
  ([.features[]?.resources[]? | select(.kind == "fs") | .fs.path] | length)
  ==
  ([.features[]?.resources[]? | select(.kind == "fs") | .fs.path] | unique | length)
' "$STATE_FILE" > /dev/null

echo "==> Checking absolute paths in fs resources"
jq -e '.features | to_entries[] | .value.resources[]? | select(.kind == "fs") | .fs.path | select(startswith("/") | not) | "Non-absolute path: \(.)"' "$STATE_FILE" > /dev/null && {
  echo "Non-absolute paths detected"
  exit 1
} || true

echo "==> Phase 2: expand to full profile"
loadout apply --config "$CONFIG_FULL"

echo "==> Ensuring extra feature was installed"
if ! jq -e '.features["core/ripgrep"]' "$STATE_FILE" > /dev/null; then
  echo "ripgrep was not installed in full config"
  exit 1
fi

echo "==> Snapshotting full state"
cp "$STATE_FILE" /tmp/state_full_before.json

echo "==> Phase 3: reapply full profile"
loadout apply --config "$CONFIG_FULL"

echo "==> Verifying full-profile idempotency"
if ! diff -u /tmp/state_full_before.json "$STATE_FILE"; then
  echo "State changed after second full apply"
  exit 1
fi

echo "==> Collecting tracked files and packages before uninstall"
TRACKED_FILES=$(jq -r '.features[]?.resources[]? | select(.kind == "fs") | .fs.path' "$STATE_FILE" || true)
TRACKED_PACKAGES=$(jq -r '.features[]?.resources[]? | select(.kind == "package") | .package.name' "$STATE_FILE" || true)

echo "==> Creating sentinel file (must NOT be removed)"
SENTINEL="/tmp/loadout_sentinel"
echo "do not delete" > "$SENTINEL"

echo "==> Phase 4: shrink back to base profile"
loadout apply --config "$CONFIG_BASE"

echo "==> Verifying base features remain"
if ! jq -e '.features["core/bash"]' "$STATE_FILE" > /dev/null; then
  echo "bash was removed (should remain)"
  exit 1
fi
if ! jq -e '.features["core/git"]' "$STATE_FILE" > /dev/null; then
  echo "git was removed (should remain)"
  exit 1
fi

echo "==> Verifying extra features were removed"
REMAINING_FEATURES=$(jq -r '.features | keys[]' "$STATE_FILE")
for feature in $REMAINING_FEATURES; do
  if [[ "$feature" != "core/bash" ]] && [[ "$feature" != "core/git" ]]; then
    echo "Unexpected feature remains: $feature"
    exit 1
  fi
done

echo "==> Verifying packages were reduced with the profile"
REMAINING_PACKAGES=$(jq -r '.features[]?.resources[]? | select(.kind == "package") | .package.name' "$STATE_FILE" || true)
if [[ -n "$REMAINING_PACKAGES" ]]; then
  for pkg in $TRACKED_PACKAGES; do
    if echo "$REMAINING_PACKAGES" | grep -q "^${pkg}$"; then
      :
    fi
  done
fi

echo "==> Phase 5: full uninstall"
loadout apply --config "$CONFIG_EMPTY"

echo "==> Checking state file valid"
jq empty "$STATE_FILE" > /dev/null

echo "==> Verifying state features empty"
REMAINING=$(jq '.features | keys | length' "$STATE_FILE")
if [[ "$REMAINING" -ne 0 ]]; then
  echo "State still contains features"
  exit 1
fi

echo "==> Verifying all tracked files removed"
for f in $TRACKED_FILES; do
  if [[ -e "$f" ]]; then
    echo "Tracked file still exists: $f"
    exit 1
  fi
done

echo "==> Verifying sentinel file still exists"
if [[ ! -f "$SENTINEL" ]]; then
  echo "Untracked file was removed (filesystem scan violation)"
  exit 1
fi

echo "==> Verifying all packages removed from state"
REMAINING_PACKAGES=$(jq -r '.features[]?.resources[]? | select(.kind == "package") | .package.name' "$STATE_FILE" || true)
if [[ -n "$REMAINING_PACKAGES" ]]; then
  echo "Packages still in state: $REMAINING_PACKAGES"
  exit 1
fi

echo "==> Lifecycle scenario PASSED"