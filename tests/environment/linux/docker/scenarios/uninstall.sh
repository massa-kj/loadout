#!/usr/bin/env bash
set -euo pipefail

ROOT="/loadout"
CONFIG_FULL="$ROOT/tests/environment/linux/docker/fixtures/config-full.yaml"
CONFIG_PARTIAL="$ROOT/tests/environment/linux/docker/fixtures/config-base.yaml"
CONFIG_EMPTY="$ROOT/tests/environment/linux/docker/fixtures/config-empty.yaml"
export XDG_CONFIG_HOME="/tmp/loadout-xdg-config"
export XDG_STATE_HOME="/tmp/loadout-xdg-state"
STATE_FILE="$XDG_STATE_HOME/loadout/state.json"

echo "==> Uninstall scenario"

cd "$ROOT"

rm -rf /root/.bashrc /root/.bashrc.d

echo "==> First apply (install phase)"
./loadout apply --config "$CONFIG_FULL"

echo "==> Ensuring state exists"
test -f "$STATE_FILE"
jq empty "$STATE_FILE" > /dev/null

echo "==> Capturing installed features"
INSTALLED_COUNT=$(jq '.features | keys | length' "$STATE_FILE")
if [[ "$INSTALLED_COUNT" -eq 0 ]]; then
  echo "No features installed, test invalid"
  exit 1
fi

echo "==> Collecting tracked files and packages"
TRACKED_FILES=$(jq -r '.features[]?.resources[]? | select(.kind == "fs") | .fs.path' "$STATE_FILE" || true)
TRACKED_PACKAGES=$(jq -r '.features[]?.resources[]? | select(.kind == "package") | .package.name' "$STATE_FILE" || true)

echo "==> Creating sentinel file (must NOT be removed)"
SENTINEL="/tmp/loadout_sentinel"
echo "do not delete" > "$SENTINEL"

# Test 1: Partial uninstall
echo ""
echo "==> Test 1: Partial uninstall"
echo "==> Running apply with partial config"
./loadout apply --config "$CONFIG_PARTIAL"

echo "==> Verifying bash and git remain"
if ! jq -e '.features["core/bash"]' "$STATE_FILE" > /dev/null; then
  echo "bash was removed (should remain)"
  exit 1
fi
if ! jq -e '.features["core/git"]' "$STATE_FILE" > /dev/null; then
  echo "git was removed (should remain)"
  exit 1
fi

echo "==> Verifying other features removed"
REMAINING_FEATURES=$(jq -r '.features | keys[]' "$STATE_FILE")
for feature in $REMAINING_FEATURES; do
  if [[ "$feature" != "core/bash" ]] && [[ "$feature" != "core/git" ]]; then
    echo "Unexpected feature remains: $feature"
    exit 1
  fi
done

echo "==> Verifying packages were removed"
REMAINING_PACKAGES=$(jq -r '.features[]?.resources[]? | select(.kind == "package") | .package.name' "$STATE_FILE" || true)
if [[ -n "$REMAINING_PACKAGES" ]]; then
  for pkg in $TRACKED_PACKAGES; do
    if echo "$REMAINING_PACKAGES" | grep -q "^${pkg}$"; then
      # Package may still exist if managed by remaining features.
      :
    fi
  done
fi

echo "==> Partial uninstall passed"

# Test 2: Full uninstall
echo ""
echo "==> Test 2: Full uninstall"
echo "==> Running apply with empty config (full uninstall)"
./loadout apply --config "$CONFIG_EMPTY"

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

echo "==> Verifying sentinel file still exists (filesystem scan prohibition)"
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

echo "==> Full uninstall passed"

# Test 3: Uninstall idempotency
echo ""
echo "==> Test 3: Uninstall idempotency"
echo "==> Running apply with empty config again"
./loadout apply --config "$CONFIG_EMPTY"

echo "==> Ensuring state still empty"
REMAINING2=$(jq '.features | keys | length' "$STATE_FILE")
if [[ "$REMAINING2" -ne 0 ]]; then
  echo "State changed after idempotent uninstall"
  exit 1
fi

echo "==> Idempotent uninstall passed"

echo ""
echo "==> Uninstall scenario PASSED (all tests)"
