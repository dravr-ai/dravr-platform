#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Complete development environment setup - database, seeds, OAuth users, all servers
# ABOUTME: One command to go from zero to fully running dev environment with test data

set -eo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# Parse command line args
BUILD_MODE="debug"
TARGET_DIR="debug"
NATIVE_BUILD=false
STREAM_LOGS=false
START_TUNNEL=false
for arg in "$@"; do
    case $arg in
        --release)
            BUILD_MODE="release"
            TARGET_DIR="release"
            shift
            ;;
        --native)
            NATIVE_BUILD=true
            shift
            ;;
        --stream-logs)
            STREAM_LOGS=true
            shift
            ;;
        --tunnel)
            START_TUNNEL=true
            shift
            ;;
    esac
done

# Project paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

# Log directory
LOG_DIR="$PROJECT_ROOT/logs"
mkdir -p "$LOG_DIR"

# Log files
SERVER_LOG="$LOG_DIR/pierre-server.log"
FRONTEND_LOG="$LOG_DIR/frontend.log"
EXPO_LOG="$LOG_DIR/expo.log"

# Ports
SERVER_PORT=8081
FRONTEND_PORT=5173
EXPO_PORT=8082
FIXTURE_PORT=9555  # dev fixture API serving seeded Strava/Garmin activities

# Credentials - use .envrc values or defaults
# These are set after sourcing .envrc below
WEB_TEST_EMAIL="webtest@pierre.dev"
WEB_TEST_PASSWORD="WebTest123!"
MOBILE_TEST_EMAIL="mobiletest@pierre.dev"
MOBILE_TEST_PASSWORD="MobileTest1234"
PHIL_TEST_EMAIL="phil_test@dravr.ai"
JF_TEST_EMAIL="jf_test@dravr.ai"
DRAVR_TEST_PASSWORD='fougeresEtSapin2017#!$'
DEMO_PASSWORD="DemoUser123!"

print_step() {
    echo -e "${GREEN}[$1/$TOTAL_STEPS]${NC} $2"
}

TOTAL_STEPS=9

echo ""
echo -e "${BLUE}============================================================================${NC}"
echo -e "${BLUE}   PIERRE DEVELOPMENT ENVIRONMENT SETUP${NC}"
echo -e "${BLUE}   Database + Seeds + OAuth Users + All Servers${NC}"
echo -e "${BLUE}============================================================================${NC}"
echo ""
echo -e "${CYAN}=== Configuration ===${NC}"
echo -e "  Build mode:      ${YELLOW}$BUILD_MODE${NC}"
echo -e "  Native build:    $( [ "$NATIVE_BUILD" = "true" ] && echo "${GREEN}yes${NC}" || echo "no" )"
echo -e "  Tunnel:          $( [ "$START_TUNNEL" = "true" ] && echo "${GREEN}yes${NC}" || echo "no" )"
echo -e "  Stream logs:     $( [ "$STREAM_LOGS" = "true" ] && echo "${GREEN}yes${NC}" || echo "no" )"
echo ""

# Load environment
if [ ! -f "$PROJECT_ROOT/.envrc" ]; then
    echo -e "${RED}ERROR: .envrc not found. Copy from .envrc.example and configure.${NC}"
    echo -e "${RED}Run: cp .envrc.example .envrc${NC}"
    exit 1
fi
set -a
source "$PROJECT_ROOT/.envrc"
set +a

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
echo ""

# Admin credentials from .envrc (with fallback defaults)
ADMIN_EMAIL="${ADMIN_EMAIL:-admin@example.com}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-AdminPassword123}"

