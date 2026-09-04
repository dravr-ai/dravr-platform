#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: MCP protocol compliance validation script
# ABOUTME: Tests pierre-claude-bridge against Model Context Protocol specification
#
# Licensed under either of Apache License, Version 2.0 or MIT License at your option.
# Copyright (c) 2026 dravr.ai

# Pierre MCP Compliance Validation Script
# Tests the pierre-claude-bridge against the MCP (Model Context Protocol) specification
# Can be run standalone or called from lint-and-test.sh
#
# Usage: ./scripts/ensure_mcp_compliance.sh

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Find bun executable early (use full path for subprocess compatibility)
# Python subprocess doesn't always inherit shell PATH, so we need absolute path
BUN_PATH=$(which bun 2>/dev/null || echo "$HOME/.bun/bin/bun")
if [ ! -x "$BUN_PATH" ]; then
    # Try common installation locations
    for candidate in "$HOME/.bun/bin/bun" "/usr/local/bin/bun" "/opt/homebrew/bin/bun"; do
        if [ -x "$candidate" ]; then
            BUN_PATH="$candidate"
            break
        fi
    done
fi

# Get the directory where this script is located
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"

echo -e "${BLUE}==== Pierre MCP Compliance Validation ====${NC}"
echo "Project root: $PROJECT_ROOT"

# Track success
COMPLIANCE_PASSED=true

# Track Pierre MCP server PID if we start it
MCP_SERVER_PID=""
SERVER_LOG=""

# Cleanup function - shut down server if we started it
cleanup_mcp_server() {
    # No probe teardown here: each inspector run is foreground and bounded by
    # `timeout`, so it and its bridge child are gone before this runs. The old
    # process-group kill existed because the python validator was backgrounded
    # and ignored SIGTERM.
    if [ -n "$MCP_SERVER_PID" ]; then
        echo ""
        echo -e "${BLUE}==== Shutting down Pierre MCP server (PID: $MCP_SERVER_PID)... ====${NC}"
        kill "$MCP_SERVER_PID" 2>/dev/null || true
        # Bounded graceful-shutdown wait, then SIGKILL. An unbounded `wait` here
        # hangs the entire step forever if the server ignores SIGTERM — give it
        # 10s to exit cleanly, then force-kill so the step always terminates.
        for _ in $(seq 1 10); do
            kill -0 "$MCP_SERVER_PID" 2>/dev/null || break
            sleep 1
        done
        kill -KILL "$MCP_SERVER_PID" 2>/dev/null || true
        echo -e "${GREEN}[OK] Pierre MCP server stopped${NC}"
        MCP_SERVER_PID=""
    fi

    # Clean up temp log file
    if [ -n "$SERVER_LOG" ] && [ -f "$SERVER_LOG" ]; then
        rm -f "$SERVER_LOG"
    fi
}

# Handle CTRL-C gracefully
handle_interrupt() {
    echo ""
    echo -e "${YELLOW}⚠️  Received interrupt signal - cleaning up...${NC}"
    cleanup_mcp_server
    exit 130
}

# Register cleanup and signal handlers
trap cleanup_mcp_server EXIT
trap handle_interrupt INT TERM

# Change to SDK directory
cd "$PROJECT_ROOT/sdk"

echo ""
echo -e "${BLUE}==== MCP Spec Compliance Validation ====${NC}"

# Check if SDK directory exists
if [ ! -d "." ]; then
    echo -e "${RED}[FAIL] SDK directory not found${NC}"
    exit 1
fi

# Look for Python MCP validator
echo -e "${BLUE}==== Checking for the MCP inspector (REQUIRED)... ====${NC}"

# The compliance client is the official inspector's CLI mode, run through npx so
# there is nothing to install or keep in sync. The version is PINNED: an unpinned
# validator is a lane that can turn red on someone else's release.
#
# This replaced Janix-ai/mcp-validator, which the lane cloned at build time until
# that repository was deleted or made private on 2026-08-27 (carnet#127). A
# single-owner repository is a single point of failure for a required gate; the
# protocol org's own tool is the one thing here we can reasonably expect to
# outlive us.
MCP_INSPECTOR_PKG="@modelcontextprotocol/inspector@2.4.0"

