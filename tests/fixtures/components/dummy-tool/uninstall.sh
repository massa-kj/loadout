#!/usr/bin/env bash
# Dummy managed_script uninstall — simulates tool removal.
# Removes the marker file at /tmp/loadout-dummy/tools/dummy-tool.
# No network access required.
set -euo pipefail

rm -f /tmp/loadout-dummy/tools/dummy-tool