# Step 1: Stop all services
print_step 1 "Stopping existing services..."
"$SCRIPT_DIR/stop-all.sh" 2>/dev/null || {
    # Fallback if stop-all.sh doesn't exist yet
    pkill -f "pierre-mcp-server" 2>/dev/null || true
    pkill -f "dravr-sciotte-server" 2>/dev/null || true
    pkill -f "pierre-dev-fixture" 2>/dev/null || true
    pkill -f "node_modules/.bin/vite" 2>/dev/null || true
    pkill -f "node_modules/@esbuild" 2>/dev/null || true
    pkill -f "expo start" 2>/dev/null || true
    pkill -f "jest-worker/build/workers/processChild" 2>/dev/null || true
    pkill -f "nativewind.*child" 2>/dev/null || true
    pkill -f "cloudflared tunnel" 2>/dev/null || true
    sleep 2
}
echo "    Done"

# Step 2: Reset database
print_step 2 "Resetting database (fresh migrations)..."
DB_PATH="$PROJECT_ROOT/data/users.db"
BACKUP_DIR="$PROJECT_ROOT/data/backups"
mkdir -p "$BACKUP_DIR"

if [ -f "$DB_PATH" ]; then
    BACKUP_NAME="users_$(date +%Y%m%d_%H%M%S).db"
    cp "$DB_PATH" "$BACKUP_DIR/$BACKUP_NAME"
    echo "    Backed up to: $BACKUP_DIR/$BACKUP_NAME"
    rm -f "$DB_PATH" "$DB_PATH-shm" "$DB_PATH-wal"
fi
echo "    Database cleared"

# Step 3: Build binaries
# All seeders are now subcommands under `pierre-cli seed <domain>` — no separate binaries.
print_step 3 "Building server binaries ($BUILD_MODE mode)..."
if [ "$BUILD_MODE" = "release" ]; then
    cargo build --release --bin pierre-mcp-server --bin pierre-cli --bin pierre-dev-fixture 2>&1 | tail -3
else
    cargo build --bin pierre-mcp-server --bin pierre-cli --bin pierre-dev-fixture 2>&1 | tail -3
fi
echo "    Build complete"

# Step 4: Run migrations and seeders
# pierre-cli runs database migrations on startup, so no temporary server needed.
# All seeders use direct DB access (no HTTP API required).
print_step 4 "Running migrations and seeders..."

PIERRE_CLI="./target/$TARGET_DIR/pierre-cli"

# Create admin user (also runs database migrations)
# Operators get the SuperAdmin role: cookie_admin_middleware derives console
# permissions from the user's role, so the operator account must be super_admin
# to retain full console access (contremaitre config, store moderation,
# impersonation). A plain `admin` role is intentionally limited.
echo "    Creating admin user (runs migrations)..."
"$PIERRE_CLI" user create \
    --email "$ADMIN_EMAIL" \
    --password "$ADMIN_PASSWORD" \
    --super-admin \
    --force 2>&1 | tail -3

# Seed coaches from a contremaitre checkout. Coach definitions live in the
# dravr-contremaitre repo as the single source of truth; expect either
# PIERRE_COACHES_DIR set in .envrc or a sibling working-copy at
# ../dravr-contremaitre. Override with PIERRE_COACHES_DIR if your checkout
# lives elsewhere.
echo "    Seeding AI coaches from contremaitre..."
COACHES_DIR="${PIERRE_COACHES_DIR:-../dravr-contremaitre/prompts/coaches}"
if [ ! -d "$COACHES_DIR" ]; then
    echo "    ! Coach source not found at $COACHES_DIR"
    echo "      Clone https://github.com/dravr-ai/dravr-contremaitre"
    echo "      next to this repo, or set PIERRE_COACHES_DIR explicitly."
    exit 1
fi
"$PIERRE_CLI" seed coaches --coaches-dir "$COACHES_DIR" 2>&1 | tail -3

# Seed demo users (direct DB, no server needed)
echo "    Seeding demo users..."
"$PIERRE_CLI" seed demo-data --days 30 2>&1 | tail -3

# Seed social data (includes webtest/mobiletest users)
echo "    Seeding social test data..."
"$PIERRE_CLI" seed social 2>&1 | tail -3

