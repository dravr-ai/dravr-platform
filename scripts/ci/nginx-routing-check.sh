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
#   - every access-log line is JSON whose severity Cloud Run can store: INFO
#     for a clean proxy, ERROR for a response nginx could not get from the
#     backend, WARNING for one served only after nginx abandoned a first
#     upstream address. The IPv6 failures above served no 5xx and were stored
#     with no severity, so nothing alerted for a week; the WARNING line is the
#     signal that was missing. A query string never reaches the log.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
NGINX_CONF="$REPO_ROOT/docker/images/frontend/nginx.conf"
HEADERS_CONF="$REPO_ROOT/docker/images/frontend/security-headers.conf"
IMAGE="nginx:alpine"
NET="nginx-routing-check-net"
STUB="nginx-routing-check-stub"
DEAD_STUB="nginx-routing-check-dead-stub"
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
    "GET /api/user/export"                            # the athlete's data export (its own location)
    "GET /mcp/tools"                                  # MCP tool discovery
    "GET /.well-known/oauth-protected-resource"       # MCP client OAuth discovery
    "GET /.well-known/oauth-protected-resource/mcp"   # RFC 9728 path-suffixed form (claude.ai)
    "GET /.well-known/oauth-authorization-server"
    "GET /.well-known/oauth-protected-resource/a2a"   # the document an A2A 401 names
    "GET /.well-known/agent-card.json"                # A2A
    "POST /a2a/jsonrpc"                               # A2A JSON-RPC binding (SSE, its own location)
    "POST /a2a/message:send"                          # A2A HTTP+JSON binding
    "POST /a2a/message:stream"                        # SSE, its own location
    "GET /a2a/tasks/t1"
    "GET /a2a/tasks/t1:subscribe"                     # SSE, its own location
    "POST /oauth2/register"                           # RFC 7591 dynamic client registration
    "POST /oauth2/token"                              # OAuth token endpoint (oauth2 family)
    "GET /oauth2/authorize"
    "GET /oauth2/login"                               # hosted sign-in page the web app opens (carnet#787)
    "POST /oauth2/login"
    "GET /api/health"
    "POST /oauth/token"
    "GET /admin/x"
    "GET /a2a/x"
    "GET /providers/x"
    "GET /providers/signin?token=t"                   # hosted credential sign-in a reconnect link opens (carnet#1207)
    "GET /providers/signin/success"
    "GET /providers/sciotte/login?token=t"            # the same page at the path links sent before carnet#1207 carry
    "GET /r/abc"
    "POST /tenants/switch"                            # the one tenants route
    "GET /ws"                                         # WebSocket proxy block
    "POST /webhooks/garmin/t/activities"              # Garmin push block (larger body cap)
)

# Paths that MUST be served by the SPA. Guards against an over-broad
# alternation swallowing app routes (e.g. a bare `mcp` matching /mcpanything).
SPA_PATHS=(
    "GET /"
    "GET /dashboard"
    "GET /dashboard/status"                           # the backend mounts nothing under /dashboard
    "GET /tenants"                                    # nor at the bare /tenants
    "GET /mcpanything"
    "GET /index.html"
    "GET /auth/callback"                              # first-party sign-in return leg (carnet#787)
    "GET /auth/callback?code=c&state=s"
)

