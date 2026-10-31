#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Guard G3 — the wellness merge and the repository wellness reads are called in one file only
# ABOUTME: merge_sleep_sessions / merge_recovery_metrics / merge_health_metrics and the get_* reads live in stored_health.rs
#
# Every sleep, recovery and body reader once merged rows itself: six call
# sites, each with its own nap filter, source narrowing and night dating,
# so two tools could answer the same night two ways (Athlete Data
# Consolidation design, D6; carnet#1281). The one merge now runs inside
# crates/pierre-services/src/stored_health.rs, and every reader takes its
# merged_* reads. This gate keeps it one: production code anywhere else that
# names dravr-equilibre's merge_* functions, calls the repository reads the
# chokepoint wraps (method or UFCS spelling), re-spells one of the two
# divergences the chokepoint unified (night dating by the UTC date of the
# end, HRV as hrv_rmssd.or(hrv_ms)), or selects from a wellness table
# outside the repository layer (crates/pierre-database), fails. A file's
# trailing #[cfg(test)] module is not production code (ADR-027) and is not
# scanned, through prod-src-scan.sh.
#
# Usage: check-one-wellness-merge.sh [ROOT]   (ROOT defaults to the repo root)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${1:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
SCAN="${PROD_SRC_SCAN:-$SCRIPT_DIR/prod-src-scan.sh}"
CHOKEPOINT="crates/pierre-services/src/stored_health.rs"

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}==== One wellness merge site (G3) ====${NC}"

cd "$ROOT"
shopt -s nullglob
SRC_DIRS=(crates/*/src)
if [ "${#SRC_DIRS[@]}" -eq 0 ]; then
    echo -e "${RED}❌ No crates/*/src under $ROOT: the scan would pass on nothing${NC}" >&2
    exit 2
fi
if [ ! -f "$CHOKEPOINT" ]; then
    echo -e "${RED}❌ $CHOKEPOINT is missing: the one merge site moved without this gate${NC}" >&2
    exit 2
fi

PATTERN='\bmerge_(sleep_sessions|recovery_metrics|health_metrics|daily_activity)\b|(\.|::)get_(sleep_sessions|recovery_metrics|health_snapshots)\(|(\.|::)(sleep|recovery)_origins\(|end_datetime\.date_naive\(\)|hrv_(rmssd|ms)\.or\('
# The repository layer owns the SQL; anywhere else a wellness table is read raw.
SQL_PATTERN='\bFROM (sleep_sessions|recovery_metrics|health_snapshots)\b'
REPOSITORY_LAYER="crates/pierre-database/src/"

# prod-src-scan.sh exits 2 on a crash; set -e propagates it, never an empty list.
hits="$(bash "$SCAN" "$PATTERN" "${SRC_DIRS[@]}")"
sql_hits="$(bash "$SCAN" "$SQL_PATTERN" "${SRC_DIRS[@]}")"
outside="$(
    {
        printf '%s\n' "$hits" | grep -v "^$CHOKEPOINT:"
        printf '%s\n' "$sql_hits" | grep -v "^$REPOSITORY_LAYER"
    } | sed '/^$/d' || true
)"
inside="$(printf '%s\n' "$hits" | grep -c "^$CHOKEPOINT:" || true)"

if [ "$inside" -eq 0 ]; then
    echo -e "${RED}❌ $CHOKEPOINT neither merges nor reads: the chokepoint the gate guards is gone${NC}" >&2
    exit 2
fi

if [ -n "$outside" ]; then
    echo -e "${RED}❌ Wellness rows merged or read outside $CHOKEPOINT:${NC}"
    printf '%s\n' "$outside" | sed 's/^/    /'
    echo "   Read them through stored_health::merged_sleep / merged_recovery / merged_body"
    echo "   with a SourceFilter instead of merging or reading the repository yourself."
    exit 1
fi

echo -e "${GREEN}✅ The wellness merge and its repository reads are called only in $CHOKEPOINT ($inside site(s))${NC}"