# Seed mobility data
echo "    Seeding mobility data (stretches, yoga)..."
"$PIERRE_CLI" seed mobility 2>&1 | tail -3

# Create the phil_test + jf_test users before synthetic seeding. The web/mobile
# test users are created by `seed social` above, but these two are not created by
# any seeder, so create them here (idempotent via --force) to keep the script
# self-contained on a fresh database. Without this, the synthetic step below
# aborts with "User not found" on a clean checkout.
echo "    Creating phil_test + jf_test users..."
"$PIERRE_CLI" user create --email "$PHIL_TEST_EMAIL" --password "$DRAVR_TEST_PASSWORD" --force 2>&1 | tail -1
"$PIERRE_CLI" user create --email "$JF_TEST_EMAIL" --password "$DRAVR_TEST_PASSWORD" --force 2>&1 | tail -1

# Seed dev activities for test users. Each user is seeded as a real Strava or
# Garmin athlete (provider_connection + oauth_token); the dev fixture API (started
# below) serves their activities back in that provider's exact API format through
# the real provider code path, so the recommender and all provider-backed features
# work exactly as they would in prod. server-full compiles in both provider-strava
# and provider-garmin (dev only — server-production keeps the prod provider set).
# Mix providers so both paths are exercised: web/phil = Strava, mobile/jf = Garmin.
echo "    Seeding dev provider activities (Strava + Garmin) for test users..."
"$PIERRE_CLI" seed synthetic-activities --email "$WEB_TEST_EMAIL" --provider strava --count 30 --days 30 2>&1 | tail -1
"$PIERRE_CLI" seed synthetic-activities --email "$MOBILE_TEST_EMAIL" --provider garmin --count 30 --days 30 2>&1 | tail -1
"$PIERRE_CLI" seed synthetic-activities --email "$PHIL_TEST_EMAIL" --provider strava --count 30 --days 30 2>&1 | tail -1
"$PIERRE_CLI" seed synthetic-activities --email "$JF_TEST_EMAIL" --provider garmin --count 30 --days 30 2>&1 | tail -1
# Canonical demo users (created by `seed demo-data` above) need a provider too,
# otherwise they hit the onboarding hard-gate and get zero coach recommendations.
# The synthetic provider they used to fall back on was removed, so seed them as
# real fixture-backed athletes: alice = Strava, bob = Garmin (exercises both paths).
"$PIERRE_CLI" seed synthetic-activities --email alice@acme.com --provider strava --count 30 --days 30 2>&1 | tail -1
"$PIERRE_CLI" seed synthetic-activities --email bob@startup.io --provider garmin --count 30 --days 30 2>&1 | tail -1

# Seed LLM usage data for consumption analytics dashboard
echo "    Seeding LLM usage data (30 days)..."
"$PIERRE_CLI" seed llm-usage --admin-email "$ADMIN_EMAIL" --days 30 2>&1 | tail -3

echo "    All seeders complete"

# Step 4b: Start the dev fixture API (serves seeded Strava/Garmin activities).
FIXTURE_LOG="$LOG_DIR/fixture.log"
echo "    Starting dev fixture API (port $FIXTURE_PORT)..."
pkill -f "pierre-dev-fixture" 2>/dev/null || true
FIXTURE_PORT="$FIXTURE_PORT" ./target/$TARGET_DIR/pierre-dev-fixture > "$FIXTURE_LOG" 2>&1 &
FIXTURE_PID=$!
for i in {1..15}; do
    if curl -s -f "http://127.0.0.1:$FIXTURE_PORT/health" > /dev/null 2>&1; then
        echo "    Fixture ready (PID: $FIXTURE_PID)"
        break
    fi
    if [ $i -eq 15 ]; then
        echo -e "${YELLOW}    Fixture did not become healthy; activities may not load. Check: tail -f $FIXTURE_LOG${NC}"
    fi
    sleep 1