if ! command -v npx >/dev/null 2>&1; then
    echo -e "${RED}[FAIL] npx not found - the MCP inspector cannot run${NC}"
    echo -e "${RED}       Node is required; CI already provisions it for the SDK lane${NC}"
    exit 1
fi
echo -e "${GREEN}[OK] Using ${MCP_INSPECTOR_PKG} via npx${NC}"
echo -e "${BLUE}==== Building pierre-claude-bridge for compliance testing... ====${NC}"
echo -e "${BLUE}     Using bun at: $BUN_PATH${NC}"
if $BUN_PATH run build; then
    echo -e "${GREEN}[OK] Bridge built successfully${NC}"
else
    echo -e "${RED}[FAIL] Bridge build failed${NC}"
    exit 1
fi

# Check if Pierre MCP server is running (required for bridge testing)
echo -e "${BLUE}==== Checking if Pierre MCP server is accessible... ====${NC}"
SERVER_ALREADY_RUNNING=false
if curl -s -f -m 2 http://localhost:8080/health >/dev/null 2>&1; then
    echo -e "${GREEN}[OK] Pierre MCP server is already running${NC}"
    SERVER_ALREADY_RUNNING=true
else
    echo -e "${YELLOW}[INFO] Pierre MCP server not running - starting it automatically...${NC}"

    # Start Pierre MCP server in background
    echo -e "${BLUE}==== Starting Pierre MCP server for testing... ====${NC}"

    # Check if we have a debug or release binary already (use absolute paths since we're in sdk/)
    # Prioritize debug binary to ensure latest code is tested
    SERVER_BINARY=""
    if [ -n "${PIERRE_SERVER_BINARY:-}" ] && [ -f "${PIERRE_SERVER_BINARY}" ]; then
        # CI pre-builds the server (off the warm release cache) and points here, so
        # the validation step never cold-builds inline (which OOM-thrashed the runner).
        SERVER_BINARY="${PIERRE_SERVER_BINARY}"
        echo -e "${GREEN}[OK] Using pre-built binary: ${PIERRE_SERVER_BINARY}${NC}"
    elif [ -f "$PROJECT_ROOT/target/debug/pierre-mcp-server" ]; then
        SERVER_BINARY="$PROJECT_ROOT/target/debug/pierre-mcp-server"
        echo -e "${GREEN}[OK] Using existing debug binary${NC}"
    elif [ -f "$PROJECT_ROOT/target/release/pierre-mcp-server" ]; then
        SERVER_BINARY="$PROJECT_ROOT/target/release/pierre-mcp-server"
        echo -e "${GREEN}[OK] Using existing release binary${NC}"
    else
        echo -e "${BLUE}Building pierre-mcp-server (release)...${NC}"
        # Build from project root, not from sdk/ — release reuses the warm CI cache
        # and avoids the debug build's debuginfo OOM on small runners.
        if (cd "$PROJECT_ROOT" && cargo build --release --bin pierre-mcp-server --quiet 2>&1); then
            SERVER_BINARY="$PROJECT_ROOT/target/release/pierre-mcp-server"
            echo -e "${GREEN}[OK] Binary built successfully${NC}"
        else
            echo -e "${RED}[FAIL] Failed to build pierre-mcp-server${NC}"
            exit 1
        fi
    fi

    if [ -n "$SERVER_BINARY" ]; then
        # Start server with minimal environment (using CI test key)
        # Redirect to temp log file for debugging startup issues
        SERVER_LOG="/tmp/pierre-mcp-server-$$.log"
        CI=true \
        HTTP_PORT=8080 \
        DATABASE_URL=sqlite::memory: \
        PIERRE_MASTER_ENCRYPTION_KEY=rEFe91l6lqLahoyl9OSzum9dKa40VvV5RYj8bHGNTeo= \
        PIERRE_ALLOW_INTERACTIVE_OAUTH=false \
        PIERRE_RSA_KEY_SIZE=2048 \
        RUST_LOG="${RUST_LOG:-info,pierre_mcp_server::mcp=debug,pierre_tool_runtime=debug}" \
        "$SERVER_BINARY" >"$SERVER_LOG" 2>&1 &
        MCP_SERVER_PID=$!

        echo -e "${GREEN}[OK] Pierre MCP server started (PID: $MCP_SERVER_PID)${NC}"
        echo -e "${BLUE}     Server logs: $SERVER_LOG${NC}"

        # Wait for server to be ready (health check)
        echo -e "${BLUE}==== Waiting for Pierre MCP server to be ready... ====${NC}"
        MAX_WAIT=60
        WAIT_COUNT=0
        while [ $WAIT_COUNT -lt $MAX_WAIT ]; do
            # Check if server process is still alive
            if ! kill -0 "$MCP_SERVER_PID" 2>/dev/null; then
                echo -e "${RED}[FAIL] Server process died unexpectedly${NC}"
                echo -e "${RED}       Last 20 lines of server log:${NC}"
                tail -20 "$SERVER_LOG" 2>/dev/null || echo "No log output"
                exit 1
            fi

            if curl -s -f -m 2 http://localhost:8080/health >/dev/null 2>&1; then
                echo -e "${GREEN}[OK] Pierre MCP server is ready (took ${WAIT_COUNT}s)${NC}"
                break
            fi
            sleep 1
            WAIT_COUNT=$((WAIT_COUNT + 1))
        done

        if [ $WAIT_COUNT -ge $MAX_WAIT ]; then
            echo -e "${RED}[FAIL] Pierre MCP server failed to become ready after ${MAX_WAIT}s${NC}"
            echo -e "${RED}       Server process status: $(kill -0 "$MCP_SERVER_PID" 2>/dev/null && echo 'running' || echo 'dead')${NC}"
            echo -e "${RED}       Last 30 lines of server log:${NC}"
            tail -30 "$SERVER_LOG" 2>/dev/null || echo "No log output"
            exit 1
        fi
    fi
