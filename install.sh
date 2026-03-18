#!/usr/bin/env bash
# install.sh — Download and install loadout from GitHub Releases.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/massa-kj/loadout/main/install.sh | bash
#   bash install.sh [--version v0.1.0] [--prefix ~/.local]
#
# Layout after install:
#   <prefix>/lib/loadout/bin/loadout   (binary)
#   <prefix>/lib/loadout/cmd/*.sh      (dispatch scripts)
#   <prefix>/bin/loadout               (symlink → ../lib/loadout/bin/loadout)

set -euo pipefail

REPO="massa-kj/loadout"
PREFIX="${HOME}/.local"
VERSION=""

# ── Argument parsing ──────────────────────────────────────────────────────────

while [[ $# -gt 0 ]]; do
    case "$1" in
        --version) VERSION="$2"; shift 2 ;;
        --prefix)  PREFIX="$2";  shift 2 ;;
        *) echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

# ── Platform detection ────────────────────────────────────────────────────────

detect_target() {
    local os arch

    case "$(uname -s)" in
        Linux)  os="linux" ;;
        Darwin) os="macos" ;;
        *)
            echo "error: unsupported OS: $(uname -s)" >&2
            echo "For Windows, download the .exe from GitHub Releases manually." >&2
            exit 1
            ;;
    esac

    case "$(uname -m)" in
        x86_64)  arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *)
            echo "error: unsupported architecture: $(uname -m)" >&2
            exit 1
            ;;
    esac

    echo "${os}-${arch}"
}

TARGET="$(detect_target)"

# ── Version resolution ────────────────────────────────────────────────────────

if [[ -z "$VERSION" ]]; then
    echo "Fetching latest release..."
    VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name"' \
        | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')"
    [[ -z "$VERSION" ]] && { echo "error: failed to fetch latest version" >&2; exit 1; }
fi

echo "Installing loadout ${VERSION} (${TARGET})..."

# ── Download ──────────────────────────────────────────────────────────────────

TARBALL="loadout-${VERSION}-${TARGET}.tar.gz"
URL="https://github.com/${REPO}/releases/download/${VERSION}/${TARBALL}"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

echo "Downloading ${URL}..."
curl -fsSL --output "${TMPDIR}/${TARBALL}" "${URL}"

# ── Install ───────────────────────────────────────────────────────────────────

LIB_DIR="${PREFIX}/lib/loadout"
BIN_DIR="${PREFIX}/bin"

# Remove previous installation
rm -rf "${LIB_DIR}"
mkdir -p "${LIB_DIR}" "${BIN_DIR}"

# Extract tarball (strip the top-level directory)
tar -xzf "${TMPDIR}/${TARBALL}" -C "${LIB_DIR}" --strip-components=1

# Make scripts executable
chmod +x "${LIB_DIR}/bin/loadout"
find "${LIB_DIR}/cmd" -name "*.sh" -exec chmod +x {} +

# Symlink binary
ln -sf "${LIB_DIR}/bin/loadout" "${BIN_DIR}/loadout"

# ── Done ──────────────────────────────────────────────────────────────────────

echo ""
echo "Installed to ${LIB_DIR}"
echo "Binary available at ${BIN_DIR}/loadout"
echo ""

if ! echo ":${PATH}:" | grep -q ":${BIN_DIR}:"; then
    echo "NOTE: ${BIN_DIR} is not in your PATH."
    echo "      Add the following to your shell profile:"
    echo "        export PATH=\"${BIN_DIR}:\$PATH\""
fi
