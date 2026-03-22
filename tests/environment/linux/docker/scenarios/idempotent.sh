#!/usr/bin/env bash
set -euo pipefail

ROOT="/loadout"
CONFIG="$ROOT/tests/environment/linux/docker/fixtures/config-base.yaml"
export XDG_CONFIG_HOME="/tmp/loadout-xdg-config"
export XDG_STATE_HOME="/tmp/loadout-xdg-state"
STATE_FILE="$XDG_STATE_HOME/loadout/state.json"

echo "==> Idempotent scenario"

cd "$ROOT"

rm -rf /root/.bashrc /root/.bashrc.d

echo "==> First apply"
./loadout apply --config "$CONFIG"

echo "==> Snapshotting state"
cp "$STATE_FILE" /tmp/state_before.json

echo "==> Second apply"
./loadout apply --config "$CONFIG"

echo "==> Comparing state"
if ! diff -u /tmp/state_before.json "$STATE_FILE"; then
  echo "State changed after second apply"
  exit 1
fi

echo "==> Verifying no duplicate resource id entries per feature"
jq -e '
  .features[]?
  | (.resources // []) as $r
  | (($r | map(.id) | length) == ($r | map(.id) | unique | length))
' "$STATE_FILE" > /dev/null

echo "==> Verifying no duplicate fs.path entries across features"
jq -e '
  ([.features[]?.resources[]? | select(.kind == "fs") | .fs.path] | length)
  ==
  ([.features[]?.resources[]? | select(.kind == "fs") | .fs.path] | unique | length)
' "$STATE_FILE" > /dev/null

echo "==> Idempotent scenario PASSED"
