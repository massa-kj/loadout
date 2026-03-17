#!/usr/bin/env bash

set -euo pipefail

# Load core libraries
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOADOUT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

source "$LOADOUT_ROOT/core/lib/env.sh"
source "$LOADOUT_ROOT/core/lib/logger.sh"
source "$LOADOUT_ROOT/core/lib/runner.sh"

FEATURE_NAME="brew"

log_task "Installing feature: $FEATURE_NAME"

# Package tracking is handled by executor (declared in feature.yaml packages section).

if has_command "brew"; then
    log_info "Homebrew is already installed"
    BREW_PREFIX=$(brew --prefix)
    log_info "Homebrew prefix: $BREW_PREFIX"
else
    log_info "Installing Homebrew..."

    # Install Homebrew via official bootstrap script
    /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"

    # Setup brew environment
    BREW_PREFIX="/home/linuxbrew/.linuxbrew"
    if [[ -x "$BREW_PREFIX/bin/brew" ]]; then
        eval "$($BREW_PREFIX/bin/brew shellenv)"

        # Add to shell profile if not already present
        PROFILE_FILE="$HOME/.profile"
        SHELLENV_LINE='eval "$(/home/linuxbrew/.linuxbrew/bin/brew shellenv)"'

        if [[ -f "$PROFILE_FILE" ]] && ! grep -qF "$SHELLENV_LINE" "$PROFILE_FILE"; then
            log_info "Adding brew shellenv to $PROFILE_FILE"
            echo '' >> "$PROFILE_FILE"
            echo '# Homebrew' >> "$PROFILE_FILE"
            echo "$SHELLENV_LINE" >> "$PROFILE_FILE"
        fi

        log_success "Homebrew installed successfully"
    else
        log_error "Homebrew installation failed"
        exit 1
    fi
fi

log_success "Feature $FEATURE_NAME installed successfully"
log_info "Reload your shell or run: eval \"\$(brew shellenv)\""
