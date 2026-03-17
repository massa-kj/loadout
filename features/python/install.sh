#!/usr/bin/env bash

set -euo pipefail

# Load core libraries
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOADOUT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

source "$LOADOUT_ROOT/core/lib/env.sh"
source "$LOADOUT_ROOT/core/lib/logger.sh"

FEATURE_NAME="python"

log_task "Installing feature: $FEATURE_NAME"

# python and uv runtime installations are handled by executor (declared in feature.yaml runtimes).
# uv Python packages are managed by the separate `uv` feature (features/uv/).

log_success "Feature $FEATURE_NAME installed successfully"
