#!/bin/bash
# ABOUTME: Pierre MCP Server startup script with proper environment loading
# ABOUTME: Loads .envrc, creates data directory, and starts server with logging and optional tunnel

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
NC='\033[0m'

# Parse arguments
START_TUNNEL=false
while [[ $# -gt 0 ]]; do
    case $1 in
        --tunnel)
            START_TUNNEL=true
            shift
            ;;
        *)
            shift
            ;;
    esac
done

# Find project root (where Cargo.toml is)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
. "$SCRIPT_DIR/dev-processes.sh"
. "$SCRIPT_DIR/tunnel-env.sh"

echo -e "${BLUE}=== Pierre MCP Server Startup ===${NC}"
echo -e "Project root: ${PROJECT_ROOT}"

cd "$PROJECT_ROOT"

# A port named on the command line outranks the one .envrc pins. `set -a;
# source` below overwrites the environment it is given, so the caller's value is
# held here and reasserted after — without this, HTTP_PORT=8091 is silently
# ignored and a second checkout has no way to avoid the first one's port.
HTTP_PORT_REQUESTED="${HTTP_PORT:-}"

# Load .envrc
ENVRC_PATH="$PROJECT_ROOT/.envrc"
if [ -f "$ENVRC_PATH" ]; then
    echo -e "${GREEN}Loading environment from: ${ENVRC_PATH}${NC}"
    set -a
    # shellcheck disable=SC1090  # path is resolved at runtime from PROJECT_ROOT
    source "$ENVRC_PATH"
    set +a
else
    echo -e "${RED}ERROR: .envrc not found at ${ENVRC_PATH}${NC}"
    echo -e "${RED}Please create .envrc with required environment variables${NC}"
    echo -e "${RED}Run: cp .envrc.example .envrc${NC}"
    exit 1
fi

# Validate critical environment variables
MISSING_VARS=()
[ -z "$DATABASE_URL" ] && MISSING_VARS+=("DATABASE_URL")
[ -z "$PIERRE_MASTER_ENCRYPTION_KEY" ] && MISSING_VARS+=("PIERRE_MASTER_ENCRYPTION_KEY")
# Sciotte backpressure limiter — mandatory at startup, no crate defaults.
[ -z "$PIERRE_SCIOTTE_MAX_CONCURRENT" ] && MISSING_VARS+=("PIERRE_SCIOTTE_MAX_CONCURRENT")
[ -z "$PIERRE_SCIOTTE_MAX_QUEUE" ] && MISSING_VARS+=("PIERRE_SCIOTTE_MAX_QUEUE")
[ -z "$PIERRE_SCIOTTE_ACQUIRE_TIMEOUT_SECS" ] && MISSING_VARS+=("PIERRE_SCIOTTE_ACQUIRE_TIMEOUT_SECS")
[ -z "$PIERRE_SCIOTTE_PERMIT_MAX_LIFETIME_SECS" ] && MISSING_VARS+=("PIERRE_SCIOTTE_PERMIT_MAX_LIFETIME_SECS")
[ -z "$PIERRE_SCIOTTE_WATCHDOG_INTERVAL_SECS" ] && MISSING_VARS+=("PIERRE_SCIOTTE_WATCHDOG_INTERVAL_SECS")
[ -z "$PIERRE_SCIOTTE_RETRY_AFTER_HINT_SECS" ] && MISSING_VARS+=("PIERRE_SCIOTTE_RETRY_AFTER_HINT_SECS")
[ -z "$PIERRE_SCIOTTE_CLOSED_RETRY_AFTER_SECS" ] && MISSING_VARS+=("PIERRE_SCIOTTE_CLOSED_RETRY_AFTER_SECS")

