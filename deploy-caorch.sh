#!/usr/bin/env bash
# deploy-caorch.sh — Deploy lightweight CAOrch agent daemon to remote VPS nodes.
#
# Usage:
#   ./deploy-caorch.sh <node>
#
# Supported nodes:
#   ypc        — YPC Dokploy (100.127.232.32, user: cecep)
#   hostinger  — Hostinger VPS (100.76.150.46, user: root)
#   tencent    — Tencent COS Jakarta

set -euo pipefail

NODE="${1:-}"

if [ -z "$NODE" ]; then
    echo "Usage: ./deploy-caorch.sh <node>"
    echo "Available nodes: ypc, hostinger, tencent"
    exit 1
fi

declare -A NODE_IPS=(
    ["ypc"]="100.127.232.32"
    ["hostinger"]="100.76.150.46"
    ["tencent"]="150.109.112.1"
)

declare -A NODE_USERS=(
    ["ypc"]="cecep"
    ["hostinger"]="root"
    ["tencent"]="root"
)

IP="${NODE_IPS[$NODE]:-}"
USER="${NODE_USERS[$NODE]:-}"

if [ -z "$IP" ]; then
    echo "Error: Unknown node '$NODE'. Configured nodes: ypc, hostinger, tencent"
    exit 1
fi

BIN="target/release/caorch"

if [ ! -f "$BIN" ]; then
    echo "📦 Building caorch agent (release mode)..."
    cargo build --release -p caorch
fi

echo "🚀 Deploying caorch agent to $NODE ($USER@$IP)..."

# Upload binary to remote node
scp "$BIN" "$USER@$IP:/tmp/caorch"

# Install binary and symlink
ssh "$USER@$IP" "
    sudo mv /tmp/caorch /usr/local/bin/caorch && \
    sudo chmod +x /usr/local/bin/caorch && \
    sudo ln -sf /usr/local/bin/caorch /usr/local/bin/cao
"

# Install systemd unit service
ssh "$USER@$IP" "sudo bash -c 'cat > /etc/systemd/system/caorch.service << \"EOF\"
[Unit]
Description=CAOrch Fleet Agent Daemon
After=network.target tailscaled.service

[Service]
Type=simple
ExecStart=/usr/local/bin/caorch run --port 8091
Restart=always
RestartSec=10
Environment=PATH=/usr/local/bin:/usr/bin:/bin

[Install]
WantedBy=multi-user.target
EOF
systemctl daemon-reload
systemctl enable --now caorch.service
'"

echo "✔ Verifying caorch agent status on $NODE..."
ssh "$USER@$IP" "systemctl status caorch --no-pager | head -n 10" || true

echo "✅ CAOrch agent successfully deployed to $NODE ($IP)!"
