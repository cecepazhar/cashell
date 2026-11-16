#!/usr/bin/env bash
# cashell install script — Sovereign DevOps Cockpit (RL-02)
# Usage: curl -fsSL https://cashell.fath.my.id/install.sh | bash
#        or: ./install.sh [--system] [--service]

set -euo pipefail

VERSION="${VERSION:-2.0.0}"
BIN_NAME="cashell"
INSTALL_SERVICE=false
SYSTEM_MODE=false

for arg in "$@"; do
    case "$arg" in
        --system)
            SYSTEM_MODE=true
            ;;
        --service)
            INSTALL_SERVICE=true
            ;;
        --version=*)
            VERSION="${arg#*=}"
            ;;
        *)
            ;;
    esac
done

ARCH="$(uname -m)"
case "$ARCH" in
    x86_64)  TARGET="x86_64-unknown-linux-musl" ;;
    aarch64) TARGET="aarch64-unknown-linux-musl" ;;
    armv7l)  TARGET="armv7-unknown-linux-musleabihf" ;;
    *)       echo "Unsupported architecture: $ARCH"; exit 1 ;;
esac

# Default target directory
if [ "$SYSTEM_MODE" = true ] || [ "$(id -u)" -eq 0 ]; then
    INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"
else
    INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
fi

echo "=========================================================="
echo "  CAShell v${VERSION} — Sovereign DevOps Cockpit"
echo "  Architecture : ${TARGET}"
echo "  Destination  : ${INSTALL_DIR}/${BIN_NAME}"
echo "=========================================================="

mkdir -p "$INSTALL_DIR"

# If local binary exists in target/release, use it; otherwise download
REPO_BIN="$(dirname "$0")/target/release/${BIN_NAME}"
if [ -f "$REPO_BIN" ]; then
    echo "Using local build binary from ${REPO_BIN}..."
    cp "$REPO_BIN" "${INSTALL_DIR}/${BIN_NAME}"
else
    URL="https://cashell.fath.my.id/releases/v${VERSION}/${BIN_NAME}-${TARGET}"
    echo "Downloading binary from ${URL}..."
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$URL" -o "${INSTALL_DIR}/${BIN_NAME}"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "${INSTALL_DIR}/${BIN_NAME}" "$URL"
    else
        echo "Error: curl or wget is required to download cashell" >&2
        exit 1
    fi
fi

chmod +x "${INSTALL_DIR}/${BIN_NAME}"

# Symlink 'cas' -> 'cashell'
ln -sf "${INSTALL_DIR}/${BIN_NAME}" "${INSTALL_DIR}/cas"
echo "✔ Created symlink: ${INSTALL_DIR}/cas -> ${INSTALL_DIR}/${BIN_NAME}"

# If system service installation requested or running with --service
if [ "$INSTALL_SERVICE" = true ] || [ "$SYSTEM_MODE" = true ]; then
    echo "Setting up system daemon service..."
    if [ "$(id -u)" -eq 0 ]; then
        # Create unprivileged system user 'cashell' if missing
        if ! id -u cashell >/dev/null 2>&1; then
            echo "Creating unprivileged system user 'cashell'..."
            useradd -r -s /usr/sbin/nologin -d /var/lib/cashell -m cashell
        fi

        # Setup limited sudoers configuration for UFW, smartctl, systemctl restart
        SUDOERS_FILE="/etc/sudoers.d/cashell"
        if [ -d "/etc/sudoers.d" ]; then
            echo "Configuring hardened sudoers rule at ${SUDOERS_FILE}..."
            cat << 'EOF' > "$SUDOERS_FILE"
# CAShell limited permissions for daemon operations (SV-06, H-06)
cashell ALL=(ALL) NOPASSWD: /usr/sbin/ufw, /usr/bin/smartctl, /bin/systemctl restart *
EOF
            chmod 0440 "$SUDOERS_FILE"
        fi

        "${INSTALL_DIR}/${BIN_NAME}" daemon install-service
    else
        echo "Installing user-level systemd service (non-root)..."
        "${INSTALL_DIR}/${BIN_NAME}" daemon install-service --user
    fi
fi

echo ""
echo "✔ CAShell installation successful!"
echo "  Version: $("${INSTALL_DIR}/${BIN_NAME}" version 2>/dev/null || echo "v${VERSION}")"
echo ""
echo "Quick start:"
echo "  cas              # Launch Cockpit TUI"
echo "  cas doctor       # Run system diagnostics"
echo "  cas help         # View all commands"
echo ""
