#!/usr/bin/env bash

set -euo pipefail

# Load core libraries
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOADOUT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

source "$LOADOUT_ROOT/core/lib/env.sh"
source "$LOADOUT_ROOT/core/lib/logger.sh"

FEATURE_NAME="uv"

log_task "Installing feature: $FEATURE_NAME"

# uv Python packages are installed by executor via backends/uv.sh.
# No manual steps required in this script.

log_success "Feature $FEATURE_NAME installed successfully"
