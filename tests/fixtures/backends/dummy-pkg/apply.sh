#!/usr/bin/env bash
# Dummy package backend — apply (install).
# No network access; records installation by creating a marker file under
# /tmp/loadout-dummy/packages/.
set -euo pipefail

if [[ "$LOADOUT_RESOURCE_KIND" != "Package" ]]; then
    echo "ERROR: dummy-pkg backend only supports Package resources" >&2
    exit 1
fi

MARKER_DIR="/tmp/loadout-dummy/packages"
MARKER="$MARKER_DIR/$LOADOUT_PACKAGE_NAME"

mkdir -p "$MARKER_DIR"

if [[ -f "$MARKER" ]]; then
    echo "dummy-pkg: '$LOADOUT_PACKAGE_NAME' already installed (marker present)" >&2
    exit 0
fi

touch "$MARKER"
echo "dummy-pkg: installed '$LOADOUT_PACKAGE_NAME'" >&2
