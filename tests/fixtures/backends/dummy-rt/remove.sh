#!/usr/bin/env bash
# Dummy runtime backend — remove (uninstall).
# Deletes the version marker created by apply.sh.
set -euo pipefail

if [[ "$LOADOUT_RESOURCE_KIND" != "Runtime" ]]; then
    echo "ERROR: dummy-rt backend only supports Runtime resources" >&2
    exit 1
fi

MARKER="/tmp/loadout-dummy/runtimes/$LOADOUT_RUNTIME_NAME/$LOADOUT_RUNTIME_VERSION"

if [[ ! -f "$MARKER" ]]; then
    echo "dummy-rt: '$LOADOUT_RUNTIME_NAME@$LOADOUT_RUNTIME_VERSION' not installed (no marker)" >&2
    exit 0
fi

rm -f "$MARKER"
echo "dummy-rt: removed '$LOADOUT_RUNTIME_NAME@$LOADOUT_RUNTIME_VERSION'" >&2
