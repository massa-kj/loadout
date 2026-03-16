#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Module: env
#
# Responsibility:
#   Define environment variables for loadout framework.
# -----------------------------------------------------------------------------

# Root directory of loadout
# Assumes this script is located at core/lib/env.sh
LOADOUT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export LOADOUT_ROOT

# Platform detection
if [[ -n "${WSL_DISTRO_NAME:-}" ]]; then
    LOADOUT_PLATFORM="wsl"
elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
    LOADOUT_PLATFORM="linux"
elif [[ "$OSTYPE" == "msys" ]] || [[ "$OSTYPE" == "win32" ]]; then
    LOADOUT_PLATFORM="windows"
else
    LOADOUT_PLATFORM="unknown"
fi
export LOADOUT_PLATFORM

# XDG base directories (Linux/WSL defaults)
LOADOUT_XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
LOADOUT_XDG_STATE_HOME="${XDG_STATE_HOME:-$HOME/.local/state}"
LOADOUT_XDG_DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
export LOADOUT_XDG_CONFIG_HOME
export LOADOUT_XDG_STATE_HOME
export LOADOUT_XDG_DATA_HOME

# Dotfiles XDG namespaces
LOADOUT_CONFIG_HOME="${LOADOUT_XDG_CONFIG_HOME}/loadout"
LOADOUT_STATE_HOME="${LOADOUT_XDG_STATE_HOME}/loadout"
LOADOUT_DATA_HOME="${LOADOUT_XDG_DATA_HOME}/loadout"
export LOADOUT_CONFIG_HOME
export LOADOUT_STATE_HOME
export LOADOUT_DATA_HOME

# loadout_state_file_path
# Return authoritative state file path.
loadout_state_file_path() {
    echo "${LOADOUT_STATE_HOME}/state.json"
}

# Features directory
LOADOUT_FEATURES_DIR="${LOADOUT_ROOT}/features"
export LOADOUT_FEATURES_DIR

# Maximum supported spec_version in feature.yaml.
# Features with a higher spec_version are classified as blocked.
SUPPORTED_FEATURE_SPEC_VERSION=1
export SUPPORTED_FEATURE_SPEC_VERSION

# Profiles directory (override allowed)
if [[ -z "${LOADOUT_PROFILES_DIR:-}" ]]; then
    LOADOUT_PROFILES_DIR="${LOADOUT_CONFIG_HOME}/profiles"
fi
export LOADOUT_PROFILES_DIR

# Source registry file (override allowed)
if [[ -z "${LOADOUT_SOURCES_FILE:-}" ]]; then
    LOADOUT_SOURCES_FILE="${LOADOUT_CONFIG_HOME}/sources.yaml"
fi
export LOADOUT_SOURCES_FILE

# Backend plugins directory
LOADOUT_BACKENDS_DIR="${LOADOUT_ROOT}/backends"
export LOADOUT_BACKENDS_DIR

# Policies directory (for default policy resolution)
LOADOUT_POLICIES_DIR="${LOADOUT_CONFIG_HOME}/policies"
export LOADOUT_POLICIES_DIR

# Policy file: prefer platform-specific, fall back to generic default.
# Can be overridden by setting LOADOUT_POLICY_FILE before sourcing env.sh.
if [[ -z "${LOADOUT_POLICY_FILE:-}" ]]; then
    _policy_candidate="${LOADOUT_POLICIES_DIR}/default.${LOADOUT_PLATFORM}.yaml"
    if [[ -f "$_policy_candidate" ]]; then
        LOADOUT_POLICY_FILE="$_policy_candidate"
    else
        LOADOUT_POLICY_FILE="${LOADOUT_POLICIES_DIR}/default.yaml"
    fi
    export LOADOUT_POLICY_FILE
    unset _policy_candidate
fi
