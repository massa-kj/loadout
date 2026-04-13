#!/usr/bin/env bash
# Dummy managed_script install — simulates tool installation.
# Creates a marker file at /tmp/loadout-dummy/tools/dummy-tool.
# No network access required.
set -euo pipefail

mkdir -p /tmp/loadout-dummy/tools
touch /tmp/loadout-dummy/tools/dummy-tool