done
# Point the real Strava/Garmin providers at the local fixture (dev only).
# Production never sets these, so it always hits the real provider APIs.
export PIERRE_STRAVA_API_BASE_URL="http://127.0.0.1:$FIXTURE_PORT"
export PIERRE_GARMIN_API_BASE_URL="http://127.0.0.1:$FIXTURE_PORT"
# Dummy Garmin OAuth creds (dev only). The provider requires client creds to
# be constructed and to register as "enabled" (it keys on GARMIN_CLIENT_ID);
# the seeded long-lived token means refresh never fires, and the fixture
# ignores credential validity — so placeholders are sufficient to exercise
# the real Garmin code path. Strava creds already come from .envrc.
export GARMIN_CLIENT_ID="${GARMIN_CLIENT_ID:-dev-fixture-garmin-client}"
export GARMIN_CLIENT_SECRET="${GARMIN_CLIENT_SECRET:-dev-fixture-garmin-secret}"

# Step 4c: Start the dedicated sciotte scraper service (ADR-021), but ONLY when
# the platform is pointed at it. DRAVR_SCIOTTE_REMOTE_URL set => the server
# routes sciotte login/scrape to this service instead of in-pod Chrome, so it
# must be up; unset => the in-process path and this block is skipped entirely
# (dev loop unchanged). Delegates to sciotte-local.sh so the serve invocation
# has a single source of truth. Never fatal: a missing ../dravr-sciotte checkout
# or a failed build warns and skips so the core stack always comes up; teardown
# is handled by stop-all.sh (dravr-sciotte-server + port 8091).
if [ -n "${DRAVR_SCIOTTE_REMOTE_URL:-}" ]; then
    echo -e "${CYAN}    Starting sciotte scraper service (ADR-021 -> $DRAVR_SCIOTTE_REMOTE_URL)...${NC}"
    if [ -d "$PROJECT_ROOT/../dravr-sciotte" ]; then
        # No provider arg: one instance serving garmin+strava (ADR-021);
        # set SCIOTTE_PROVIDER to pin a single provider instead.
        if "$PROJECT_ROOT/scripts/sciotte-local.sh" serve-bg; then
            echo "    Sciotte service ready"
        else
            echo -e "${YELLOW}    Sciotte service failed to start — remote sciotte scrapes will error until it is up (see logs/sciotte-service-*.log)${NC}"
        fi
    else
        echo -e "${YELLOW}    ../dravr-sciotte not found — skipping sciotte service; remote sciotte scrapes will error${NC}"
    fi
fi

# Step 5: Start Pierre server
print_step 5 "Starting Pierre MCP Server (port $SERVER_PORT)..."
RUST_LOG=info ./target/$TARGET_DIR/pierre-mcp-server > "$SERVER_LOG" 2>&1 &
SERVER_PID=$!

# Wait for health check
for i in {1..30}; do
    if curl -s -f "http://127.0.0.1:$SERVER_PORT/health" > /dev/null 2>&1; then
        echo "    Server ready (PID: $SERVER_PID)"
        break
    fi
    if [ $i -eq 30 ]; then
        echo -e "${RED}    Server failed to start. Check: tail -f $SERVER_LOG${NC}"
        exit 1
    fi
    sleep 1
done

