#!/usr/bin/env bash

set -euo pipefail

# Load core libraries
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOADOUT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

source "$LOADOUT_ROOT/core/lib/env.sh"
source "$LOADOUT_ROOT/core/lib/logger.sh"

FEATURE_NAME="node"

log_task "Installing feature: $FEATURE_NAME"

# node runtime installation is handled by executor (declared in feature.yaml runtimes).
# npm global packages are managed by the separate `npm` feature (features/npm/).

log_success "Feature $FEATURE_NAME installed successfully"
