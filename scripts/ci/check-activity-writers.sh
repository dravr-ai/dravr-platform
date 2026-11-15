#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Guard — every provider's activities reach cached_activities only through the activity store's projection
# ABOUTME: A production upsert_activities* or delete_*activit* call outside the store's sink, or any by-date DELETE, fails
#
# Every provider read, push and upload is kept in dravr-barrage, the
# activity store, which projects it into cached_activities through one sink
# (carnet#1482, D3). One retention system expires it, from barrage (D8): no
# code path may age rows out of the cache by date behind its back. This gate
# keeps both true in production code:
#
#   1. `upsert_activities` / `upsert_activities_synced_at` / `amend_activity` are called, in
#      method or UFCS spelling, only from
#        - the activity store's sink (pierre-services activity_cache_sink.rs):
#          the one cache write both projections make, the live sink's and
#          the server's archive sink, which hands its rows to it — a line
#          scan cannot tell two functions of one file apart, so the file is
#          the unit;
#        - the repository that defines them (the trait's default method and
#          the implementation its macro emits for both backends);
#        - crates/pierre-test-support, the test fixtures (ADR-027).
#   2. No `DELETE FROM cached_activities … start_date <` or `… synced_at <`
#      appears anywhere, the repositories included: a by-date prune is a
#      second retention system, and the repository's last one
#      (`prune_activities_before`) went once the store's sweep expired
#      activities.
#   3. The cache's row deletes — `delete_cached_activity`,
#      `delete_provider_activities`, and any `delete_cached_activit*` a
#      repository grows back (the by-window one went once the eviction
#      moved to the store) — are called only where the writes of 1 are: a row deleted behind the
#      store's back leaves its record, which the next projection serves
#      again. A caller drops activities through the store
#      (`LiveActivityStore::discard`, `discard_between`, `delete_source`) or
#      the provider purge, which retract them through the sink.
#
# A file's trailing #[cfg(test)] module is not production code (ADR-027) and
# is not scanned, through prod-src-scan.sh. Test suites under tests/ are not
# under src/ and are not scanned either.
#
# Usage: check-activity-writers.sh [ROOT]   (ROOT defaults to the repo root)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${1:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
SCAN="${PROD_SRC_SCAN:-$SCRIPT_DIR/prod-src-scan.sh}"
SINK="crates/pierre-services/src/activity_cache_sink.rs"
REPOSITORY="crates/pierre-database/src/repositories/activity_cache.rs"
TEST_SUPPORT="crates/pierre-test-support/"

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}==== Activities reach the cache only through the activity store ====${NC}"

cd "$ROOT"
shopt -s nullglob
SRC_DIRS=(crates/*/src)
if [ "${#SRC_DIRS[@]}" -eq 0 ]; then
    echo -e "${RED}❌ No crates/*/src under $ROOT: the scan would pass on nothing${NC}" >&2
    exit 2
fi
for file in "$SINK" "$REPOSITORY"; do
    if [ ! -f "$file" ]; then
        echo -e "${RED}❌ $file is missing: it moved without this gate${NC}" >&2
        exit 2
    fi
done

failed=0

# 1. The cache writes. prod-src-scan.sh exits 2 on a crash; set -e
# propagates it, never an empty list.
CALLS='(\.|::)\s*(upsert_activities(_synced_at)?|amend_activity)\s*\('
calls="$(bash "$SCAN" "$CALLS" -U --vimgrep "${SRC_DIRS[@]}")"
in_sink="$(printf '%s\n' "$calls" | grep -c "^$SINK:" || true)"
if [ "$in_sink" -eq 0 ]; then
    echo -e "${RED}❌ $SINK writes no cached activity: the projection this gate guards moved${NC}" >&2
    exit 2
fi
outside="$(printf '%s\n' "$calls" | sed '/^$/d' \
    | grep -v "^$SINK:" | grep -v "^$REPOSITORY:" | grep -v "^$TEST_SUPPORT" || true)"
if [ -n "$outside" ]; then
    echo -e "${RED}❌ cached_activities written outside the activity store:${NC}"
    printf '%s\n' "$outside" | sed 's/^/    /'
    echo "   Keep activities through pierre_services::live_activities::LiveActivityStore"
    echo "   (ToolRuntime::live_activities()); it projects them into the cache."
    failed=1
fi

# 2. By-date deletes: a DELETE of cached_activities whose statement, up to
# its `;`, bounds start_date or synced_at from above.
PRUNES='(?i)DELETE\s+FROM\s+cached_activities\b[^;]*?\b(start_date|synced_at)\s*<'
prunes="$(bash "$SCAN" "$PRUNES" -U --vimgrep "${SRC_DIRS[@]}")"
prunes_outside="$(printf '%s\n' "$prunes" | sed '/^$/d' || true)"
if [ -n "$prunes_outside" ]; then
    echo -e "${RED}❌ cached_activities pruned by date:${NC}"
    printf '%s\n' "$prunes_outside" | sed 's/^/    /'
    echo "   Retention is the activity store's: one policy, one sweep (carnet#1482, D8)."
    failed=1
fi

# 3. The cache's row deletes, in method or UFCS spelling.
DELETES='(\.|::)\s*(delete_cached_activit\w*|delete_provider_activities)\s*\('
deletes="$(bash "$SCAN" "$DELETES" -U --vimgrep "${SRC_DIRS[@]}")"
deletes_outside="$(printf '%s\n' "$deletes" | sed '/^$/d' \
    | grep -v "^$SINK:" | grep -v "^$REPOSITORY:" | grep -v "^$TEST_SUPPORT" || true)"
if [ -n "$deletes_outside" ]; then
    echo -e "${RED}❌ cached activities deleted outside the activity store:${NC}"
    printf '%s\n' "$deletes_outside" | sed 's/^/    /'
    echo "   Drop them through LiveActivityStore (discard, discard_between, delete_source)"
    echo "   or the provider purge, so the store's records go with the rows."
    failed=1
fi

if [ "$failed" -ne 0 ]; then
    exit 1
fi
echo -e "${GREEN}✅ cached_activities written and deleted only by the activity store's sink ($in_sink write site(s)), its repository and the test fixtures; no by-date prune${NC}"
