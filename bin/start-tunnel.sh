#!/usr/bin/env bash
# ABOUTME: Starts a Cloudflare tunnel for testing the server from a device
# ABOUTME: Rewrites the BASE_URL line in .envrc

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
. "$SCRIPT_DIR/dev-processes.sh"
. "$SCRIPT_DIR/tunnel-env.sh"
TUNNEL_LOG="/tmp/cloudflare-tunnel.log"

if [[ $# -gt 0 ]]; then
    echo "Unknown option: $1"
    echo "Usage: $0"
    exit 1
fi

# Check if cloudflared is installed
if ! command -v cloudflared &> /dev/null; then
    echo -e "\033[0;31m[ERROR]\033[0m cloudflared is not installed."
    echo "Install it with: brew install cloudflare/cloudflare/cloudflared"
    exit 1
fi

# Warn if .envrc doesn't exist (tunnel will create it, but it needs other vars)
ENVRC_FILE="$PROJECT_ROOT/.envrc"
if [[ ! -f "$ENVRC_FILE" ]]; then
    echo -e "\033[0;33m[WARN]\033[0m .envrc not found at $ENVRC_FILE"
    echo "The tunnel will create it with BASE_URL, but you need other variables."
    echo "Consider running: cp .envrc.example .envrc"
fi

# Check if Pierre server is running on port 8081
if ! curl -s http://127.0.0.1:8081/health > /dev/null 2>&1; then
    echo -e "\033[0;33m[WARN]\033[0m Pierre server not running on port 8081."
    echo "Start it with: ./bin/start-server.sh"
    echo "Continuing anyway - tunnel will connect once server starts..."
fi

# Stop this checkout's own tunnel. A name pattern would take every worktree's.
dev_stop tunnel "Cloudflare tunnel"

echo -e "\033[0;32m[INFO]\033[0m Starting Cloudflare tunnel to 127.0.0.1:8081..."

# 127.0.0.1, never `localhost`: the server binds IPv4 (HOST="localhost" is not a
# SocketAddr, so multitenant.rs falls back to 127.0.0.1), while localhost
# resolves ::1 first on macOS. An explicit loopback literal leaves cloudflared
# no address to dial but the one the server is on.
dev_spawn tunnel "$TUNNEL_LOG" cloudflared tunnel --url http://127.0.0.1:8081
TUNNEL_PID=$DEV_SPAWNED_PID

# Wait for tunnel URL to be available
echo -e "\033[0;32m[INFO]\033[0m Waiting for tunnel URL..."
for _ in $(seq 1 30); do
    TUNNEL_URL=$(grep -ao 'https://[a-z0-9-]*\.trycloudflare\.com' "$TUNNEL_LOG" 2>/dev/null | head -1)
    if [[ -n "$TUNNEL_URL" ]]; then
        break
    fi
    sleep 1
done

if [[ -z "$TUNNEL_URL" ]]; then
    echo -e "\033[0;31m[ERROR]\033[0m Failed to get tunnel URL after 30 seconds."
    echo "Check $TUNNEL_LOG for details."
    dev_stop tunnel "Cloudflare tunnel"
    exit 1
fi

echo -e "\033[0;32m[SUCCESS]\033[0m Tunnel URL: $TUNNEL_URL"

# Point .envrc at the tunnel. One anchored line: .envrc holds every secret this
# project has.
tunnel_env_arm "$PROJECT_ROOT" "$TUNNEL_URL"
echo -e "\033[0;32m[INFO]\033[0m BASE_URL points at the tunnel"

echo ""
echo -e "\033[0;33m>>> IMPORTANT: Run these commands to complete setup <<<\033[0m"
echo ""
echo "  1. In the backend directory:"
echo "     direnv allow && ./bin/stop-server.sh --server-only && ./bin/start-server.sh"
echo ""
echo "  2. The tunnel is running in the background (PID: $TUNNEL_PID)"
echo "     Stop it with: ./bin/stop-tunnel.sh"
echo ""
