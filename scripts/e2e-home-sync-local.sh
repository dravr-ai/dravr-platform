#!/usr/bin/env bash
# ABOUTME: Runs the real-backend Home sync spec (web) or Maestro flow (--mobile) locally, on an isolated Pierre server pointed at the scraper double
# ABOUTME: Own ports, own SQLite file, own processes: the dev stack on 8081 and the real scraper it talks to are left alone
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# frontend/e2e-real/home-sync.real.spec.ts needs a Pierre server whose
# DRAVR_SCIOTTE_REMOTE_URL names the scraper double the spec starts, and whose
# SQLite file it can write a past sync time into. The standard dev stack
# (bin/setup-db-with-seeds-and-oauth-and-start-servers.sh) points the server
# at the real scraper instead, so this script boots the stack the spec needs
# the way CI's integration workflow does — a fresh database with the seeded
# admin, the server, the SPA proxied to it — runs the spec, and stops
# everything it started, whatever the outcome.
#
# With --mobile it runs the mobile half instead,
# frontend-mobile/.maestro/home-sync/, on the booted simulator or emulator:
# the seeded Maestro athlete, the double behind its control API
# (frontend/e2e-real/home-sync-control.ts), and Metro serving Expo Go on 8082
# with the app pointed at the isolated server.
#
# Usage: scripts/e2e-home-sync-local.sh [--mobile] [extra playwright or maestro args]
# Ports: E2E_PIERRE_PORT (8095), E2E_VITE_PORT (5185), SCIOTTE_DOUBLE_PORT (8097),
#        SCIOTTE_CONTROL_PORT (8098, --mobile), Metro 8082 (--mobile, fixed by the flows' deep link).
# Server log level: E2E_RUST_LOG (warn); the log path is printed on exit.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MOBILE=false
if [ "${1:-}" = "--mobile" ]; then
    MOBILE=true
    shift
fi
CONTROL_PORT="${SCIOTTE_CONTROL_PORT:-8098}"
METRO_PORT=8082
MOBILE_EMAIL="mobiletest@pierre.dev"
MOBILE_PASSWORD="MobileTest1234"
PIERRE_PORT="${E2E_PIERRE_PORT:-8095}"
VITE_PORT="${E2E_VITE_PORT:-5185}"
DOUBLE_PORT="${SCIOTTE_DOUBLE_PORT:-8097}"
ADMIN_EMAIL="admin@example.com"
ADMIN_PASSWORD="AdminPassword123"
# A throwaway key for a throwaway database, as CI uses one.
MASTER_KEY="rEFe91l6lqLahoyl9OSzum9dKa40VvV5RYj8bHGNTeo="
# The route request's bound, shortened as CI shortens it; the spec reads it too.
ROUTE_ANSWER_SECS=5

PORTS=("$PIERRE_PORT" "$DOUBLE_PORT")
if [ "$MOBILE" = true ]; then
    PORTS+=("$CONTROL_PORT" "$METRO_PORT")
else
    PORTS+=("$VITE_PORT")
fi
for port in "${PORTS[@]}"; do
    if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
        echo "port $port is in use; set E2E_PIERRE_PORT / E2E_VITE_PORT / SCIOTTE_DOUBLE_PORT to free ones" >&2
        exit 1
    fi
done

WORK="$(mktemp -d "${TMPDIR:-/tmp}/e2e-home-sync.XXXXXX")"
DATABASE_PATH="$WORK/e2e-home-sync.db"
SERVER_PID=""
VITE_PID=""
CONTROL_PID=""
METRO_PID=""

# Stop one process this script started: SIGTERM, then SIGKILL once it has had
# the server's own drain budget (a few seconds) to finish. The server drains
# on SIGTERM and leaves the exit to its host, as Cloud Run's grace does.
stop() {
    local pid="$1"
    [ -n "$pid" ] || return 0
    kill "$pid" 2>/dev/null || return 0
    for _ in $(seq 1 15); do
        kill -0 "$pid" 2>/dev/null || return 0
        sleep 1
    done
    kill -9 "$pid" 2>/dev/null || true
}

cleanup() {
    stop "$METRO_PID"
    stop "$CONTROL_PID"
    stop "$VITE_PID"
    stop "$SERVER_PID"
    wait 2>/dev/null || true
    echo "logs: $WORK/server.log $WORK/vite.log $WORK/control.log $WORK/metro.log"
}
trap cleanup EXIT

wait_for() {
    local url="$1" what="$2"
    for _ in $(seq 1 90); do
        if curl -fsS "$url" >/dev/null 2>&1; then
            return 0
        fi
        sleep 2
    done
    echo "$what did not answer at $url" >&2
    return 1
}

echo "building pierre-mcp-server and pierre-cli"
(cd "$ROOT" && cargo build --bin pierre-mcp-server --bin pierre-cli)