fi

# Run MCP compliance checks (REQUIRED - NO EXCEPTIONS POLICY)
echo -e "${BLUE}==== Running MCP protocol compliance checks (REQUIRED)... ====${NC}"
BRIDGE_PATH="$(pwd)/dist/cli.js"
echo -e "${BLUE}     Bridge under test: $BUN_PATH $BRIDGE_PATH${NC}"

if [ ! -f "$BRIDGE_PATH" ]; then
    echo -e "${RED}[FAIL] Bridge not found at $BRIDGE_PATH${NC}"
    exit 1
fi

# stdio, not HTTP, and the bridge rather than the server: an MCP host launches
# the bridge, so that is what the protocol contract is actually observed at.
# Pointing the inspector at the server's HTTP endpoint would test a different
# thing and leave the bridge - the half with no Rust coverage - unexercised.
INSPECTOR_TIMEOUT=180
TIMEOUT_CMD=""
if command -v timeout >/dev/null 2>&1; then
    TIMEOUT_CMD="timeout --kill-after=30 $INSPECTOR_TIMEOUT"
elif command -v gtimeout >/dev/null 2>&1; then
    TIMEOUT_CMD="gtimeout --kill-after=30 $INSPECTOR_TIMEOUT"
else
    echo -e "${YELLOW}[WARN] no timeout command; a wedged probe will not be bounded${NC}"
fi

INSPECTOR_OUT="$(mktemp)"
INSPECTOR_ERR="$(mktemp)"

# `npx --yes` downloads the inspector and its dependency tree on first use, and
# every probe below runs under INSPECTOR_TIMEOUT. Without this step the first
# probe is timing a ~4MB package install plus its tree against a budget meant
# for a wedged protocol call, and the two probes after it are fast only because
# the first one paid. That difference is the whole spread: the same probe has
# measured 17s and 141s against a 180s bound on unchanged tool schemas, and has
# timed out at 180s, purely on registry latency.
#
# Fetching it here leaves the probe timeout measuring the protocol call. The
# bound is separate and generous because a slow registry is not a compliance
# failure; if it fails, the probes still run and simply pay the download as
# before, so this can only make the lane more accurate, never less.
echo -e "${BLUE}==== Fetching the MCP inspector before the timed probes... ====${NC}"
# Generous, because the fetch is the slow part and its duration is the registry's
# to decide: 432s measured on a runner where the probe itself then took 3.8s.
INSPECTOR_FETCH_TIMEOUT=900
if [ -n "$TIMEOUT_CMD" ]; then
    INSPECTOR_FETCH_CMD="${TIMEOUT_CMD%% *} --kill-after=30 $INSPECTOR_FETCH_TIMEOUT"
