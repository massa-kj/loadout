#!/usr/bin/env bash
# Run Docker-based integration tests

set -euo pipefail

# Detect loadout root (this script is at tests/e2e/linux/docker/test.sh)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOADOUT_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

cd "$LOADOUT_ROOT"

IMAGE_BASE="loadout-base"
IMAGE_NAME="loadout-test"
DOCKERFILE="tests/e2e/linux/docker/Dockerfile"

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

# Usage
usage() {
    local exit_code="${1:-0}"
    cat <<EOF
Usage: $(basename "$0") <command>

Run Docker-based integration tests for loadout.

Commands:
  build              Build installed test image (default for scenarios)
  build-base         Build base image (minimal OS + repo for binary validation)
  lifecycle          Run consolidated lifecycle scenario
  minimal            Run minimal scenario (debug)
  idempotent         Run idempotent scenario (debug)
  uninstall          Run uninstall scenario (debug)
  version-install    Run version install scenario
  version-upgrade    Run version upgrade scenario
  version-mixed      Run version mixed scenario
  all                Run all scenarios (default)
  shell              Open interactive shell in installed container
  base-shell         Open interactive shell in base container
  clean              Remove test images

Examples:
  $(basename "$0") build            # Build installed image
  $(basename "$0") minimal          # Run minimal test only
  $(basename "$0") all              # Run all scenarios
  $(basename "$0") shell            # Shell with loadout installed
  $(basename "$0") base-shell       # Shell for binary validation

EOF
    exit "$exit_code"
}

# Build base image (platforms/bootstrap stage)
build_base_image() {
    log_step "Building base image (pre-bootstrap)..."
    log_info "Target: base  →  $IMAGE_BASE"

    docker build -f "$DOCKERFILE" --target base -t "$IMAGE_BASE" .

    log_step "Base image build complete"
}

# Build bootstrapped test image
build_image() {
    log_step "Building installed test image..."
    log_info "Target: installed  →  $IMAGE_NAME"
    log_info "Dockerfile: $DOCKERFILE"

    docker build -f "$DOCKERFILE" --target installed -t "$IMAGE_NAME" .

    log_step "Installed image build complete"
}

# Run scenario
run_scenario() {
    local scenario="$1"
    local script="./tests/e2e/linux/docker/scenarios/${scenario}.sh"
    
    log_step "Running ${scenario} scenario..."
    
    if ! docker run --rm "$IMAGE_NAME" "$script"; then
        echo ""
        log_info "Test failed: $scenario"
        return 1
    fi
    
    echo ""
    return 0
}

# Clean test images
clean_image() {
    log_step "Removing test images..."
    docker rmi "$IMAGE_NAME" 2>/dev/null || true
    docker rmi "$IMAGE_BASE" 2>/dev/null || true
    log_step "Clean complete"
}

# Open interactive shell in bootstrapped container
open_shell() {
    log_step "Opening interactive shell in installed container..."
    log_info "loadout is installed at ~/.local/bin/loadout. You can run:"
    log_info "  loadout plan -c ~/.config/loadout/configs/config-base.yaml"
    log_info "  loadout apply -c ~/.config/loadout/configs/config-base.yaml"
    log_info "  ./tests/e2e/linux/docker/scenarios/minimal.sh"
    log_info ""
    log_info "Environment:"
    log_info "  user source: ~/.config/loadout (features/ and backends/ location)"
    echo ""

    docker run --rm -it "$IMAGE_NAME" /bin/bash
}

# Open interactive shell in base container (before bootstrap)
open_base_shell() {
    log_step "Opening interactive shell in base container (pre-installation)..."
    log_info "Repository is available at /tmp/loadout-repo."
    log_info "You can test pre-release binaries:"
    log_info "  ./target/debug/loadout --help"
    log_info "  ./target/debug/loadout plan -c tests/fixtures/configs/config-base.yaml"
    echo ""

    docker run --rm -it "$IMAGE_BASE" /bin/bash
}

# Main
COMMAND="${1:-}"

case "$COMMAND" in
    build)
        build_image
        ;;
    build-base)
        build_base_image
        ;;
    minimal)
        build_image
        run_scenario "minimal"
        ;;
    lifecycle)
        build_image
        run_scenario "lifecycle"
        ;;
    idempotent)
        build_image
        run_scenario "idempotent"
        ;;
    uninstall)
        build_image
        run_scenario "uninstall"
        ;;
    version-install)
        build_image
        run_scenario "version_install"
        ;;
    version-upgrade)
        build_image
        run_scenario "version_upgrade"
        ;;
    version-mixed)
        build_image
        run_scenario "version_mixed"
        ;;
    all)
        build_image
        run_scenario "lifecycle"
        # run_scenario "version_install"
        # run_scenario "version_upgrade"
        # run_scenario "version_mixed"
        log_step "All tests passed!"
        ;;
    shell)
        build_image
        open_shell
        ;;
    base-shell)
        build_base_image
        open_base_shell
        ;;
    clean)
        clean_image
        ;;
    help|--help|-h)
        usage 0
        ;;
    *)
        echo "Unknown command: $COMMAND"
        echo ""
        usage 1
        ;;
esac
