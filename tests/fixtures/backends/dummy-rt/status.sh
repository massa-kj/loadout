#!/usr/bin/env bash
# Dummy runtime backend — status query.
# Returns "installed" if the version marker exists, "not_installed" otherwise.
set -euo pipefail

if [[ "$LOADOUT_RESOURCE_KIND" != "Runtime" ]]; then
    echo "unknown"
    exit 0
fi

MARKER="/tmp/loadout-dummy/runtimes/$LOADOUT_RUNTIME_NAME/$LOADOUT_RUNTIME_VERSION"

if [[ -f "$MARKER" ]]; then
    echo "installed"
else
    echo "not_installed"
fi
