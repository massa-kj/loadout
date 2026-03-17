#!/usr/bin/env bash

set -euo pipefail

# Load core libraries
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOADOUT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

source "$LOADOUT_ROOT/core/lib/env.sh"
source "$LOADOUT_ROOT/core/lib/logger.sh"

FEATURE_NAME="brew"

log_task "Uninstalling feature: $FEATURE_NAME"

# Resources (packages, runtimes, files) are removed by executor
# (reads from state and calls backend uninstall).
# Feature state entry is also removed by executor after this script completes.

# Note: We do NOT uninstall Homebrew itself as it may have many packages
# log_warn "Homebrew itself is not uninstalled (many packages may depend on it)"
# log_info "To manually uninstall Homebrew, run:"
# log_info "  /bin/bash -c \"\$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/uninstall.sh)\""

log_success "Feature $FEATURE_NAME uninstalled successfully"
