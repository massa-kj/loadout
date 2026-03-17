#!/usr/bin/env bash

set -euo pipefail

# Load core libraries
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOADOUT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

source "$LOADOUT_ROOT/core/lib/env.sh"
source "$LOADOUT_ROOT/core/lib/logger.sh"
source "$LOADOUT_ROOT/core/lib/runner.sh"

FEATURE_NAME="rust"

log_task "Installing feature: $FEATURE_NAME"

# rust and rust-analyzer runtime installations are handled by executor
# (declared in feature.yaml runtimes section).
# Activate mise so that cargo etc. are available in PATH.
if has_command "mise"; then
    eval "$(mise activate bash 2>/dev/null)" || true
fi

if has_command "cargo"; then
    log_info "cargo is available"
fi

log_success "Feature $FEATURE_NAME installed successfully"