cleanup() {
    docker rm -f "$STUB" "$DEAD_STUB" "$PROXY" >/dev/null 2>&1 || true
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
mkdir -p "$WORK/www/assets"
echo 'export const ASSET_SERVED = 1;' > "$WORK/www/assets/present-Ab12Cd34.js"
# The stub answers every path, except one it drops without a response (444),
# which makes nginx log an upstream error for the log check at the end. It
# takes a body of any size, so a body cap the check meets is the proxy's. The
# body names the host the request was forwarded for and its Authorization, so
# the discovery cache check can tell whose answer a cached response is.
printf 'server {\n listen 80;\n client_max_body_size 0;\n location = %s { return 444; }\n location ~ ^/webhooks/garmin/[^/]+/upstream-fails$ { return 444; }\n location / { add_header Content-Type text/plain; return 200 "BACKEND-REACHED $request_method $uri host=$http_x_forwarded_host auth=$http_authorization\\n"; }\n}\n' \
    "$UPSTREAM_FAILS_PATH" > "$WORK/stub.conf"
# A second address for the stub's name that drops every request, joined at the
# end so nginx has a first address to abandon (the access-log retry check).
printf 'server {\n listen 80;\n location / { return 444; }\n}\n' > "$WORK/dead-stub.conf"
QUERY_SECRET="routing-check-code-5f1e"
# Garmin's push URL carries the webhook secret as a path segment.
PATH_SECRET="routing-check-garmin-token-9c2a"

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

docker run -d --name "$STUB" --network "$NET" --network-alias "$STUB" \
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

# MCP requests with no credential: on mcp.dravr.ai/mcp nginx answers any
# request without `Authorization: Bearer <token>` with the backend's 401 and
# challenge, so the API is never woken. Anything shaped like a bearer (valid
# or not, any case of "Bearer"), a CORS preflight, another host and another
# /mcp path still reach the backend.
echo "-- MCP requests with no credential --"
MCPBEAT_UA='mcpbeat/0.1 (+https://mcpbeat.com/bot/; liveness check)'
probe_401() { # <label> <method> <curl args...>
    local label="$1" method="$2" status
    shift 2
    status="$(curl -s -X "$method" "$@" -D "$WORK/probe.headers" -o "$WORK/probe.body" \
        -w '%{http_code}' -H 'Host: mcp.dravr.ai' "http://localhost:$PORT/mcp" --max-time 10 || true)"
    # The location declares its own add_header, so the security headers ride
    # only on its include of security-headers.conf.
    if [ "$status" = "401" ] \
        && grep -qF '"code":-32001' "$WORK/probe.body" \
        && ! grep -qF 'BACKEND-REACHED' "$WORK/probe.body" \
        && grep -qiF 'www-authenticate: Bearer resource_metadata="https://mcp.dravr.ai/.well-known/oauth-protected-resource/mcp", scope="fitness:read profile:read"' "$WORK/probe.headers" \
        && grep -qi '^content-type: application/json' "$WORK/probe.headers" \
        && grep -qi '^x-content-type-options: nosniff' "$WORK/probe.headers"; then
        printf '  ok   %-6s %-45s -> nginx 401 (%s)\n' "$method" /mcp "$label"
    else
        printf '  FAIL %-6s %-45s -> %s (expected nginx 401 with the challenge and security headers, %s)\n' "$method" /mcp "$status" "$label"
        failures=$((failures + 1))
    fi
}
probe_backend() { # <label> <method> <path> <curl args...>
    local label="$1" method="$2" path="$3" body
    shift 3
    body="$(curl -s -X "$method" "$@" "http://localhost:$PORT$path" --max-time 10 | head -1)"
    if [[ "$body" == BACKEND-REACHED* ]]; then
        printf '  ok   %-6s %-45s -> backend (%s)\n' "$method" "$path" "$label"
    else
        printf '  FAIL %-6s %-45s -> nginx (expected backend, %s)\n' "$method" "$path" "$label"
        failures=$((failures + 1))
    fi
}
probe_401 "a prober, no header" POST -A "$MCPBEAT_UA"
probe_401 "a node client, no header" POST -A 'node'
probe_401 "GET, no header" GET -A 'node'
probe_401 "an empty bearer" POST -H 'Authorization: Bearer '
probe_401 "another scheme" POST -H 'Authorization: Basic dXNlcjpwYXNz'
probe_401 "a key with no scheme" POST -H 'Authorization: pk_live_abc'
probe_401 "a scheme with no space" POST -H 'Authorization: Bearerx'
probe_backend "a bearer" POST /mcp -H 'Host: mcp.dravr.ai' -A "$MCPBEAT_UA" -H 'Authorization: Bearer x'
probe_backend "a lowercase bearer" POST /mcp -H 'Host: mcp.dravr.ai' -H 'Authorization: bearer x'
probe_backend "a bearer after several spaces" POST /mcp -H 'Host: mcp.dravr.ai' -H 'Authorization: Bearer   x'
probe_backend "a CORS preflight" OPTIONS /mcp -H 'Host: mcp.dravr.ai' -H 'Origin: https://example.com' -H 'Access-Control-Request-Method: POST'
probe_backend "another host, no header" POST /mcp -H 'Host: app.dravr.ai'
probe_backend "another /mcp path, no header" GET /mcp/tools -H 'Host: mcp.dravr.ai'
# The answer's location is internal: a client naming it gets nginx's 404.
internal_status="$(curl -s -o /dev/null -w '%{http_code}' -H 'Host: mcp.dravr.ai' \
    "http://localhost:$PORT/__mcp_unauthenticated_401" --max-time 10 || true)"
if [ "$internal_status" = "404" ]; then
    printf '  ok   %-6s %-45s -> 404 (internal)\n' GET /__mcp_unauthenticated_401
else
    printf '  FAIL %-6s %-45s -> %s (expected 404, the location is internal)\n' GET /__mcp_unauthenticated_401 "$internal_status"
    failures=$((failures + 1))
fi

# Paths under /mcp/ the backend does not serve get nginx's 404 and never reach
# it; /mcp/tools still does (checked above). The backend's own routes under
# /mcp/ must stay exactly /mcp/tools, or this rule would hide a new one.
echo "-- unknown paths under /mcp/ --"
for spec in "GET /mcp/owners.json" "POST /mcp/messages" "GET /mcp/.well-known/glama.json"; do
    method="${spec%% *}" path="${spec#* }"
    status="$(curl -s -X "$method" -o "$WORK/sub.body" -w '%{http_code}' -H 'Host: mcp.dravr.ai' \
        -H 'Authorization: Bearer x' "http://localhost:$PORT$path" --max-time 10 || true)"
    if [ "$status" = "404" ] && ! grep -qF 'BACKEND-REACHED' "$WORK/sub.body"; then
        printf '  ok   %-6s %-45s -> nginx 404\n' "$method" "$path"
    else
        printf '  FAIL %-6s %-45s -> %s (expected nginx 404)\n' "$method" "$path" "$status"
        failures=$((failures + 1))
    fi
done
# The scanner's exit status is checked apart from the filter, so a scan that
# fails, or finds no /mcp/ route at all, is a FAIL rather than a pass or a
# silent exit under pipefail.
if ! backend_routes="$(python3 "$REPO_ROOT/scripts/ci/backend-routes.py" routes "$REPO_ROOT")"; then
    echo "  FAIL backend-routes.py could not list the backend's routes"
    failures=$((failures + 1))
else
    mcp_subroutes="$(cut -f1 <<<"$backend_routes" | { grep '^/mcp/' || true; } | sort -u | tr '\n' ' ')"
    if [ "$mcp_subroutes" = "/mcp/tools " ]; then
        echo "  ok   the backend serves only /mcp/tools under /mcp/"
    else
        echo "  FAIL the backend serves '${mcp_subroutes}' under /mcp/; nginx 404s every path there but /mcp/tools (\$mcp_unknown_subpath in nginx.conf)"
        failures=$((failures + 1))
    fi
fi

# Discovery documents. The stub's body names the forwarded host and the
# Authorization it received, so a response nginx served from another request's
# entry shows here: the cache is per host, a request with Authorization is
# answered by the backend and never stored, and the second identical request
# is a cache HIT (asserted from the access log below, as is jwks.json never
# being cached).
echo "-- discovery cache --"
discovery_get() { # <host> <path> [curl args...]
    local host="$1" path="$2"
    shift 2
    curl -s -H "Host: $host" "$@" "http://localhost:$PORT$path" --max-time 10 | head -1 || true
}
expect_body() { # <label> <body> <substring>
    if [[ "$2" == BACKEND-REACHED*"$3"* ]]; then
        echo "  ok   $1"
    else
        echo "  FAIL $1 (got: ${2:0:120})"
        failures=$((failures + 1))
    fi
}
PRM=/.well-known/oauth-protected-resource/mcp
# Twice on one host: a MISS, then the HIT the access log is checked for.
discovery_get mcp.dravr.ai "$PRM" >/dev/null
discovery_get mcp.dravr.ai "$PRM" >/dev/null
expect_body "each host has its own discovery cache entry" "$(discovery_get app.dravr.ai "$PRM")" "host=app.dravr.ai "
# An entry with no Authorization exists first, so a request with one that
# reads it, or whose answer replaces it, shows in the body.
CARD=/.well-known/agent-card.json
DISCOVERY_BEARER="Bearer routing-check-discovery"
discovery_get mcp.dravr.ai "$CARD" >/dev/null
expect_body "a request with Authorization bypasses the cached entry" \
    "$(discovery_get mcp.dravr.ai "$CARD" -H "Authorization: $DISCOVERY_BEARER")" "auth=$DISCOVERY_BEARER"
card_after="$(discovery_get mcp.dravr.ai "$CARD")"
if [[ "$card_after" == BACKEND-REACHED* && "$card_after" != *"$DISCOVERY_BEARER"* ]]; then
    echo "  ok   the answer to a request with Authorization is never stored"
else
    echo "  FAIL a request with no Authorization was served the answer to one with it (got: ${card_after:0:120})"
    failures=$((failures + 1))
fi
for uncached_path in /.well-known/jwks.json /.well-known/did.json /.well-known/dravr-checkpoint; do
    discovery_get mcp.dravr.ai "$uncached_path" >/dev/null
    discovery_get mcp.dravr.ai "$uncached_path" >/dev/null
done

# Hashed assets: a chunk the image holds is served as JavaScript, and one it
# lacks is a 404 — never the SPA shell, which a module import of a chunk from
# the previous release would otherwise receive as text/html with a 200.
echo "-- hashed assets --"
asset_present="$(curl -s -o /dev/null -w '%{http_code} %{content_type}' "http://localhost:$PORT/assets/present-Ab12Cd34.js" --max-time 10 || true)"
if [[ "$asset_present" == "200 application/javascript"* ]]; then
    printf '  ok   %-6s %-45s -> 200 javascript\n' GET /assets/present-Ab12Cd34.js
else
    printf '  FAIL %-6s %-45s -> %s (expected 200 application/javascript)\n' GET /assets/present-Ab12Cd34.js "$asset_present"
    failures=$((failures + 1))
fi
asset_missing_status="$(curl -s -o "$WORK/asset-missing.body" -w '%{http_code}' "http://localhost:$PORT/assets/missing-Zz99Yy88.js" --max-time 10 || true)"
if [ "$asset_missing_status" = "404" ] && ! grep -qF 'SPA-INDEX' "$WORK/asset-missing.body"; then
    printf '  ok   %-6s %-45s -> 404\n' GET /assets/missing-Zz99Yy88.js
else
    printf '  FAIL %-6s %-45s -> %s (expected 404, not the SPA shell)\n' GET /assets/missing-Zz99Yy88.js "$asset_missing_status"
    failures=$((failures + 1))
fi

# The SPA frames /__/auth/iframe to read a Google sign-in result back, so the
# Firebase proxy must allow same-origin framing. The upstream does not resolve
# here; `always` headers ride on the error response all the same.
echo "-- Firebase auth pages are frameable by this origin --"
curl -s -D "$WORK/firebase-iframe.headers" -o /dev/null "http://localhost:$PORT/__/auth/iframe" --max-time 15 || true
if grep -qi '^x-frame-options: SAMEORIGIN' "$WORK/firebase-iframe.headers" \
    && grep -qi "^content-security-policy: frame-ancestors 'self'" "$WORK/firebase-iframe.headers" \
    && ! grep -qi "frame-ancestors 'none'" "$WORK/firebase-iframe.headers"; then
    printf '  ok   %-6s %-45s -> framing SAMEORIGIN\n' GET /__/auth/iframe
else
    printf '  FAIL %-6s %-45s -> framing headers wrong (expected SAMEORIGIN and frame-ancestors '"'"'self'"'"' only)\n' GET /__/auth/iframe
    failures=$((failures + 1))
fi

# A Garmin activityDetails push is far past nginx's 1 MB default body cap; the
# Garmin block raises it, so a 4 MB push must still reach the backend.
echo "-- large Garmin push --"
head -c 4194304 /dev/zero | tr '\0' 'x' > "$WORK/large-push.json"
large_body="$(curl -s -X POST --data-binary "@$WORK/large-push.json" \
    "http://localhost:$PORT/webhooks/garmin/t/activityDetails" --max-time 20 | head -1)"
if [[ "$large_body" == BACKEND-REACHED* ]]; then
    printf '  ok   %-6s %-45s -> backend (4 MB body)\n' POST /webhooks/garmin/t/activityDetails
else
    printf '  FAIL %-6s %-45s -> refused (expected backend with a 4 MB body)\n' POST /webhooks/garmin/t/activityDetails
    failures=$((failures + 1))
fi

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

# ---------------------------------------------------------------------------
# The access log is JSON with a severity Cloud Run stores.
# ---------------------------------------------------------------------------

echo ""
echo "-- access log severity --"

curl -s -o /dev/null "http://localhost:$PORT/oauth2/authorize?code=$QUERY_SECRET&state=$QUERY_SECRET" --max-time 10 || true
curl -s -o /dev/null -X POST -d '{}' "http://localhost:$PORT/webhooks/garmin/$PATH_SECRET/sleeps" --max-time 10 || true
# A Garmin push the upstream drops: nginx's error log would write its raw
# request line, token and all, were the Garmin location not to silence it.
curl -s -o /dev/null -X POST -d '{}' "http://localhost:$PORT/webhooks/garmin/$PATH_SECRET/upstream-fails" --max-time 10 || true

# Join a dead address under the stub's name, then ask until nginx has dialled
# it first and moved on. Docker's DNS rotates the two records and the resolver
# re-asks every 10s, so the dead one leads within a few requests.
docker run -d --name "$DEAD_STUB" --network "$NET" --network-alias "$STUB" \
    -v "$WORK/dead-stub.conf:/etc/nginx/conf.d/default.conf:ro" "$IMAGE" >/dev/null
retry_served=1
for _ in $(seq 1 40); do
    body="$(curl -s "http://localhost:$PORT/api/health?retry" --max-time 5 || true)"
    [[ "$body" == BACKEND-REACHED* ]] || retry_served=0
    if docker logs "$PROXY" 2>/dev/null | grep -F '"path":"/api/health"' | grep -qF '"severity":"WARNING"'; then
        break
    fi
    sleep 0.5
done

if ! access_logs="$(docker logs "$PROXY" 2>/dev/null)"; then
    echo "  FAIL could not read the nginx container's access log"
    failures=$((failures + 1))
else
    access_json="$(grep '^{' <<<"$access_logs" || true)"
    if [ -z "$access_json" ]; then
        echo "  FAIL no JSON access-log line on stdout; Cloud Run stores plain text with no severity"
        printf '%s\n' "$access_logs" | tail -5 | sed 's/^/    /'
        failures=$((failures + 1))
    elif ! bad="$(jq -c 'select((.severity|type) != "string" or (.status|type) != "number")' <<<"$access_json" 2>&1)" || [ -n "$bad" ]; then
        echo "  FAIL an access-log line is not JSON with a severity and a numeric status:"
        printf '%s\n' "$bad" | head -3 | sed 's/^/    /'
        failures=$((failures + 1))
    else
        expect_line() { # <label> <jq selector>
            if jq -e "select($2)" <<<"$access_json" >/dev/null 2>&1 && [ -n "$(jq -c "select($2)" <<<"$access_json")" ]; then
                echo "  ok   $1"
            else
                echo "  FAIL $1"
                failures=$((failures + 1))
            fi
        }
        expect_line "a proxied 200 is INFO" '.path == "/api/health" and .status == 200 and .severity == "INFO"'
        # An empty upstream is the proof that a request with no credential
        # never dialled the API.
        expect_line "a repeated discovery fetch is a cache HIT with no upstream" '.path == "/.well-known/oauth-protected-resource/mcp" and .cache == "HIT" and (.upstream == "" or .upstream == "-")'
        # Every line of a key or transparency document reached the backend
        # with no cache status: each has its own uncached location (the
        # signing keys, the checkpoint key's did:web, the newest checkpoint),
        # so the repeated fetch above would be a HIT were it cached, and fewer
        # than two lines would mean the check saw nothing.
        for uncached_path in /.well-known/jwks.json /.well-known/did.json /.well-known/dravr-checkpoint; do
            path_lines="$(jq -c --arg p "$uncached_path" 'select(.path == $p)' <<<"$access_json")"
            path_direct="$(jq -c 'select((.cache == "" or .cache == "-") and .upstream != "" and .upstream != "-")' <<<"$path_lines" | wc -l | tr -d ' ')"
            path_total="$(printf '%s' "$path_lines" | grep -c '^{' || true)"
            if [ "$path_total" -ge 2 ] && [ "$path_direct" = "$path_total" ]; then
                echo "  ok   $uncached_path always reaches the backend, uncached ($path_direct requests)"
            else
                echo "  FAIL $uncached_path went through the discovery cache ($path_direct of $path_total requests reached the backend uncached)"
                failures=$((failures + 1))
            fi
        done
        expect_line "an MCP request nginx answers is logged as a 401 with no upstream" '.path == "/__mcp_unauthenticated_401" and .status == 401 and (.upstream == "" or .upstream == "-")'
        expect_line "an upstream failure is ERROR" ".path == \"$UPSTREAM_FAILS_PATH\" and .status == 502 and .severity == \"ERROR\""
        expect_line "a 200 served after abandoning an address is WARNING" '.path == "/api/health" and .status == 200 and .severity == "WARNING" and (.upstream | contains(", "))'
        if [ "$retry_served" -ne 1 ]; then
            echo "  FAIL a request failed while one upstream address was dead; nginx did not retry the next"
            failures=$((failures + 1))
        fi
        if grep -qF -- "$QUERY_SECRET" <<<"$access_logs"; then
            echo "  FAIL a query string reached the access log (OAuth codes travel there)"
            failures=$((failures + 1))
        else
            echo "  ok   the query string stays out of the log"
        fi
        # stdout and stderr: the access log is the first, the error log the
        # second.
        if ! all_logs="$(docker logs "$PROXY" 2>&1)"; then
            echo "  FAIL could not read the nginx container's error log"
            failures=$((failures + 1))
        elif grep -qF -- "$PATH_SECRET" <<<"$all_logs"; then
            echo "  FAIL the Garmin push token reached nginx's logs (access or error log):"
            grep -F -- "$PATH_SECRET" <<<"$all_logs" | head -3 | sed 's/^/    /'
            failures=$((failures + 1))
        else
            echo "  ok   the Garmin push token stays out of the access and error logs"
        fi
        expect_line "a Garmin push is logged with its token redacted" '.path == "/webhooks/garmin/[redacted]/sleeps"'
        # The success marker for the error-log check above: the dropped push
        # did reach the upstream and fail, so an error line was due.
        expect_line "a Garmin push the upstream drops is a 502 with its token redacted" '.path == "/webhooks/garmin/[redacted]/upstream-fails" and .status == 502'
    fi
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
