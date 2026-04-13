#!/usr/bin/env bash
# Run Docker-based integration tests

set -euo pipefail

# Detect loadout root (this script is at tests/e2e/linux/docker/test.sh)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOADOUT_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

cd "$LOADOUT_ROOT"

# Image names per stage
IMAGE_OS="loadout-os"       # bare Ubuntu, no loadout
IMAGE_DEV="loadout-dev"     # built from source inside Docker
IMAGE_TEST="loadout-test"   # binaries built inside Docker (from builder)

DOCKERFILE="tests/e2e/linux/docker/Dockerfile"

# Color output
readonly COLOR_RESET='\033[0m'
readonly COLOR_GREEN='\033[0;32m'
readonly COLOR_BLUE='\033[0;34m'
readonly COLOR_YELLOW='\033[0;33m'

log_step() { echo -e "${COLOR_GREEN}==>${COLOR_RESET} $*"; }
log_info()  { echo -e "${COLOR_BLUE}[INFO]${COLOR_RESET} $*"; }
log_warn()  { echo -e "${COLOR_YELLOW}[WARN]${COLOR_RESET} $*"; }

# ── Usage ─────────────────────────────────────────────────────────────────────

usage() {
    local exit_code="${1:-0}"
    cat <<EOF
Usage: $(basename "$0") <command>

Run Docker-based integration tests for loadout.

Images
  Three images serve the three development stages:

  os      Bare Ubuntu + system deps, no loadout installed.
          Starting point for testing manual install (install.sh or copy a binary).

  dev     loadout built from source inside Docker.
          Self-contained; no host pre-build required.
          For quickly trying the latest code in an isolated environment.

  test    Binaries from the builder stage, installed into the OS image.
          No Rust toolchain inside; faster than dev.
          For running E2E scenarios. No host cargo build required.

Build commands
  build-os          Build the bare OS image
  build-dev         Build the self-contained dev image (cargo build inside Docker)
  build             Build the test image (cargo build runs inside Docker)

Shell commands
  os-shell          Interactive shell in the OS image (no loadout)
  dev-shell         Interactive shell in the dev image
  shell             Interactive shell in the test image

Scenario commands (use the test image)
  minimal           Run minimal scenario
  idempotent        Run idempotent scenario
  lifecycle         Run lifecycle scenario
  uninstall         Run uninstall scenario
  version-install   Run version install scenario
  version-upgrade   Run version upgrade scenario
  version-mixed     Run version mixed scenario
  managed-script    Run managed-script scenario
  all               Run all scenarios

Maintenance
  clean             Remove all test images

Examples
  $(basename "$0") build-dev        # Build self-contained dev image
  $(basename "$0") dev-shell        # Explore the dev environment
  $(basename "$0") build            # Build test image (no host cargo build needed)
  $(basename "$0") minimal          # Run minimal scenario
  $(basename "$0") all              # Run all scenarios
  $(basename "$0") os-shell         # Start from bare OS, install manually

EOF
    exit "$exit_code"
}

# ── Image builders ─────────────────────────────────────────────────────────────

build_os_image() {
    log_step "Building OS image (bare, no loadout)..."
    if ! docker build -f "$DOCKERFILE" --target os -t "$IMAGE_OS" .; then
        echo ""
        log_warn "OS image build FAILED"
        return 1
    fi
    log_step "OS image ready: $IMAGE_OS"
}

build_dev_image() {
    log_step "Building dev image (cargo build inside Docker)..."
    log_info "This may take several minutes on first run (Rust toolchain install + compile)"
    if ! docker build -f "$DOCKERFILE" --target dev -t "$IMAGE_DEV" .; then
        echo ""
        log_warn "Dev image build FAILED"
        return 1
    fi
    log_step "Dev image ready: $IMAGE_DEV"
}

build_test_image() {
    log_step "Building test image (binaries from builder stage)..."
    if ! docker build -f "$DOCKERFILE" --target test -t "$IMAGE_TEST" .; then
        echo ""
        log_warn "Test image build FAILED"
        return 1
    fi
    log_step "Test image ready: $IMAGE_TEST"
}

# ── Scenario runner ────────────────────────────────────────────────────────────

run_scenario() {
    local scenario="$1"
    log_step "Running scenario: $scenario"

    if ! docker run --rm "$IMAGE_TEST" loadout-e2e "$scenario"; then
        echo ""
        log_warn "Scenario FAILED: $scenario"
        return 1
    fi

    echo ""
    return 0
}

# ── Shell openers ──────────────────────────────────────────────────────────────

open_os_shell() {
    log_step "Opening shell in OS image (no loadout installed)"
    log_info "Repo is at /tmp/loadout-repo. Try:"
    log_info "  bash install.sh --prefix ~/.local"
    log_info "  cp target/release/loadout ~/.local/bin/"
    echo ""
    docker run --rm -it "$IMAGE_OS" /bin/bash
}

open_dev_shell() {
    log_step "Opening shell in dev image (loadout built from source)"
    log_info "loadout and loadout-e2e are installed at ~/.local/bin/"
    log_info "Config: ~/.config/loadout/configs/"
    log_info "Try:"
    log_info "  loadout apply --config ~/.config/loadout/configs/config-base.yaml"
    log_info "  loadout-e2e minimal"
    echo ""
    docker run --rm -it "$IMAGE_DEV" /bin/bash
}

open_test_shell() {
    log_step "Opening shell in test image (binaries from builder)"
    log_info "loadout and loadout-e2e are installed at ~/.local/bin/"
    log_info "Config: ~/.config/loadout/configs/"
    log_info "Try:"
    log_info "  loadout apply --config ~/.config/loadout/configs/config-base.yaml"
    log_info "  loadout-e2e minimal"
    log_info "  loadout-e2e all"
    echo ""
    docker run --rm -it "$IMAGE_TEST" /bin/bash
}

# ── Clean ──────────────────────────────────────────────────────────────────────

clean_images() {
    log_step "Removing test images..."
    docker rmi "$IMAGE_OS"   2>/dev/null && log_info "Removed $IMAGE_OS"   || true
    docker rmi "$IMAGE_DEV"  2>/dev/null && log_info "Removed $IMAGE_DEV"  || true
    docker rmi "$IMAGE_TEST" 2>/dev/null && log_info "Removed $IMAGE_TEST" || true
    log_step "Clean complete"
}

# ── Main ───────────────────────────────────────────────────────────────────────

COMMAND="${1:-}"

case "$COMMAND" in
    # Image builds
    build-os)   build_os_image ;;
    build-dev)  build_dev_image ;;
    build)      build_test_image ;;

    # Shells
    os-shell)   build_os_image  && open_os_shell ;;
    dev-shell)  build_dev_image && open_dev_shell ;;
    shell)      build_test_image && open_test_shell ;;

    # Scenarios
    minimal)        build_test_image && run_scenario "minimal" ;;
    idempotent)     build_test_image && run_scenario "idempotent" ;;
    lifecycle)      build_test_image && run_scenario "lifecycle" ;;
    uninstall)      build_test_image && run_scenario "uninstall" ;;
    version-install) build_test_image && run_scenario "version-install" ;;
    version-upgrade) build_test_image && run_scenario "version-upgrade" ;;
    version-mixed)   build_test_image && run_scenario "version-mixed" ;;
    managed-script)  build_test_image && run_scenario "managed-script" ;;

    all)
        build_test_image
        run_scenario "all"
        log_step "All scenarios passed!"
        ;;

    clean)          clean_images ;;
    help|--help|-h) usage 0 ;;

    *)
        echo "Unknown command: '${COMMAND}'"
        echo ""
        usage 1
        ;;
esac
