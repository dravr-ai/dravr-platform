#!/bin/bash
# ABOUTME: Stops this checkout's dev stack — server, dev fixture, Vite, Expo, tunnel
# ABOUTME: Delegates to stop-all.sh; --server-only stops just this checkout's backend

set -e

# Colors for output
YELLOW='\033[0;33m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
. "$SCRIPT_DIR/dev-processes.sh"
. "$SCRIPT_DIR/tunnel-env.sh"

usage() {
    cat <<'EOF'
Usage: ./bin/stop-server.sh [--server-only]

  (no args)       Stop THIS CHECKOUT's dev stack: server, dev fixture, Vite,
                  Expo, Metro/NativeWind workers, and the Cloudflare tunnel.
                  Another worktree's stack is named and left running.
  --server-only   Stop ONLY this checkout's backend, leaving the frontend and
                  Expo running. Used to simulate a backend outage — the
                  test-mobile-app skill relies on this to check that the app
                  shows its offline banner and Retry works.
EOF
}

case "${1:-}" in
    -h | --help)
        usage
        exit 0
        ;;
    --server-only)
        echo -e "${YELLOW}=== Pierre MCP Server Shutdown (this checkout, server only) ===${NC}"
        dev_stop pierre-server "Pierre MCP Server"
        # The port this checkout's start scripts were pinned to: a caller
        # override first, then what .envrc declares, then the default — the
        # order the start scripts resolve. Run without direnv the ambient
        # environment carries no HTTP_PORT, and the bare default is a DIFFERENT
        # checkout's port.
        dev_reclaim_port \
            "${HTTP_PORT:-$(tunnel_env_declared_port "$PROJECT_ROOT/.envrc" HTTP_PORT 8081)}" \
            "Pierre MCP Server" || true
        echo -e "${YELLOW}Frontend, Expo and the dev fixture are still running.${NC}"
        echo -e "${YELLOW}Run ./bin/stop-server.sh with no arguments to stop everything.${NC}"
        exit 0
        ;;
    "")
        # The whole stack, not the backend alone. Nothing else stops when the
        # server does: Vite, Expo and the dev fixture keep 5173/8082/9555 and
        # roughly 1.5GB of Node between them.
        exec "$SCRIPT_DIR/stop-all.sh"
        ;;
    *)
        echo "Unknown argument: $1" >&2
        usage >&2
        exit 1
        ;;
esac
