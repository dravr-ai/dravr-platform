#!/usr/bin/env bash
# ABOUTME: Verifies the frontend nginx routes backend paths to the backend, not the SPA
# ABOUTME: Runs the real nginx config against a stub upstream; catches proxy-alternation drift
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists
# ---------------
# Nothing else in CI traverses nginx. The SDK and MCP-compliance suites hit the
# backend directly on localhost, and Playwright hits the vite dev server with
# API routes mocked — so `docker/images/frontend/nginx.conf` was built by CI but
# never exercised. `/mcp` was missing from its proxy alternation for months:
# `POST /mcp` returned nginx's 405 and `GET /mcp/tools` returned index.html with
# a healthy-looking 200, while the SPA told users to point Claude Desktop and
# ChatGPT at exactly that URL.
#
# A status-code check would NOT have caught it — the SPA fallback answers 200.
# This asserts the response came from the upstream, which is the only signal
# that distinguishes "proxied" from "served the app shell".

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
NGINX_CONF="$REPO_ROOT/docker/images/frontend/nginx.conf"
HEADERS_CONF="$REPO_ROOT/docker/images/frontend/security-headers.conf"
IMAGE="nginx:alpine"
NET="nginx-routing-check-net"
STUB="nginx-routing-check-stub"
PROXY="nginx-routing-check-proxy"
PORT="${NGINX_ROUTING_CHECK_PORT:-8097}"
WORK="$(mktemp -d)"

# Paths that MUST reach the backend. Adding a backend route to nginx.conf
# without adding it here leaves it uncovered; adding it here without wiring
# nginx fails this check.
BACKEND_PATHS=(
    "POST /mcp"                                       # MCP JSON-RPC (the regression)
    "GET /mcp/tools"                                  # MCP tool discovery
    "GET /.well-known/oauth-protected-resource"       # MCP client OAuth discovery
    "GET /.well-known/oauth-authorization-server"
    "GET /.well-known/agent-card.json"                # A2A
    "POST /oauth2/register"                           # RFC 7591 dynamic client registration
    "POST /oauth2/token"                              # OAuth token endpoint (oauth2 family)
    "GET /oauth2/authorize"
    "GET /api/health"
    "POST /oauth/token"
    "GET /admin/x"
    "GET /a2a/x"
    "GET /providers/x"
    "GET /r/abc"
)

# Paths that MUST be served by the SPA. Guards against an over-broad
# alternation swallowing app routes (e.g. a bare `mcp` matching /mcpanything).
SPA_PATHS=(
    "GET /"
    "GET /dashboard"
    "GET /mcpanything"
    "GET /index.html"
)

cleanup() {
    docker rm -f "$STUB" "$PROXY" >/dev/null 2>&1 || true
    docker network rm "$NET" >/dev/null 2>&1 || true
    rm -rf "$WORK" || true
}
trap cleanup EXIT

echo "==== nginx routing check ===="

# Substitute the template vars the Docker entrypoint would normally fill, and
# swap the public resolver for Docker's embedded DNS so container names resolve.
sed -e 's|${BACKEND_URL}|http://'"$STUB"':80|g' \
    -e 's|${FIREBASE_PROJECT_ID}|routing-check|g' \
    -e 's|resolver 8.8.8.8 ipv6=off valid=300s;|resolver 127.0.0.11 ipv6=off valid=10s;|g' \
    "$NGINX_CONF" > "$WORK/nginx.conf"

mkdir -p "$WORK/www"
echo '<!doctype html><html>SPA-INDEX</html>' > "$WORK/www/index.html"
printf 'server {\n listen 80;\n location / { add_header Content-Type text/plain; return 200 "BACKEND-REACHED $request_method $uri\\n"; }\n}\n' \
    > "$WORK/stub.conf"

docker network create "$NET" >/dev/null 2>&1 || true
docker run -d --name "$STUB" --network "$NET" \
    -v "$WORK/stub.conf:/etc/nginx/conf.d/default.conf:ro" "$IMAGE" >/dev/null
