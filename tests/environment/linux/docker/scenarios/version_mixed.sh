#!/usr/bin/env bash
set -euo pipefail

ROOT="/loadout"
PROFILE_MIXED="$ROOT/tests/environment/linux/docker/fixtures/profile-version-mixed.yaml"
export XDG_CONFIG_HOME="/tmp/loadout-xdg-config"
export XDG_STATE_HOME="/tmp/loadout-xdg-state"
STATE_FILE="$XDG_STATE_HOME/loadout/state.json"

echo "==> Version mixed scenario"

cd "$ROOT"

# Use test-specific policy (no backup, standard backends)
export LOADOUT_POLICY_FILE="$ROOT/tests/environment/linux/docker/fixtures/policy.yaml"

echo "==> Running apply with mixed features"
./loadout apply "$PROFILE_MIXED"

echo "==> Checking state file existence"
test -f "$STATE_FILE"

echo "==> Validating JSON format"
jq empty "$STATE_FILE" > /dev/null

echo "==> Verifying node has version in state"
NODE_VERSION=$(jq -r '.features["core/node"].resources[] | select(.kind == "runtime") | .runtime.version' "$STATE_FILE")
if [[ "$NODE_VERSION" != "20" ]]; then
  echo "Node version not recorded: $NODE_VERSION"
  exit 1
fi

echo "==> Verifying git has no version in state"
GIT_RUNTIME_COUNT=$(jq -r '[.features["core/git"].resources[]? | select(.kind == "runtime")] | length' "$STATE_FILE")
if [[ "$GIT_RUNTIME_COUNT" != "0" ]]; then
  echo "Git should not have runtime recorded"
  exit 1
fi

echo "==> Verifying bash has no version in state"
BASH_RUNTIME_COUNT=$(jq -r '[.features["core/bash"].resources[]? | select(.kind == "runtime")] | length' "$STATE_FILE")
if [[ "$BASH_RUNTIME_COUNT" != "0" ]]; then
  echo "Bash should not have runtime recorded"
  exit 1
fi

echo "==> Verifying all features installed"
FEATURE_COUNT=$(jq '.features | keys | length' "$STATE_FILE")
if [[ "$FEATURE_COUNT" -lt 5 ]]; then
  echo "Not all features installed: $FEATURE_COUNT"
  exit 1
fi

echo ""
echo "==> Version mixed scenario PASSED"
