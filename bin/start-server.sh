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

echo -e "${BLUE}=== Pierre MCP Server Startup ===${NC}"
echo -e "Project root: ${PROJECT_ROOT}"

cd "$PROJECT_ROOT"

# Load .envrc
ENVRC_PATH="$PROJECT_ROOT/.envrc"
if [ -f "$ENVRC_PATH" ]; then
    echo -e "${GREEN}Loading environment from: ${ENVRC_PATH}${NC}"
    set -a
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
export HTTP_PORT="${HTTP_PORT:-8081}"

# Kill any existing server and tunnel
if pgrep -f "pierre-mcp-server" > /dev/null; then
    echo -e "Stopping existing server..."
    pkill -f "pierre-mcp-server" 2>/dev/null || true
    sleep 2
fi
pkill -f "cloudflared tunnel" 2>/dev/null || true

# Start Cloudflare tunnel if requested
TUNNEL_URL=""
if [ "$START_TUNNEL" = "true" ]; then
    echo -e "${BLUE}Starting Cloudflare tunnel...${NC}"
    if ! command -v cloudflared &> /dev/null; then
        echo -e "${RED}cloudflared not installed. Run: brew install cloudflare/cloudflare/cloudflared${NC}"
        echo -e "${YELLOW}Skipping tunnel setup${NC}"
    else
        TUNNEL_LOG="$PROJECT_ROOT/logs/tunnel.log"
        cloudflared tunnel --url http://localhost:$HTTP_PORT > "$TUNNEL_LOG" 2>&1 &
        TUNNEL_PID=$!

        # Wait for tunnel URL
        for i in {1..30}; do
            TUNNEL_URL=$(grep -o 'https://[a-z0-9-]*\.trycloudflare\.com' "$TUNNEL_LOG" 2>/dev/null | head -1) || true
            if [ -n "$TUNNEL_URL" ]; then
                break
            fi
            sleep 1
        done

        if [ -n "$TUNNEL_URL" ]; then
            echo -e "${GREEN}Tunnel URL: ${TUNNEL_URL}${NC}"
            export BASE_URL="$TUNNEL_URL"

            # Update mobile .env if it exists
            MOBILE_ENV="$PROJECT_ROOT/frontend-mobile/.env"
            if [ -f "$MOBILE_ENV" ]; then
                sed -i '' "s|^EXPO_PUBLIC_API_URL=.*|EXPO_PUBLIC_API_URL=$TUNNEL_URL|" "$MOBILE_ENV" 2>/dev/null || true
            fi
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

# Start server in background with logging
SERVER_LOG="$PROJECT_ROOT/logs/pierre-server.log"
cargo run --bin pierre-mcp-server > "$SERVER_LOG" 2>&1 &
SERVER_PID=$!

# Wait for server to be healthy
for i in {1..30}; do
    if curl -s -f "http://localhost:$HTTP_PORT/health" > /dev/null 2>&1; then
        echo -e "${GREEN}Server is healthy (PID: $SERVER_PID)${NC}"
        echo -e "Log: tail -f $SERVER_LOG"
        if [ -n "$TUNNEL_URL" ]; then
            echo -e "Tunnel: $TUNNEL_URL (PID: $TUNNEL_PID)"
        fi
        exit 0
    fi
    sleep 1
done

echo -e "${RED}Server failed to start within 30s. Check: tail -f $SERVER_LOG${NC}"
exit 1
