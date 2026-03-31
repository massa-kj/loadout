#!/usr/bin/env bash
# Dummy package backend — remove (uninstall).
# Deletes the marker file created by apply.sh.
set -euo pipefail

if [[ "$LOADOUT_RESOURCE_KIND" != "Package" ]]; then
    echo "ERROR: dummy-pkg backend only supports Package resources" >&2
    exit 1
fi

MARKER="/tmp/loadout-dummy/packages/$LOADOUT_PACKAGE_NAME"

if [[ ! -f "$MARKER" ]]; then
    echo "dummy-pkg: '$LOADOUT_PACKAGE_NAME' not installed (no marker)" >&2
    exit 0
fi

rm -f "$MARKER"
echo "dummy-pkg: removed '$LOADOUT_PACKAGE_NAME'" >&2
