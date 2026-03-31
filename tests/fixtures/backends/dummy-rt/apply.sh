#!/usr/bin/env bash
# Dummy runtime backend — apply (install).
# No network access; records installation by creating a marker file under
# /tmp/loadout-dummy/runtimes/<name>/<version>.
set -euo pipefail

if [[ "$LOADOUT_RESOURCE_KIND" != "Runtime" ]]; then
    echo "ERROR: dummy-rt backend only supports Runtime resources" >&2
    exit 1
fi

MARKER_DIR="/tmp/loadout-dummy/runtimes/$LOADOUT_RUNTIME_NAME"
MARKER="$MARKER_DIR/$LOADOUT_RUNTIME_VERSION"

mkdir -p "$MARKER_DIR"

if [[ -f "$MARKER" ]]; then
    echo "dummy-rt: '$LOADOUT_RUNTIME_NAME@$LOADOUT_RUNTIME_VERSION' already installed (marker present)" >&2
    exit 0
fi

touch "$MARKER"
echo "dummy-rt: installed '$LOADOUT_RUNTIME_NAME@$LOADOUT_RUNTIME_VERSION'" >&2
