#!/usr/bin/env bash
# ABOUTME: Runs the real-backend Home sync spec (web) locally, on an isolated Pierre server pointed at the session-reader double
# ABOUTME: Own ports, own SQLite file, own processes: the dev stack on 8081 and the real session-read service it talks to are left alone
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# frontend/e2e-real/home-sync.real.spec.ts needs a Pierre server whose
# DRAVR_SCIOTTE_REMOTE_URL names the session-reader double the spec starts, and whose
# SQLite file it can write a past sync time into. The standard dev stack
# (bin/setup-db-with-seeds-and-oauth-and-start-servers.sh) points the server
# at the real service instead, so this script boots the stack the spec needs
# the way CI's integration workflow does — a fresh database with the seeded
# admin, the server, the SPA proxied to it — runs the spec, and stops
# everything it started, whatever the outcome.
#
# Usage: scripts/e2e-home-sync-local.sh [extra playwright args]
# Ports: E2E_PIERRE_PORT (8095), E2E_VITE_PORT (5185), SCIOTTE_DOUBLE_PORT (8097).
# Server log level: E2E_RUST_LOG (warn); the log path is printed on exit.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PIERRE_PORT="${E2E_PIERRE_PORT:-8095}"
VITE_PORT="${E2E_VITE_PORT:-5185}"
DOUBLE_PORT="${SCIOTTE_DOUBLE_PORT:-8097}"
ADMIN_EMAIL="admin@example.com"
ADMIN_PASSWORD="AdminPassword123"
# A throwaway key for a throwaway database, as CI uses one.
MASTER_KEY="rEFe91l6lqLahoyl9OSzum9dKa40VvV5RYj8bHGNTeo="
# The route request's bound, shortened as CI shortens it; the spec reads it too.
ROUTE_ANSWER_SECS=5
# The model a new conversation records, as CI names one: no chain is
# configured and nothing behind it is called, but opening a coach's thread
# about an athlete, and the CLI that seeds one, need a model to record.
E2E_MODEL="local-e2e-model"

PORTS=("$PIERRE_PORT" "$DOUBLE_PORT" "$VITE_PORT")
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
    stop "$VITE_PID"
    stop "$SERVER_PID"
    wait 2>/dev/null || true
    echo "logs: $WORK/server.log $WORK/vite.log"
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
"$ROOT/target/debug/pierre-cli" user create --email "$ADMIN_EMAIL" --password "$ADMIN_PASSWORD" --force

echo "starting Pierre on $PIERRE_PORT, its session reader at the double on $DOUBLE_PORT"
(
    cd "$ROOT"
    HTTP_PORT="$PIERRE_PORT" \
        PIERRE_RSA_KEY_SIZE=2048 \
        RUST_LOG="${E2E_RUST_LOG:-warn}" \
        STRAVA_CLIENT_ID=test_client_id_local \
        STRAVA_CLIENT_SECRET=test_client_secret_local \
        STRAVA_REDIRECT_URI="http://localhost:$PIERRE_PORT/auth/strava/callback" \
        FRONTEND_URL="http://localhost:$VITE_PORT" \
        DRAVR_SCIOTTE_REMOTE_URL="http://127.0.0.1:$DOUBLE_PORT" \
        PIERRE_HOME_ROUTE_ANSWER_SECS="$ROUTE_ANSWER_SECS" \
        PIERRE_LLM_DEFAULT_MODEL="$E2E_MODEL" \
        exec ./target/debug/pierre-mcp-server
) >"$WORK/server.log" 2>&1 &
SERVER_PID=$!
wait_for "http://127.0.0.1:$PIERRE_PORT/health" "Pierre"

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
