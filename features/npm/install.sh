#!/usr/bin/env bash

set -euo pipefail

# Load core libraries
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOADOUT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

source "$LOADOUT_ROOT/core/lib/env.sh"
source "$LOADOUT_ROOT/core/lib/logger.sh"

FEATURE_NAME="npm"

log_task "Installing feature: $FEATURE_NAME"

# npm global packages are installed by executor via backends/npm.sh.
# No manual steps required in this script.

log_success "Feature $FEATURE_NAME installed successfully"