export DATABASE_URL="sqlite:$DATABASE_PATH"
export PIERRE_MASTER_ENCRYPTION_KEY="$MASTER_KEY"
"$ROOT/target/debug/pierre-cli" user create --email "$ADMIN_EMAIL" --password "$ADMIN_PASSWORD" --super-admin --force

echo "starting Pierre on $PIERRE_PORT, its scraper at the double on $DOUBLE_PORT"
(
    cd "$ROOT"
    HTTP_PORT="$PIERRE_PORT" \
        PIERRE_RSA_KEY_SIZE=2048 \
        RUST_LOG="${E2E_RUST_LOG:-warn}" \
        STRAVA_CLIENT_ID=test_client_id_local \
        STRAVA_CLIENT_SECRET=test_client_secret_local \
        STRAVA_REDIRECT_URI="http://localhost:$PIERRE_PORT/auth/strava/callback" \
        PIERRE_SCIOTTE_MAX_CONCURRENT=2 \
        PIERRE_SCIOTTE_MAX_QUEUE=8 \
        PIERRE_SCIOTTE_ACQUIRE_TIMEOUT_SECS=10 \
        PIERRE_SCIOTTE_PERMIT_MAX_LIFETIME_SECS=300 \
        PIERRE_SCIOTTE_WATCHDOG_INTERVAL_SECS=15 \
        PIERRE_SCIOTTE_RETRY_AFTER_HINT_SECS=5 \
        PIERRE_SCIOTTE_CLOSED_RETRY_AFTER_SECS=60 \
        DRAVR_SCIOTTE_REMOTE_URL="http://127.0.0.1:$DOUBLE_PORT" \
        PIERRE_HOME_ROUTE_ANSWER_SECS="$ROUTE_ANSWER_SECS" \
        exec ./target/debug/pierre-mcp-server
) >"$WORK/server.log" 2>&1 &
SERVER_PID=$!
wait_for "http://127.0.0.1:$PIERRE_PORT/health" "Pierre"

if [ "$MOBILE" = true ]; then
    "$ROOT/target/debug/pierre-cli" user create --email "$MOBILE_EMAIL" --password "$MOBILE_PASSWORD" --force

    echo "starting the scraper double on $DOUBLE_PORT behind its control API on $CONTROL_PORT"
    (
        cd "$ROOT/frontend"
        PIERRE_URL="http://127.0.0.1:$PIERRE_PORT" \
            SCIOTTE_DOUBLE_PORT="$DOUBLE_PORT" \
            SCIOTTE_CONTROL_PORT="$CONTROL_PORT" \
            E2E_REAL_DATABASE_PATH="$DATABASE_PATH" \
            HOME_SYNC_EMAIL="$MOBILE_EMAIL" \
            HOME_SYNC_PASSWORD="$MOBILE_PASSWORD" \
            exec bun e2e-real/home-sync-control.ts
    ) >"$WORK/control.log" 2>&1 &
    CONTROL_PID=$!
    wait_for "http://127.0.0.1:$CONTROL_PORT/health" "the double's control API"

    echo "starting Metro on $METRO_PORT for Expo Go, the app pointed at Pierre"
    (
        cd "$ROOT/frontend-mobile"
        EXPO_PUBLIC_API_URL="http://127.0.0.1:$PIERRE_PORT" CI=1 \
            exec bunx expo start --go --port "$METRO_PORT"
    ) >"$WORK/metro.log" 2>&1 &
    METRO_PID=$!
    wait_for "http://127.0.0.1:$METRO_PORT/status" "Metro"

    cd "$ROOT/frontend-mobile"
    maestro test --env CONTROL_URL="http://127.0.0.1:$CONTROL_PORT" "$@" \
        .maestro/home-sync/01-sync-failed-retry-lands-rides-and-map.yaml
    exit $?
fi

echo "starting the SPA on $VITE_PORT, proxied to Pierre"
(
    cd "$ROOT/frontend"
    VITE_BACKEND_URL="http://127.0.0.1:$PIERRE_PORT" exec bun run dev -- --port "$VITE_PORT" --strictPort
) >"$WORK/vite.log" 2>&1 &
VITE_PID=$!
wait_for "http://localhost:$VITE_PORT" "the SPA"

cd "$ROOT/frontend"
PIERRE_URL="http://127.0.0.1:$PIERRE_PORT" \
    FRONTEND_URL="http://localhost:$VITE_PORT" \
    SCIOTTE_DOUBLE_PORT="$DOUBLE_PORT" \
    E2E_REAL_DATABASE_PATH="$DATABASE_PATH" \
    PIERRE_HOME_ROUTE_ANSWER_SECS="$ROUTE_ANSWER_SECS" \
    ADMIN_EMAIL="$ADMIN_EMAIL" \
    ADMIN_PASSWORD="$ADMIN_PASSWORD" \
    bunx playwright test --config=playwright.real.config.ts e2e-real/home-sync.real.spec.ts "$@"
