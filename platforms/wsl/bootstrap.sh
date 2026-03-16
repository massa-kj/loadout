#!/usr/bin/env bash
# Minimal setup for running loadout in a WSL environment

set -euo pipefail

# Color output
readonly COLOR_RESET='\033[0m'
readonly COLOR_GREEN='\033[0;32m'
readonly COLOR_BLUE='\033[0;34m'
readonly COLOR_YELLOW='\033[0;33m'

log_step() {
    echo -e "${COLOR_GREEN}==>${COLOR_RESET} $*"
}

log_info() {
    echo -e "${COLOR_BLUE}[INFO]${COLOR_RESET} $*"
}

log_warn() {
    echo -e "${COLOR_YELLOW}[WARN]${COLOR_RESET} $*"
}

# Detect LOADOUT_ROOT
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOADOUT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
export LOADOUT_ROOT

log_info "LOADOUT_ROOT: $LOADOUT_ROOT"
log_info "Platform: WSL"

# Check if running in WSL
if [[ -z "${WSL_DISTRO_NAME:-}" ]]; then
    log_warn "Not running in WSL environment, but continuing..."
fi

# Update apt cache
log_step "Updating apt package cache..."
sudo apt-get update -qq

# Install minimum required tools
log_step "Installing required dependencies..."

# Check and install git
if ! command -v git &>/dev/null; then
    log_info "Installing git..."
    sudo apt-get install -y git
else
    log_info "git already installed"
fi

# Check and install curl
if ! command -v curl &>/dev/null; then
    log_info "Installing curl..."
    sudo apt-get install -y curl
else
    log_info "curl already installed"
fi

# Check and install jq
if ! command -v jq &>/dev/null; then
    log_info "Installing jq..."
    sudo apt-get install -y jq
else
    log_info "jq already installed"
fi

# Check and install yq
if ! command -v yq &>/dev/null; then
    log_info "Installing yq..."
    # Install yq from GitHub releases
    YQ_VERSION="v4.40.5"
    YQ_BINARY="yq_linux_amd64"
    curl -sL "https://github.com/mikefarah/yq/releases/download/${YQ_VERSION}/${YQ_BINARY}" \
        -o /tmp/yq
    chmod +x /tmp/yq
    sudo mv /tmp/yq /usr/local/bin/yq
    log_info "yq installed: $(yq --version)"
else
    log_info "yq already installed"
fi

# Verify all dependencies
log_step "Verifying dependencies..."
MISSING_DEPS=()

for cmd in git curl jq yq; do
    if ! command -v "$cmd" &>/dev/null; then
        MISSING_DEPS+=("$cmd")
    fi
done

if [[ ${#MISSING_DEPS[@]} -gt 0 ]]; then
    echo "ERROR: Missing dependencies: ${MISSING_DEPS[*]}"
    exit 1
fi

log_info "All dependencies installed successfully"

# Export environment variables to a file for future use
log_step "Setting up environment..."
cat > "$LOADOUT_ROOT/.env.bootstrap" <<EOF
export LOADOUT_ROOT="$LOADOUT_ROOT"
export LOADOUT_PLATFORM="wsl"
EOF

log_info "Bootstrap environment saved to .env.bootstrap"

echo ""
log_step "Bootstrap complete!"
echo ""
echo "Next steps:"
echo "  1. Review a profile: cat profiles/wsl.yaml"
echo "  2. Apply a profile: ./loadout apply profiles/wsl.yaml"
echo ""
