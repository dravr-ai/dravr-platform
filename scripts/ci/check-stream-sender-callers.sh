#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: WARNS when a public SSE/WS/A2A stream-sender method has no production caller
# ABOUTME: A send_*/broadcast_* with zero non-test callers is an unwired stream path (e.g. send_a2a_task_update)

# A `pub fn send_*` / `pub fn broadcast_*` on an SSE / WebSocket / A2A stream
# manager exists to push data to a connected client. If nothing in production
# code ever calls it, the stream path is unwired — the method advertises a
# capability the server never exercises. The A2A SSE `send_a2a_task_update`
# was dead exactly this way.
#
# Caller detection across trait objects and re-exports is heuristic (a method
# can be invoked dynamically, or a test client can define a same-named method),
# so this gate is WARNING-ONLY. It reports two buckets:
#   - NO CALLERS: zero call sites in src/ and zero in tests/ (strongest signal)
#   - TEST-ONLY:  zero call sites in src/, but called from tests/
# It never fails the build.

# No `set -e`: this is an advisory gate that always exits 0, and the rg -c
# caller counts legitimately exit nonzero when a method has zero call sites.
set -uo pipefail

YELLOW='\033[1;33m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"
cd "$PROJECT_ROOT"

echo -e "${BLUE}==== Stream-sender caller detection (advisory) ====${NC}"

# Crates/dirs that own SSE / WebSocket / A2A streaming managers.
SCAN_DIRS=(
    crates/pierre-sse/src
    crates/pierre-server/src/sse
    crates/pierre-a2a/src
    crates/pierre-routes-a2a/src
)

EXISTING_DIRS=()
for d in "${SCAN_DIRS[@]}"; do
    [ -d "$d" ] && EXISTING_DIRS+=("$d")
done

if [ "${#EXISTING_DIRS[@]}" -eq 0 ]; then
    echo -e "${GREEN}✅ No stream-manager source dirs present — nothing to check${NC}"
    exit 0
fi

NO_CALLER_LIST=""
TEST_ONLY_LIST=""

while IFS= read -r line; do
    # line looks like: path:lineno:    pub async fn send_foo(
    name=$(echo "$line" | grep -oE 'fn (send_|broadcast_)[a-z0-9_]+' | sed 's/fn //')
    [ -z "$name" ] && continue
    loc=$(echo "$line" | cut -d: -f1-2)
    src_callers=$( { rg -c -e "\.${name}\s*\(" crates/*/src 2>/dev/null || true; } | awk -F: '{s+=$2} END{print s+0}')
    test_callers=$( { rg -c -e "\.${name}\s*\(" crates/*/tests 2>/dev/null || true; } | awk -F: '{s+=$2} END{print s+0}')
    if [ "$src_callers" -eq 0 ]; then
        if [ "$test_callers" -eq 0 ]; then
            NO_CALLER_LIST="${NO_CALLER_LIST}${name}  (${loc})\n"
        else
            TEST_ONLY_LIST="${TEST_ONLY_LIST}${name}  (${loc}) — ${test_callers} test caller(s)\n"
        fi
    fi
done < <(rg -ne 'pub (async )?fn (send_|broadcast_)' "${EXISTING_DIRS[@]}" 2>/dev/null || true)

if [ -n "$NO_CALLER_LIST" ]; then
    echo -e "${YELLOW}⚠️  Stream sender(s) with NO caller (production or test):${NC}"
    echo -e "$NO_CALLER_LIST" | sed '/^$/d' | sed 's/^/    /'
fi
if [ -n "$TEST_ONLY_LIST" ]; then
    echo -e "${YELLOW}⚠️  Stream sender(s) called ONLY from tests (no production caller):${NC}"
    echo -e "$TEST_ONLY_LIST" | sed '/^$/d' | sed 's/^/    /'
fi

if [ -z "$NO_CALLER_LIST" ] && [ -z "$TEST_ONLY_LIST" ]; then
    echo -e "${GREEN}✅ Every stream sender has at least one production caller${NC}"
else
    echo -e "${YELLOW}   Wire these to a production call site or delete them. Advisory only — not failing the build.${NC}"
fi

exit 0
