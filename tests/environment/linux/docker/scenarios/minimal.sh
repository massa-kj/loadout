#!/usr/bin/env bash
set -euo pipefail

ROOT="/loadout"
CONFIG="$ROOT/tests/environment/linux/docker/fixtures/config-base.yaml"
export XDG_CONFIG_HOME="/tmp/loadout-xdg-config"
export XDG_STATE_HOME="/tmp/loadout-xdg-state"
STATE_FILE="$XDG_STATE_HOME/loadout/state.json"

echo "==> Minimal scenario"

cd "$ROOT"

rm -rf /root/.bashrc /root/.bashrc.d

echo "==> Running apply"
./loadout apply --config "$CONFIG"

echo "==> Checking state file existence"
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

echo "==> Checking duplicate fs.path across features"
DUP_FS_PATHS=$(jq -r '[.features | to_entries[] | .value.resources[]? | select(.kind == "fs") | .fs.path] | (length != (unique | length))' "$STATE_FILE")
if [[ "$DUP_FS_PATHS" == "true" ]]; then
  echo "Duplicate fs.path entries detected"
  exit 1
fi

echo "==> Checking absolute paths in fs resources"
jq -e '.features | to_entries[] | .value.resources[]? | select(.kind == "fs") | .fs.path | select(startswith("/") | not) | "Non-absolute path: \(.)"' "$STATE_FILE" > /dev/null && {
  echo "Non-absolute paths detected"
  exit 1
} || true

echo "==> Minimal scenario PASSED"