if [ ${#MISSING_VARS[@]} -ne 0 ]; then
    echo -e "${RED}ERROR: Missing required environment variables:${NC}"
    for var in "${MISSING_VARS[@]}"; do
        echo -e "${RED}  - $var${NC}"
    done
    echo -e "${RED}Please check your .envrc file${NC}"
    exit 1
fi

echo -e "${GREEN}Environment validated successfully${NC}"

# Ensure data and logs directories exist
mkdir -p "$PROJECT_ROOT/data"
mkdir -p "$PROJECT_ROOT/logs"

# Set sensible defaults
export RUST_LOG="${RUST_LOG:-info}"
export HTTP_PORT="${HTTP_PORT_REQUESTED:-${HTTP_PORT:-8081}}"

# Start Cloudflare tunnel if requested
TUNNEL_URL=""
TUNNEL_PID=""
if [ "$START_TUNNEL" = "true" ]; then
    echo -e "${BLUE}Starting Cloudflare tunnel...${NC}"
    if ! command -v cloudflared &> /dev/null; then
        echo -e "${RED}cloudflared not installed. Run: brew install cloudflare/cloudflare/cloudflared${NC}"
        echo -e "${YELLOW}Skipping tunnel setup${NC}"
    else
        dev_stop tunnel "Cloudflare tunnel"
        TUNNEL_LOG="$PROJECT_ROOT/logs/tunnel.log"
        # 127.0.0.1, never `localhost`: the server binds IPv4 (HOST="localhost"
        # is not a SocketAddr, so multitenant.rs falls back to 127.0.0.1), while
        # localhost resolves ::1 first on macOS. An explicit loopback literal
        # leaves cloudflared no address to dial but the one the server is on.
        dev_spawn tunnel "$TUNNEL_LOG" cloudflared tunnel --url "http://127.0.0.1:$HTTP_PORT"
        TUNNEL_PID=$DEV_SPAWNED_PID

        # Wait for tunnel URL
        for _ in $(seq 1 30); do
            TUNNEL_URL=$(grep -ao 'https://[a-z0-9-]*\.trycloudflare\.com' "$TUNNEL_LOG" 2>/dev/null | head -1) || true
            if [ -n "$TUNNEL_URL" ]; then
                break
            fi
            sleep 1
        done

        if [ -n "$TUNNEL_URL" ]; then
            echo -e "${GREEN}Tunnel URL: ${TUNNEL_URL}${NC}"
            # Exported for the server spawned below, not written into .envrc:
            # this script's BASE_URL lives as long as the process it starts.
            export BASE_URL="$TUNNEL_URL"
            # The mobile app reads its API base from a file Expo loads at build
            # time, so that one is rewritten — a single anchored line, leaving
            # the Firebase and Google client ids beside it intact.
            tunnel_env_set_plain "$PROJECT_ROOT/frontend-mobile/.env" EXPO_PUBLIC_API_URL "$TUNNEL_URL"
        else
            echo -e "${RED}Failed to get tunnel URL. Check: tail -f $TUNNEL_LOG${NC}"
        fi
    fi
fi

echo -e "${BLUE}Starting Pierre MCP Server on port ${HTTP_PORT}...${NC}"
echo -e "Log level: ${RUST_LOG}"
if [ -n "$TUNNEL_URL" ]; then
    echo -e "Tunnel: ${TUNNEL_URL}"
fi
echo ""

# Build first, then run the binary. The recorded pid is then the server's own,
# so it is the pid that must appear behind the port and the pid a stop signals;
# `cargo run` would record the build supervisor, which can never be either.
# bin/setup-db-with-seeds-and-oauth-and-start-servers.sh runs the same built
# binary with the same default features, so there is one way the server starts.
SERVER_LOG="$PROJECT_ROOT/logs/pierre-server.log"
SERVER_BIN="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}/debug/pierre-mcp-server"
cargo build --bin pierre-mcp-server
if [ ! -x "$SERVER_BIN" ]; then
    echo -e "${RED}Build produced no binary at $SERVER_BIN${NC}"
    exit 1
fi

# Clear the port immediately before spawning, not before the build: a port taken
# minutes ahead of the process that needs it is a peer's stack stopped for
# nothing. A start is the one place that takes a port off a stranger — the
# operator asked for HTTP_PORT here — so the single listener is named (pid,
# directory, command) and then that one pid is signalled. Never a name pattern:
# "pierre-mcp-server" matches every worktree on the machine.
dev_stop pierre-server "Pierre MCP Server"
if ! dev_take_port "$HTTP_PORT" "Pierre MCP Server"; then
    echo -e "${RED}Port $HTTP_PORT is still held. Start on a free port instead: HTTP_PORT=8091 ./bin/start-server.sh${NC}"
    exit 1
fi

dev_spawn pierre-server "$SERVER_LOG" "$SERVER_BIN"
SERVER_PID=$DEV_SPAWNED_PID

# Wait for the server to be healthy AND to be the process behind the port.
#
# The probe dials 127.0.0.1, never `localhost`: the server binds IPv4, but
# `localhost` resolves to ::1 first on macOS, so when anything else holds IPv6
# *:8081 — an `expo start` that grabbed the reserved port is the usual culprit —
# curl reaches that instead and a serving server reads as "failed to start".
# Ownership is the other half: a 200 proves the port answered, and this server
# can be exiting on "Address already in use" while a neighbour's answers it.
if dev_wait_healthy "$HTTP_PORT" "$SERVER_PID"; then
    echo -e "${GREEN}Server is healthy (PID: $SERVER_PID)${NC}"
    echo -e "Log: tail -f $SERVER_LOG"
    if [ -n "$TUNNEL_URL" ]; then
        echo -e "Tunnel: $TUNNEL_URL (PID: $TUNNEL_PID)"
    fi
    exit 0
fi

echo -e "${RED}Server did not come up on $HTTP_PORT. Check: tail -f $SERVER_LOG${NC}"
tail -5 "$SERVER_LOG"
exit 1