# Step 5b: Start Cloudflare tunnel (if --tunnel)
if [ "$START_TUNNEL" = "true" ]; then
    echo -e "${CYAN}    Starting Cloudflare tunnel...${NC}"
    if ! command -v cloudflared &> /dev/null; then
        echo -e "${RED}    cloudflared not installed. Run: brew install cloudflare/cloudflare/cloudflared${NC}"
        echo -e "${YELLOW}    Skipping tunnel setup${NC}"
    else
        TUNNEL_LOG="$LOG_DIR/tunnel.log"
        cloudflared tunnel --url http://localhost:$SERVER_PORT > "$TUNNEL_LOG" 2>&1 &
        TUNNEL_PID=$!

        # Wait for tunnel URL
        TUNNEL_URL=""
        for i in {1..30}; do
            TUNNEL_URL=$(grep -o 'https://[a-z0-9-]*\.trycloudflare\.com' "$TUNNEL_LOG" 2>/dev/null | head -1) || true
            if [ -n "$TUNNEL_URL" ]; then
                break
            fi
            sleep 1
        done

        if [ -n "$TUNNEL_URL" ]; then
            echo "    Tunnel URL: $TUNNEL_URL"

            # Update BASE_URL in .envrc
            if grep -q '^export BASE_URL=' "$PROJECT_ROOT/.envrc" 2>/dev/null; then
                sed -i.bak "s|^export BASE_URL=.*|export BASE_URL=\"$TUNNEL_URL\"|" "$PROJECT_ROOT/.envrc"
                rm -f "$PROJECT_ROOT/.envrc.bak"
            else
                echo "export BASE_URL=\"$TUNNEL_URL\"" >> "$PROJECT_ROOT/.envrc"
            fi

            # Update mobile .env
            echo "EXPO_PUBLIC_API_URL=\"$TUNNEL_URL\"" > "$PROJECT_ROOT/frontend-mobile/.env"
            echo "    Updated .envrc and frontend-mobile/.env"

            # Restart server so it picks up BASE_URL for OAuth callbacks
            kill $SERVER_PID 2>/dev/null || true
            sleep 1
            set -a
            source "$PROJECT_ROOT/.envrc"
            set +a
            # Re-assert the dev fixture overrides — re-sourcing .envrc must not
            # send the providers back to the real Strava/Garmin APIs.
            export PIERRE_STRAVA_API_BASE_URL="http://127.0.0.1:$FIXTURE_PORT"
            export PIERRE_GARMIN_API_BASE_URL="http://127.0.0.1:$FIXTURE_PORT"
            RUST_LOG=info ./target/$TARGET_DIR/pierre-mcp-server > "$SERVER_LOG" 2>&1 &
            SERVER_PID=$!
            for i in {1..15}; do
                if curl -s -f "http://127.0.0.1:$SERVER_PORT/health" > /dev/null 2>&1; then
                    echo "    Server restarted with tunnel BASE_URL (PID: $SERVER_PID)"
                    break
                fi
                sleep 1
            done
        else
            echo -e "${RED}    Failed to get tunnel URL. Check: tail -f $TUNNEL_LOG${NC}"
            TUNNEL_URL=""
        fi
    fi
fi

# Step 6: Install frontend dependencies
print_step 6 "Installing frontend dependencies..."
if [ -d "$PROJECT_ROOT/frontend" ]; then
    cd "$PROJECT_ROOT/frontend"
    bun install --frozen-lockfile > /dev/null 2>&1
    echo "    frontend/ dependencies installed"
fi
if [ -d "$PROJECT_ROOT/frontend-mobile" ]; then
    cd "$PROJECT_ROOT/frontend-mobile"
    bun install --frozen-lockfile > /dev/null 2>&1
    echo "    frontend-mobile/ dependencies installed"
fi
cd "$PROJECT_ROOT"

# Step 7: Start web frontend
print_step 7 "Starting Web Frontend (port $FRONTEND_PORT)..."
if [ -d "$PROJECT_ROOT/frontend" ]; then
    cd "$PROJECT_ROOT/frontend"
    bun run dev > "$FRONTEND_LOG" 2>&1 &
    FRONTEND_PID=$!
    cd "$PROJECT_ROOT"
    echo "    Frontend starting (PID: $FRONTEND_PID)"
else
    echo -e "${YELLOW}    frontend/ not found, skipping${NC}"
    FRONTEND_PID=""
fi

