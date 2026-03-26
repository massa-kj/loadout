#!/usr/bin/env bash
set -euo pipefail

ROOT="/tmp/loadout-repo"
CONFIG_V20="$HOME/.config/loadout/configs/config-version-v20.yaml"
CONFIG_V22="$HOME/.config/loadout/configs/config-version-v22.yaml"
export XDG_CONFIG_HOME="/tmp/loadout-xdg-config"
export XDG_STATE_HOME="/tmp/loadout-xdg-state"
STATE_FILE="$XDG_STATE_HOME/loadout/state.json"

echo "==> Version upgrade scenario"

cd "$ROOT"

echo "==> First apply (Node 20)"
loadout apply --config "$CONFIG_V20"

# Activate brew and mise for tests
eval "$(/home/linuxbrew/.linuxbrew/bin/brew shellenv bash)"
eval "$(mise activate bash)"

echo "==> Verifying Node 20 installed"
NODE_VERSION_1=$(jq -r '.features["core/node"].resources[] | select(.kind == "runtime") | .runtime.version' "$STATE_FILE")
if [[ "$NODE_VERSION_1" != "20" ]]; then
  echo "Node version not recorded correctly: $NODE_VERSION_1"
  exit 1
fi

echo "==> Verifying node 20 package in state"
NODE_PACKAGE=$(jq -r '.features["core/node"].resources[] | select(.kind == "package" and (.package.name | startswith("node@"))) | .package.name' "$STATE_FILE")
if [[ ! "$NODE_PACKAGE" =~ ^node@20 ]]; then
  echo "Node 20 package not registered: $NODE_PACKAGE"
  exit 1
fi

echo ""
echo "==> Second apply (Node 22 - should trigger reinstall)"
loadout apply --config "$CONFIG_V22"

echo "==> Verifying Node 22 installed"
NODE_VERSION_2=$(jq -r '.features["core/node"].resources[] | select(.kind == "runtime") | .runtime.version' "$STATE_FILE")
if [[ "$NODE_VERSION_2" != "22" ]]; then
  echo "Node version not updated correctly: $NODE_VERSION_2"
  exit 1
fi

echo "==> Verifying node 22 package in state"
NODE_PACKAGE=$(jq -r '.features["core/node"].resources[] | select(.kind == "package" and (.package.name | startswith("node@"))) | .package.name' "$STATE_FILE")
if [[ ! "$NODE_PACKAGE" =~ ^node@22 ]]; then
  echo "Node 22 package not registered: $NODE_PACKAGE"
  exit 1
fi

echo "==> Verifying version changed from 20 to 22"
if [[ "$NODE_VERSION_1" == "$NODE_VERSION_2" ]]; then
  echo "Version did not change"
  exit 1
fi

echo ""
echo "==> Version upgrade scenario PASSED"
