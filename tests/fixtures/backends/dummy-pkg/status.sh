#!/usr/bin/env bash
# Dummy package backend — status query.
# Returns "installed" if the marker file exists, "not_installed" otherwise.
set -euo pipefail

if [[ "$LOADOUT_RESOURCE_KIND" != "Package" ]]; then
    echo "unknown"
    exit 0
fi

MARKER="/tmp/loadout-dummy/packages/$LOADOUT_PACKAGE_NAME"

if [[ -f "$MARKER" ]]; then
    echo "installed"
else
    echo "not_installed"
fi
