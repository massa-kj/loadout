#!/usr/bin/env bash
# -----------------------------------------------------------------------------
# Module: logger
#
# Responsibility:
#   Provide logging functions with color-coded output.
#
# Public API (Stable):
#   log_debug <message>
#   log_info <message>
#   log_success <message>
#   log_warn <message>
#   log_error <message>
#   log_task <message>
# -----------------------------------------------------------------------------

# Color definitions
readonly LOG_COLOR_RESET='\033[0m'
readonly LOG_COLOR_RED='\033[0;31m'
readonly LOG_COLOR_GREEN='\033[0;32m'
readonly LOG_COLOR_YELLOW='\033[0;33m'
readonly LOG_COLOR_BLUE='\033[0;34m'
readonly LOG_COLOR_GRAY='\033[0;90m'

# log_debug <message>
# Output debug level log message.
log_debug() {
    echo -e "${LOG_COLOR_GRAY}[DEBUG] $*${LOG_COLOR_RESET}" >&2
}

# log_info <message>
# Output info level log message.
log_info() {
    echo -e "${LOG_COLOR_BLUE}[INFO] $*${LOG_COLOR_RESET}" >&2
}

# log_success <message>
# Output success level log message.
log_success() {
    echo -e "${LOG_COLOR_GREEN}[SUCCESS] $*${LOG_COLOR_RESET}" >&2
}

# log_warn <message>
# Output warning level log message.
log_warn() {
    echo -e "${LOG_COLOR_YELLOW}[WARN] $*${LOG_COLOR_RESET}" >&2
}

# log_error <message>
# Output error level log message.
log_error() {
    echo -e "${LOG_COLOR_RED}[ERROR] $*${LOG_COLOR_RESET}" >&2
}

# log_task <message>
# Output task execution marker for start/end of processing.
log_task() {
    echo -e "${LOG_COLOR_GREEN}==>${LOG_COLOR_RESET} $*" >&2
}
