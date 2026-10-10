#!/usr/bin/env bash
# cashell install script
# Usage: curl -fsSL https://cashell.fath.my.id/install.sh | sh

set -e

ARCH="$(uname -m)"
case "$ARCH" in
    x86_64)  TARGET="x86_64-unknown-linux-musl" ;;
    aarch64) TARGET="aarch64-unknown-linux-musl" ;;
    armv7l)  TARGET="armv7-unknown-linux-musleabihf" ;;
    *)       echo "Unsupported arch: $ARCH"; exit 1 ;;
esac

VERSION="${VERSION:-0.1.0}"
BIN_NAME="cashell"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
URL="https://cashell.fath.my.id/releases/v${VERSION}/${BIN_NAME}-${TARGET}"

echo "Installing cashell v${VERSION} (${TARGET})..."
echo "Destination: ${INSTALL_DIR}/${BIN_NAME}"

mkdir -p "$INSTALL_DIR"
curl -fsSL "$URL" -o "${INSTALL_DIR}/${BIN_NAME}"
chmod +x "${INSTALL_DIR}/${BIN_NAME}"

# Verify
"${INSTALL_DIR}/${BIN_NAME}" version

echo ""
echo "Done! Add to PATH if needed:"
echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
echo ""