# Step 8: Start Expo Mobile
print_step 8 "Starting Expo Mobile (port $EXPO_PORT)..."
if [ -d "$PROJECT_ROOT/frontend-mobile" ]; then
    cd "$PROJECT_ROOT/frontend-mobile"

    if [ "$NATIVE_BUILD" = "true" ]; then
        # --native flag: build native app with Xcode (for speech recognition, native MMKV)
        echo "    Native build requested — using bin/build-native-app.sh..."
        "$PROJECT_ROOT/bin/build-native-app.sh" --no-bundler > "$EXPO_LOG" 2>&1 &
        EXPO_PID=$!
        echo "    Building native iOS app (this may take several minutes)..."
        echo "    Watch progress: tail -f $EXPO_LOG"
        # Start Metro in dev-client mode since native build uses --no-bundler
        npx expo start --dev-client --port "$EXPO_PORT" >> "$EXPO_LOG" 2>&1 &
    else
        # Default: use Expo Go (fast, no Xcode needed)
        # --ios installs Expo Go if missing and launches on simulator
        # --go forces Expo Go mode (not dev client)
        npx expo start --ios --go --port "$EXPO_PORT" > "$EXPO_LOG" 2>&1 &
        EXPO_PID=$!
    fi

    cd "$PROJECT_ROOT"
    echo "    Expo starting (PID: $EXPO_PID)"
else
    echo -e "${YELLOW}    frontend-mobile/ not found, skipping${NC}"
    EXPO_PID=""
fi

# Step 9: Generate admin token
print_step 9 "Generating admin API token..."
ADMIN_LOGIN=$(curl -s -X POST "http://localhost:$SERVER_PORT/oauth/token" \
    -H "Content-Type: application/x-www-form-urlencoded" \
    -d "grant_type=password&username=$ADMIN_EMAIL&password=$ADMIN_PASSWORD")
ADMIN_TOKEN=$(echo "$ADMIN_LOGIN" | jq -r '.access_token // empty')

if [ -n "$ADMIN_TOKEN" ]; then
    echo "$ADMIN_TOKEN" > "$LOG_DIR/admin-token.txt"
    echo "    Token saved to: $LOG_DIR/admin-token.txt"
else
    echo -e "${YELLOW}    Could not generate token (will work after first login)${NC}"
    ADMIN_TOKEN="(login to generate)"
fi

# Wait for services to stabilize
sleep 2

# Print summary
echo ""
echo -e "${BLUE}============================================================================${NC}"
echo -e "${BLUE}   SETUP COMPLETE${NC}"
echo -e "${BLUE}============================================================================${NC}"
echo ""
echo -e "${CYAN}=== Test Credentials ===${NC}"
echo ""
printf "%-20s %-30s %-20s\n" "User Type" "Email" "Password"
printf "%-20s %-30s %-20s\n" "────────────────────" "──────────────────────────────" "────────────────────"
printf "%-20s %-30s %-20s\n" "Admin" "$ADMIN_EMAIL" "$ADMIN_PASSWORD"
printf "%-20s %-30s %-20s\n" "Web Test" "$WEB_TEST_EMAIL" "$WEB_TEST_PASSWORD"
printf "%-20s %-30s %-20s\n" "Mobile Test" "$MOBILE_TEST_EMAIL" "$MOBILE_TEST_PASSWORD"
printf "%-20s %-30s %-20s\n" "Phil Test" "$PHIL_TEST_EMAIL" "$DRAVR_TEST_PASSWORD"
printf "%-20s %-30s %-20s\n" "JF Test" "$JF_TEST_EMAIL" "$DRAVR_TEST_PASSWORD"
printf "%-20s %-30s %-20s\n" "Demo Users" "alice@acme.com, bob@startup.io" "$DEMO_PASSWORD"
echo ""
echo -e "${CYAN}=== Admin API Token ===${NC}"
echo ""
if [ "$ADMIN_TOKEN" != "(login to generate)" ]; then
    echo "${ADMIN_TOKEN:0:60}..."
else
    echo "$ADMIN_TOKEN"
