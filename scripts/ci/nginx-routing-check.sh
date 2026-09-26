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
#
# Two more properties of the config are checked here, because nothing else runs
# nginx:
#   - nginx starts before its upstream exists. A static `proxy_pass
#     ${BACKEND_URL}` anywhere resolves the backend host while nginx loads, and
#     the variable proxy_pass of the backend block then matches that upstream
#     by name and dials its AAAA addresses first instead of asking the ipv6=off
#     resolver (1,057 "connect() to [2600:...]:443 failed (101: Network
#     unreachable)" lines on dev in the week to 2026-09-26). With the upstream
#     absent, that config fails at load, so starting the stub second catches it.
#   - an upstream failure never writes a client-supplied address to the logs.
#     nginx's error log names $remote_addr ("client: ...") on every upstream
#     error, so any directive that rewrites $remote_addr from a header (realip)
#     would put athletes' addresses in WARN/ERROR lines.

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
# A backend path the stub drops, and the client address a request to it names
# in every forwarding header (RFC 5737 documentation range).
UPSTREAM_FAILS_PATH="/api/routing-check-upstream-fails"
CLIENT_SUPPLIED_ADDRESS="198.51.100.77"

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
    "GET /dashboard/status"                           # 7-route family, previously SPA-shelled
    "GET /fitness/config"
    "GET /health-data/recovery"
    "GET /tenants"                                    # bare path: list
    "POST /tenants"                                   # bare path: create
    "GET /ws"                                         # WebSocket proxy block
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
# The stub answers every path, except one it drops without a response (444),
# which makes nginx log an upstream error for the log check at the end.
printf 'server {\n listen 80;\n location = %s { return 444; }\n location / { add_header Content-Type text/plain; return 200 "BACKEND-REACHED $request_method $uri\\n"; }\n}\n' \
    "$UPSTREAM_FAILS_PATH" > "$WORK/stub.conf"

docker network create "$NET" >/dev/null 2>&1 || true

# nginx first, while the stub's name resolves to nothing (see the header).
docker run -d --name "$PROXY" --network "$NET" -p "$PORT:8080" \
    -v "$WORK/nginx.conf:/etc/nginx/nginx.conf:ro" \
    -v "$HEADERS_CONF:/etc/nginx/security-headers.conf:ro" \
    -v "$WORK/www:/usr/share/nginx/html:ro" "$IMAGE" >/dev/null

ready=0
for _ in $(seq 1 30); do
    if curl -s -o /dev/null "http://localhost:$PORT/" --max-time 2; then
        ready=1
        break
    fi
    # nginx exits at once on a config it cannot load; stop waiting then.
    [ "$(docker inspect -f '{{.State.Running}}' "$PROXY" 2>/dev/null || true)" = "true" ] || break
    sleep 1
done
if [ "$ready" -ne 1 ]; then
    echo "❌ nginx did not start before its upstream existed. A location that"
    echo "   resolves BACKEND_URL while nginx loads (a static proxy_pass) fails"
    echo "   here, and in production makes every backend request dial the AAAA"
    echo "   addresses resolved at startup. Use the variable form:"
    echo "     set \$backend_upstream \${BACKEND_URL}; proxy_pass \$backend_upstream\$request_uri;"
    docker logs "$PROXY" 2>&1 | tail -20 || true
    exit 1
fi

docker run -d --name "$STUB" --network "$NET" \
    -v "$WORK/stub.conf:/etc/nginx/conf.d/default.conf:ro" "$IMAGE" >/dev/null

stub_ready=0
for _ in $(seq 1 30); do
    body="$(curl -s "http://localhost:$PORT/api/health" --max-time 2 || true)"
    if [[ "$body" == BACKEND-REACHED* ]]; then
        stub_ready=1
        break
    fi
    sleep 1
done
if [ "$stub_ready" -ne 1 ]; then
    echo "❌ the stub upstream never answered through nginx"
    docker logs "$PROXY" 2>&1 | tail -20 || true
    exit 1
fi

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
           <(python3 "$REPO_ROOT/scripts/ci/backend-routes.py" prefixes "$REPO_ROOT") \
           > /tmp/route-surface.diff 2>&1; then
    echo "  ok   route surface unchanged"
else
    echo "  FAIL the backend's top-level path prefixes changed:"
    sed 's/^/    /' /tmp/route-surface.diff | head -20
    echo ""
    echo "  A '+' line is a NEW backend prefix. Decide whether nginx must proxy it"
    echo "  (docker/images/frontend/nginx.conf) — an unproxied prefix returns the"
    echo "  SPA shell with a 200, which looks healthy and is not. Then refresh:"
    echo "    python3 scripts/ci/backend-routes.py prefixes . > scripts/ci/backend-route-prefixes.txt"
    failures=$((failures + 1))
fi

# ---------------------------------------------------------------------------
# Upstream errors name nginx's TCP peer, never a client-supplied address.
# ---------------------------------------------------------------------------

echo ""
echo "-- upstream error log --"

curl -s -o /dev/null "http://localhost:$PORT$UPSTREAM_FAILS_PATH" --max-time 10 \
    -H "X-Forwarded-For: $CLIENT_SUPPLIED_ADDRESS" \
    -H "X-Real-IP: $CLIENT_SUPPLIED_ADDRESS" \
    -H "Forwarded: for=$CLIENT_SUPPLIED_ADDRESS" || true

if ! proxy_logs="$(docker logs "$PROXY" 2>&1)"; then
    echo "  FAIL could not read the nginx container's logs"
    failures=$((failures + 1))
elif ! upstream_error="$(grep -F "$UPSTREAM_FAILS_PATH" <<<"$proxy_logs" | grep -F '[error]' | grep -F 'client: ')"; then
    # The success marker: without the error line the leak check below proves
    # nothing, so its absence fails rather than passes.
    echo "  FAIL nginx logged no upstream error for $UPSTREAM_FAILS_PATH; the check did not run"
    printf '%s\n' "$proxy_logs" | tail -10 | sed 's/^/    /'
    failures=$((failures + 1))
elif grep -qF -- "$CLIENT_SUPPLIED_ADDRESS" <<<"$proxy_logs"; then
    echo "  FAIL a client-supplied address reached nginx's logs:"
    grep -F -- "$CLIENT_SUPPLIED_ADDRESS" <<<"$proxy_logs" | head -5 | sed 's/^/    /'
    echo "  Something rewrites \$remote_addr from a request header (realip?). The"
    echo "  error log's client field is that address, at WARN/ERROR, which is PII."
    failures=$((failures + 1))
else
    echo "  ok   upstream error logged without the client-supplied address"
    echo "       ${upstream_error:0:160}"
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
