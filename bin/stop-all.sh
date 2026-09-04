#!/bin/bash
# ABOUTME: Stops this checkout's Pierre development services (server, fixture, frontend, mobile, tunnel)
# ABOUTME: Every process it stops is one this checkout recorded; a peer worktree's stack is named and left running

set -e

# Colors
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
. "$SCRIPT_DIR/dev-processes.sh"
. "$SCRIPT_DIR/tunnel-env.sh"

echo -e "${YELLOW}=== Stopping This Checkout's Pierre Services ===${NC}"

# Each name below is a pid file this checkout wrote. Stopping the recorded
# process group takes its children with it, so the esbuild, Metro and
# NativeWind workers need no names of their own — naming them would match
# every other worktree's workers as well.
dev_stop pierre-server "Pierre MCP Server"
dev_stop fixture "Dev Fixture API"
dev_stop sciotte "Sciotte scraper service"
dev_stop vite "Vite dev server"
dev_stop expo "Expo / Metro"
dev_stop expo-build "Native app build"
dev_stop tunnel "Cloudflare tunnel"

# The ports this checkout's own start scripts were pinned to: a caller override
# first, then what .envrc declares, then the default — the order the start
# scripts resolve. Run without direnv, the ambient environment carries neither
# variable, and the bare default is a DIFFERENT checkout's port: a worktree
# pinned to 8091 would reclaim 8081 and leave its own server up.
SERVER_PORT="${HTTP_PORT:-$(tunnel_env_declared_port "$PROJECT_ROOT/.envrc" HTTP_PORT 8081)}"
METRO_PORT="${EXPO_PORT:-$(tunnel_env_declared_port "$PROJECT_ROOT/.envrc" EXPO_PORT 8082)}"

# A stack started before this checkout kept pid files leaves a listener with no
# record. Reclaim it when it resolves to this worktree; name it and leave it
# alone when it belongs to another.
foreign=0
dev_reclaim_port "$SERVER_PORT" "Pierre MCP Server" || foreign=1
dev_reclaim_port 8091 "Sciotte scraper service" || foreign=1
dev_reclaim_port 5173 "Vite dev server" || foreign=1
dev_reclaim_port "$METRO_PORT" "Expo / Metro" || foreign=1
dev_reclaim_port 9555 "Dev Fixture API" || foreign=1
if [ "$foreign" -eq 1 ]; then
    echo -e "${YELLOW}  Ports above belong to another checkout and were left running.${NC}"
fi

# A quick tunnel's hostname stops resolving the moment its process dies, so the
# files that name one are pointed back at the local server here — whether or not
# there was a tunnel left to stop, since the reported outage was a tunnel that
# died on its own. A BASE_URL an operator set by hand is left as it is.
if tunnel_env_reset "$PROJECT_ROOT"; then
    echo "  BASE_URL and EXPO_PUBLIC_API_URL reset to the local server"
    echo "  Run 'direnv allow' and restart Pierre to pick it up"
fi

echo -e "${GREEN}This checkout's services stopped${NC}"