fi
echo ""
echo -e "${CYAN}=== Services ===${NC}"
echo ""
printf "%-15s %-35s %-10s %-8s\n" "Service" "URL" "Status" "PID"
printf "%-15s %-35s %-10s %-8s\n" "───────────────" "───────────────────────────────────" "──────────" "────────"

# Check server
if curl -s -f "http://127.0.0.1:$SERVER_PORT/health" > /dev/null 2>&1; then
    printf "%-15s %-35s ${GREEN}%-10s${NC} %-8s\n" "Pierre Server" "http://localhost:$SERVER_PORT" "Running" "$SERVER_PID"
else
    printf "%-15s %-35s ${RED}%-10s${NC} %-8s\n" "Pierre Server" "http://localhost:$SERVER_PORT" "Down" "-"
fi

# Check frontend (port-based: bun/vite may spawn child processes with different PIDs)
if [ -z "$FRONTEND_PID" ]; then
    printf "%-15s %-35s ${YELLOW}%-10s${NC} %-8s\n" "Web Frontend" "http://localhost:$FRONTEND_PORT" "Skipped" "-"
elif curl -s -o /dev/null --connect-timeout 2 "http://localhost:$FRONTEND_PORT" 2>/dev/null; then
    printf "%-15s %-35s ${GREEN}%-10s${NC} %-8s\n" "Web Frontend" "http://localhost:$FRONTEND_PORT" "Running" "$FRONTEND_PID"
else
    printf "%-15s %-35s ${YELLOW}%-10s${NC} %-8s\n" "Web Frontend" "http://localhost:$FRONTEND_PORT" "Starting" "$FRONTEND_PID"
fi

# Check Expo (port-based: bun spawns Metro as a child process with a different PID)
if [ -z "$EXPO_PID" ]; then
    printf "%-15s %-35s ${YELLOW}%-10s${NC} %-8s\n" "Expo Mobile" "http://localhost:$EXPO_PORT" "Skipped" "-"
elif curl -s -o /dev/null --connect-timeout 2 "http://localhost:$EXPO_PORT" 2>/dev/null; then
    printf "%-15s %-35s ${GREEN}%-10s${NC} %-8s\n" "Expo Mobile" "http://localhost:$EXPO_PORT" "Running" "$EXPO_PID"
else
    printf "%-15s %-35s ${YELLOW}%-10s${NC} %-8s\n" "Expo Mobile" "http://localhost:$EXPO_PORT" "Starting" "$EXPO_PID"
fi

# Check tunnel
if [ -n "${TUNNEL_URL:-}" ]; then
    printf "%-15s %-35s ${GREEN}%-10s${NC} %-8s\n" "CF Tunnel" "$TUNNEL_URL" "Running" "$TUNNEL_PID"
fi

echo ""
echo -e "${CYAN}=== Log Files ===${NC}"
echo ""
echo "  Pierre Server:  tail -f $SERVER_LOG"
echo "  Web Frontend:   tail -f $FRONTEND_LOG"
echo "  Expo Mobile:    tail -f $EXPO_LOG"
echo "  All logs:       tail -f $LOG_DIR/*.log"
echo ""
echo -e "${CYAN}=== Quick Commands ===${NC}"
echo ""
echo "  Stop all:       ./bin/stop-all.sh"
echo "  Server only:    ./bin/start-server.sh"
echo "  Reset & start:  ./bin/setup-db-with-seeds-and-oauth-and-start-servers.sh"
echo ""
echo -e "${GREEN}Ready for development!${NC}"
echo ""

# Stream all logs to console if --stream-logs was passed
if [ "$STREAM_LOGS" = "true" ]; then
    echo -e "${CYAN}Streaming all logs (Ctrl+C to stop)...${NC}"
    echo ""
    tail -f "$SERVER_LOG" "$FRONTEND_LOG" "$EXPO_LOG" 2>/dev/null
fi
