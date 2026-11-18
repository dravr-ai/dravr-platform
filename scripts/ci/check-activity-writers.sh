#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Guard — every provider's activities reach cached_activities only through the activity store's projection
# ABOUTME: A cache write or row delete outside the store's sink, a by-date DELETE, or a place or declared-home call outside its caller fails
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
#   4. A row's place (slice 6, carnet#1484) and the athlete's declared home
#      (carnet#1593) have one caller: `replace_clustering`,
#      `create_athlete_place`, `home_declaration` (the one read of the
#      declared home, which is never served) and `store_home_declaration`
#      are called only by the places service (pierre-services
#      athlete_places.rs), besides the repository that defines them and the
#      test fixtures. A caller that calls one of them no more fails closed, as
#      a sink that writes nothing does: the call it guarded moved, and the
#      gate would pass on nothing.
#   5. The declared home's table, `athlete_declared_home`, is named in
#      production code only by the places repository and the three per-user
#      lists (the account wipe, the references list, the athlete's export),
#      so no other repository reads it behind rule 4's back. A places
#      repository that no longer names it fails closed.
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
PLACES_REPOSITORY="crates/pierre-database/src/repositories/athlete_places.rs"
# method|the one production file that may call it
PLACE_CALLS=(
    "replace_clustering|crates/pierre-services/src/athlete_places.rs"
    "create_athlete_place|crates/pierre-services/src/athlete_places.rs"
    "home_declaration|crates/pierre-services/src/athlete_places.rs"
    "store_home_declaration|crates/pierre-services/src/athlete_places.rs"
)
DECLARED_HOME_TABLE="athlete_declared_home"
# The production files that may name the declared home's table.
DECLARED_HOME_NAMERS=(
    "$PLACES_REPOSITORY"
    "crates/pierre-database/src/repositories/account_reset.rs"
    "crates/pierre-database/src/repositories/user_references.rs"
    "crates/pierre-database/src/repositories/user_export.rs"
)

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

# 4. The place writes and the declared home's calls, each from its one caller.
place_sites=0
for entry in "${PLACE_CALLS[@]}"; do
    method="${entry%%|*}"
    owner="${entry#*|}"
    found="$(bash "$SCAN" "(\.|::)\s*${method}\s*\(" -U --vimgrep "${SRC_DIRS[@]}")"
    owned="$(printf '%s\n' "$found" | grep -c "^$owner:" || true)"
    if [ "$owned" -eq 0 ]; then
        echo -e "${RED}❌ $owner calls no $method: the place call this gate guards moved${NC}" >&2
        exit 2
    fi
    place_sites=$((place_sites + owned))
    stray="$(printf '%s\n' "$found" | sed '/^$/d' \
        | grep -v "^$owner:" | grep -v "^$PLACES_REPOSITORY:" | grep -v "^$TEST_SUPPORT" || true)"
    if [ -n "$stray" ]; then
        echo -e "${RED}❌ $method called outside $owner:${NC}"
        printf '%s\n' "$stray" | sed 's/^/    /'
        echo "   A row's place and the declared home have one caller (carnet#1484, carnet#1593);"
        echo "   call it there. The declared home is never served."
        failed=1
    fi
done

# 5. The declared home's table, named only where it is read, wiped, listed
# or exported.
named="$(bash "$SCAN" "\b${DECLARED_HOME_TABLE}\b" --vimgrep "${SRC_DIRS[@]}")"
if ! printf '%s\n' "$named" | grep -q "^$PLACES_REPOSITORY:"; then
    echo -e "${RED}❌ $PLACES_REPOSITORY names no $DECLARED_HOME_TABLE: the table this gate guards moved${NC}" >&2
    exit 2
fi
stray_names="$(printf '%s\n' "$named" | sed '/^$/d')"
for namer in "${DECLARED_HOME_NAMERS[@]}"; do
    stray_names="$(printf '%s\n' "$stray_names" | grep -v "^$namer:" || true)"
done
if [ -n "$stray_names" ]; then
    echo -e "${RED}❌ $DECLARED_HOME_TABLE named outside its repository and the per-user lists:${NC}"
    printf '%s\n' "$stray_names" | sed 's/^/    /'
    echo "   The declared home has one reader, the places recompute, through"
    echo "   AthletePlaceRepository::home_declaration (carnet#1593)."
    failed=1
fi

if [ "$failed" -ne 0 ]; then
    exit 1
fi
echo -e "${GREEN}✅ cached_activities written and deleted only by the activity store's sink ($in_sink write site(s)), its repository and the test fixtures; no by-date prune; places written and the declared home called only by their one caller ($place_sites site(s)); its table named only by its repository and the per-user lists${NC}"
