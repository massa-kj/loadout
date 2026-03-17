#!/usr/bin/env bash

set -euo pipefail

# Load core libraries
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOADOUT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

source "$LOADOUT_ROOT/core/lib/env.sh"
source "$LOADOUT_ROOT/core/lib/logger.sh"

FEATURE_NAME="rust"

log_task "Uninstalling feature: $FEATURE_NAME"

# Resources (packages, runtimes, files) are removed by executor
# (reads from state and calls backend uninstall).
# Feature state entry is also removed by executor after this script completes.

log_success "Feature $FEATURE_NAME uninstalled successfully"
