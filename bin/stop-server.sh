#!/bin/bash
# ABOUTME: Stops the Pierre dev stack — server, dev fixture, Vite, Expo, tunnel
# ABOUTME: Delegates to stop-all.sh; --server-only stops just the backend

set -e

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

usage() {
    cat <<'EOF'
Usage: ./bin/stop-server.sh [--server-only]

  (no args)       Stop the whole dev stack: server, dev fixture, Vite, Expo,
                  Metro/NativeWind workers, and the Cloudflare tunnel.
  --server-only   Stop ONLY the backend on :8081, leaving the frontend and
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
        echo -e "${YELLOW}=== Pierre MCP Server Shutdown (server only) ===${NC}"
        if pgrep -f "pierre-mcp-server" > /dev/null; then
            pkill -f "pierre-mcp-server" 2>/dev/null || true
            sleep 2
            if pgrep -f "pierre-mcp-server" > /dev/null; then
                echo -e "${YELLOW}Force killing remaining processes...${NC}"
                pkill -9 -f "pierre-mcp-server" 2>/dev/null || true
                sleep 1
            fi
            echo -e "${GREEN}Pierre MCP Server stopped${NC}"
        else
            echo -e "${YELLOW}No running Pierre MCP Server instances found${NC}"
        fi
        # Cargo may still be holding a build of the server.
        pkill -f "cargo.*pierre-mcp-server" 2>/dev/null || true
        echo -e "${YELLOW}Frontend, Expo and the dev fixture are still running.${NC}"
        echo -e "${YELLOW}Run ./bin/stop-server.sh with no arguments to stop everything.${NC}"
        exit 0
        ;;
    "")
        # Default: stop everything. This used to stop only the backend, which
        # silently left Vite, Expo and the dev fixture holding 5173/8082/9555
        # (and ~1.5GB of Node) after every "shutdown".
        exec "$SCRIPT_DIR/stop-all.sh"
        ;;
    *)
        echo "Unknown argument: $1" >&2
        usage >&2
        exit 1
        ;;
esac