docker run -d --name "$PROXY" --network "$NET" -p "$PORT:8080" \
    -v "$WORK/nginx.conf:/etc/nginx/nginx.conf:ro" \
    -v "$HEADERS_CONF:/etc/nginx/security-headers.conf:ro" \
    -v "$WORK/www:/usr/share/nginx/html:ro" "$IMAGE" >/dev/null

for _ in $(seq 1 30); do
    curl -s -o /dev/null "http://localhost:$PORT/" --max-time 2 && break
    sleep 1
done

failures=0

check() {
    local spec="$1" expect="$2"
    local method="${spec%% *}" path="${spec#* }" body
    body="$(curl -s -X "$method" "http://localhost:$PORT$path" --max-time 10 | head -1)"

    case "$expect" in
        backend)
            if [[ "$body" == BACKEND-REACHED* ]]; then
                printf '  ok   %-6s %-45s -> backend\n' "$method" "$path"
            else
                printf '  FAIL %-6s %-45s -> SPA/nginx (expected backend)\n' "$method" "$path"
                failures=$((failures + 1))
            fi
            ;;
        spa)
            if [[ "$body" == BACKEND-REACHED* ]]; then
                printf '  FAIL %-6s %-45s -> backend (expected SPA)\n' "$method" "$path"
                failures=$((failures + 1))
            else
                printf '  ok   %-6s %-45s -> SPA\n' "$method" "$path"
            fi
            ;;
    esac
}

echo "-- must reach the backend --"
for spec in "${BACKEND_PATHS[@]}"; do check "$spec" backend; done
echo "-- must be served by the SPA --"
for spec in "${SPA_PATHS[@]}"; do check "$spec" spa; done

# ---------------------------------------------------------------------------
# Route-surface snapshot.
#
# The path checks above only cover paths someone remembered to list. This
# catches the case that actually keeps happening: a new backend route family
# lands, nobody adds it to nginx, and it silently serves index.html with a 200.
# Any change to the extracted prefix set fails here and forces a decision.
# ---------------------------------------------------------------------------

echo ""
echo "-- backend route-surface snapshot --"

SNAPSHOT="$REPO_ROOT/scripts/ci/backend-route-prefixes.txt"
if diff -u <(awk '!/^#/ && NF {print $1}' "$SNAPSHOT") \
           <(python3 "$REPO_ROOT/scripts/ci/backend-route-prefixes.py" "$REPO_ROOT") \
           > /tmp/route-surface.diff 2>&1; then
    echo "  ok   route surface unchanged"
else
    echo "  FAIL the backend's top-level path prefixes changed:"
    sed 's/^/    /' /tmp/route-surface.diff | head -20
    echo ""
    echo "  A '+' line is a NEW backend prefix. Decide whether nginx must proxy it"
    echo "  (docker/images/frontend/nginx.conf) — an unproxied prefix returns the"
    echo "  SPA shell with a 200, which looks healthy and is not. Then refresh:"
    echo "    python3 scripts/ci/backend-route-prefixes.py . > scripts/ci/backend-route-prefixes.txt"
    failures=$((failures + 1))
fi

if [ "$failures" -gt 0 ]; then
    echo ""
    echo "❌ $failures routing failure(s)."
    echo "   A backend path served by the SPA usually means it is missing from the"
    echo "   proxy alternation in docker/images/frontend/nginx.conf. Note that the"
    echo "   alternation group requires a trailing slash, so a bare path like /mcp"
    echo "   needs its own alternative. Keep the vite dev proxy and the PWA"
    echo "   navigateFallbackDenylist in frontend/vite.config.ts in sync."
    exit 1
fi

echo ""
echo "✅ nginx routing check passed (${#BACKEND_PATHS[@]} backend, ${#SPA_PATHS[@]} SPA)"
