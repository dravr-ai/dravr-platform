#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Guard G4 — one precedence: no provider-priority table, and canonical_order reached only through rank_activity_copy
# ABOUTME: dravr-equilibre's precedence::rank decides which source's copy stands; pierre-providers classifies rows into sources
#
# Eight rules once decided which source won a night, a day or a workout:
# ProviderPriority, its recovery_priority, DevicePriority/resolve_duplicate,
# the activity merge's own GPS-first canonical order, the Strava override of
# the by-id read, an alphabetical sleep source, a hard-coded [GARMIN, WHOOP]
# roster rule... so the sleep merge said WHOOP while the agent card said
# Garmin (Athlete Data Consolidation design, D4). One rule decides now,
# dravr-equilibre's precedence::rank, over sources the one classifier
# (crates/pierre-providers/src/precedence.rs) reads off the stored rows.
#
# This gate keeps it one: production code that names a retired priority
# table fails, and the activity merge's tie-break, canonical_order, is
# named only in crates/pierre-providers/src/deduplication/detection.rs and
# there exactly twice — its definition and its one call, inside
# rank_activity_copy, after the rank. After the sort, one rule may still move
# a group's member: DECISION 1's edit on Strava (lift_edited_strava_copy). A
# second reorder (a native-twin lift once overrode the rank this way) fails:
# `members.insert(` is named exactly once in the merge. A file's trailing
# #[cfg(test)] module is not production code (ADR-027), through
# prod-src-scan.sh.
#
# Usage: check-one-precedence.sh [ROOT]   (ROOT defaults to the repo root)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${1:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
SCAN="${PROD_SRC_SCAN:-$SCRIPT_DIR/prod-src-scan.sh}"
MERGE="crates/pierre-providers/src/deduplication/detection.rs"

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}==== One precedence (G4) ====${NC}"

cd "$ROOT"
shopt -s nullglob
SRC_DIRS=(crates/*/src)
if [ "${#SRC_DIRS[@]}" -eq 0 ]; then
    echo -e "${RED}❌ No crates/*/src under $ROOT: the scan would pass on nothing${NC}" >&2
    exit 2
fi
if [ ! -f "$MERGE" ]; then
    echo -e "${RED}❌ $MERGE is missing: the activity merge moved without this gate${NC}" >&2
    exit 2
fi

RETIRED='\b(ProviderPriority|recovery_priority|DevicePriority|resolve_duplicate)\b'
TIE_BREAK='\bcanonical_order\('

# prod-src-scan.sh exits 2 on a crash; set -e propagates it, never an empty list.
retired="$(bash "$SCAN" "$RETIRED" "${SRC_DIRS[@]}")"
tie_breaks="$(bash "$SCAN" "$TIE_BREAK" "${SRC_DIRS[@]}")"
elsewhere="$(printf '%s\n' "$tie_breaks" | grep -v "^$MERGE:" | sed '/^$/d' || true)"
inside="$(printf '%s\n' "$tie_breaks" | grep -c "^$MERGE:" || true)"
rank_site="$(bash "$SCAN" 'rank\(Kind::Activity' "$MERGE")"
reorders="$(bash "$SCAN" '\bmembers\.insert\(' "$MERGE")"
reorder_count="$(printf '%s\n' "$reorders" | sed '/^$/d' | wc -l | tr -d ' ')"

failed=false
if [ -n "$retired" ]; then
    echo -e "${RED}❌ A retired priority table is named:${NC}"
    printf '%s\n' "$retired" | sed 's/^/    /'
    echo "   Rank sources with dravr_equilibre::precedence::rank (pierre_providers::precedence)."
    failed=true
fi
if [ -n "$elsewhere" ]; then
    echo -e "${RED}❌ canonical_order named outside $MERGE:${NC}"
    printf '%s\n' "$elsewhere" | sed 's/^/    /'
    failed=true
fi
if [ "$inside" -ne 2 ]; then
    echo -e "${RED}❌ canonical_order is named $inside time(s) in $MERGE; exactly 2 (its definition and its call in rank_activity_copy):${NC}"
    printf '%s\n' "$tie_breaks" | grep "^$MERGE:" | sed 's/^/    /' || true
    failed=true
fi
if [ "$reorder_count" -ne 1 ]; then
    echo -e "${RED}❌ A group's members are moved after the rank $reorder_count time(s) in $MERGE; exactly 1 (the edit on Strava, lift_edited_strava_copy):${NC}"
    printf '%s\n' "$reorders" | sed '/^$/d; s/^/    /'
    echo "   Order copies by rank_activity_copy alone; a second reorder is a second precedence rule."
    failed=true
fi
if [ -z "$rank_site" ]; then
    echo -e "${RED}❌ $MERGE no longer ranks a workout's copies with rank(Kind::Activity, …)${NC}" >&2
    exit 2
fi
if [ "$failed" = true ]; then
    exit 1
fi

echo -e "${GREEN}✅ One precedence: no retired priority table, canonical_order only behind rank_activity_copy${NC}"