else
    INSPECTOR_FETCH_CMD=""
fi
fetch_start=$(date +%s)
fetch_code=0
CI=true $INSPECTOR_FETCH_CMD npx --yes "$MCP_INSPECTOR_PKG" --cli --help \
    >/dev/null 2>&1 || fetch_code=$?
fetch_secs=$(($(date +%s) - fetch_start))
if [ "$fetch_code" -eq 124 ] || [ "$fetch_code" -eq 137 ]; then
    # Only a timeout tells us the cache is unpopulated. Say so: the probes below
    # will pay the download inside their own budget and may time out for that
    # reason rather than a protocol one, and that must not read as a schema fault.
    echo -e "${YELLOW}[WARN] Inspector fetch hit its ${INSPECTOR_FETCH_TIMEOUT}s bound${NC}"
    echo -e "${YELLOW}       A probe timeout below is the registry, not the server${NC}"
else
    # Any other exit is --help's own business; the cache is populated either way,
    # which is all this step is for.
    echo -e "${GREEN}[OK] Inspector cached in ${fetch_secs}s${NC}"
fi

# $1 = label, $2 = expected exit code, rest = inspector arguments.
# CI=true makes the bridge use encrypted file storage instead of keytar, which
# otherwise blocks on a headless runner.
run_probe() {
    local label="$1"; shift
    local want="$1"; shift
    local code=0

    echo -e "${BLUE}     -> ${label}${NC}"
    CI=true $TIMEOUT_CMD npx --yes "$MCP_INSPECTOR_PKG" --cli \
        "$BUN_PATH" "$BRIDGE_PATH" "$@" >"$INSPECTOR_OUT" 2>"$INSPECTOR_ERR" || code=$?

    if [ "$code" -eq "$want" ]; then
        echo -e "${GREEN}     [OK] ${label}${NC}"
        return 0
    fi

    echo -e "${RED}     [FAIL] ${label} - exit ${code}, expected ${want}${NC}"
    # The bridge narrates its own startup on stderr; keep the tail, where the
    # actual error lands.
    grep -vE 'ExperimentalWarning|trace-warnings|npm warn|ES Module' "$INSPECTOR_ERR" \
        | tail -20 | sed 's/^/       /'
    head -c 800 "$INSPECTOR_OUT" | sed 's/^/       /'
    COMPLIANCE_PASSED=false
    return 1
}

# tools/list under --strict: the inspector reports tool-schema portability
# problems in full and exits 6 if any is error-severity. This is the part our own
# Rust and SDK suites cannot do for us - they assert our schemas against our own
# reading of the spec, and a misreading we share cannot be caught from inside.
run_probe "tools/list (schema portability, --strict)" 0 --method tools/list --strict || true

# The other two advertised surfaces. A server that names a capability in
# initialize and then errors on the matching list call is the exact defect
# mcp_resources_prompts_conformance_test.rs guards server-side; this covers the
# same ground from outside, through the bridge.
run_probe "resources/list" 0 --method resources/list --format json || true
run_probe "prompts/list" 0 --method prompts/list --format json || true

rm -f "$INSPECTOR_OUT" "$INSPECTOR_ERR"

if [ "$COMPLIANCE_PASSED" = true ]; then
    echo ""
    echo -e "${GREEN}==== MCP compliance checks passed ====${NC}"
    exit 0
fi

echo ""
echo -e "${RED}==== MCP compliance checks FAILED ====${NC}"
echo -e "${RED}     Reproduce locally:${NC}"
echo -e "${RED}       cd sdk && npx ${MCP_INSPECTOR_PKG} --cli \$(which bun) \$(pwd)/dist/cli.js --method tools/list --strict${NC}"
exit 1
