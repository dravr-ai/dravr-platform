#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Guard G1 — each athlete-data table is written from one repository file, its wellness writes called from one store
# ABOUTME: Athlete-data INSERT/UPDATE outside its owner fails; uploaded files are written and keyed only by the originals store
#
# A day's wellness was written by two sets of statements on the same keys —
# the health sync replacing whole rows, the Garmin push merging reading by
# reading — and cached_activities by two INSERTs and an extra UPDATE, each
# with its own rule for a missing value (Athlete Data Consolidation design,
# D2/D3; carnet#1305). Each table now has one owning repository file, and
# the wellness repository writes are called from one service file, the
# wellness store. This gate keeps it that way:
#
#   1. For every table below, the production files that INSERT INTO it or
#      UPDATE it are exactly its owning file. The statement is matched in any
#      case and across any whitespace, line breaks included. A soft delete is
#      not a write of readings and is recognised by its shape, whatever it is
#      named and wherever it lives: `UPDATE <table> SET deleted_at = <bind>
#      WHERE`, with deleted_at the only column it sets.
#   2. The repository wellness write methods (upsert_sleep_session,
#      upsert_recovery_metrics, upsert_health_snapshot, upsert_daily_activity,
#      upsert_data_source,
#      clear_provider_recovery_scores, clear_provider_sleep_scores, and the
#      series repository's insert / insert_batch) are called, in method or
#      UFCS spelling, only from crates/pierre-services/src/wellness_store.rs
#      (and, delegating to themselves, from their own repository file).
#   3. Uploaded files have one store (S12): the upload table's repository
#      writes and drain (record_uploaded_file, record_uploaded_file_bytes,
#      delete_uploaded_file, pending_upload_bytes, clear_drained_bytes) are
#      called only from crates/pierre-providers/src/originals.rs (and,
#      delegating to themselves, from the upload table's own repository
#      file, no other owner's), and the blob key of an
#      upload (`BlobKey::upload(`) is built only there, so no other code
#      reads, writes or erases an uploaded file around the store's checks.
#
# synthetic_activities is
# fixture-only data read by the dev fixture, not athlete data, and is not
# listed. A file's trailing #[cfg(test)] module is not production code
# (ADR-027) and is not scanned, through prod-src-scan.sh.
#
# Usage: check-one-writer.sh [ROOT]   (ROOT defaults to the repo root)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${1:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
SCAN="${PROD_SRC_SCAN:-$SCRIPT_DIR/prod-src-scan.sh}"
STORE="crates/pierre-services/src/wellness_store.rs"
ORIGINALS="crates/pierre-providers/src/originals.rs"
# The upload table's repository file: the one owner file exempt from rule 3,
# since its methods are the ones the rule guards.
UPLOAD_OWNER="crates/pierre-database/src/repositories/uploaded_activity_files.rs"
HEALTH="crates/pierre-database/src/repositories/health_persistence.rs"

# table owning-file
OWNERS=(
    "sleep_sessions $HEALTH"
    "recovery_metrics $HEALTH"
    "health_snapshots $HEALTH"
    "daily_activity $HEALTH"
    "data_sources $HEALTH"
    "cached_activities crates/pierre-database/src/repositories/activity_cache/sql.rs"
    "data_point_series crates/pierre-database/src/repositories/time_series.rs"
    "activity_streams crates/pierre-database/src/repositories/activity_streams.rs"
    "uploaded_activity_files crates/pierre-database/src/repositories/uploaded_activity_files.rs"
)

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}==== One writer per athlete-data table (G1) ====${NC}"

cd "$ROOT"
shopt -s nullglob
SRC_DIRS=(crates/*/src)
if [ "${#SRC_DIRS[@]}" -eq 0 ]; then
    echo -e "${RED}❌ No crates/*/src under $ROOT: the scan would pass on nothing${NC}" >&2
    exit 2
fi
if [ ! -f "$STORE" ]; then
    echo -e "${RED}❌ $STORE is missing: the one wellness writer moved without this gate${NC}" >&2
    exit 2
fi
if [ ! -f "$ORIGINALS" ]; then
    echo -e "${RED}❌ $ORIGINALS is missing: the one store of uploaded files moved without this gate${NC}" >&2
    exit 2
fi

# Whether the statement starting at FILE:LINE:COLUMN is a soft delete of
# TABLE: its text from that column over the next lines, Rust line
# continuations dropped and whitespace squeezed, reads
# `UPDATE <table> SET deleted_at = $n|? WHERE`.
is_soft_delete() {
    local file="$1" line="$2" column="$3" table="$4" text
    text="$(sed -n "${line},$((line + 5))p" "$file" \
        | sed 's/\\[[:space:]]*$//' | tr '\n\t' '  ' \
        | cut -c"${column}"- | tr -s ' ')"
    printf '%s\n' "$text" \
        | grep -Eiq "^UPDATE ${table} SET deleted_at = (\\\$[0-9]+|\\?) WHERE( |$)"
}

failed=0
for entry in "${OWNERS[@]}"; do
    table="${entry%% *}"
    owner="${entry#* }"
    if [ ! -f "$owner" ]; then
        echo -e "${RED}❌ $owner, the writer of $table, is missing: it moved without this gate${NC}" >&2
        exit 2
    fi
    # prod-src-scan.sh exits 2 on a crash; set -e propagates it, never an empty list.
    # --vimgrep with -U: one `path:line:column:text` per statement, at the
    # line it starts on, however many lines it spans.
    hits="$(bash "$SCAN" "(?i)(INSERT\\s+INTO\\s+${table}([^_a-z0-9]|\$)|UPDATE\\s+${table}\\b)" \
        -U --vimgrep "${SRC_DIRS[@]}")"
    writers=""
    while IFS= read -r hit; do
        [ -z "$hit" ] && continue
        file="${hit%%:*}"
        rest="${hit#*:}"
        line="${rest%%:*}"
        rest="${rest#*:}"
        column="${rest%%:*}"
        if is_soft_delete "$file" "$line" "$column" "$table"; then
            continue
        fi
        writers="$(printf '%s\n%s' "$writers" "$file")"
    done <<< "$hits"
    files="$(printf '%s\n' "$writers" | sed '/^$/d' | sort -u)"
    if [ "$files" != "$owner" ]; then
        echo -e "${RED}❌ $table is written from $(printf '%s\n' "$files" | grep -c . || true) file(s); its one writer is $owner:${NC}"
        printf '%s\n' "$hits" | sed '/^$/d; s/^/    /'
        failed=1
    fi
done

CALLS='(\.|::)\s*(upsert_sleep_session|upsert_recovery_metrics|upsert_health_snapshot|upsert_daily_activity|upsert_data_source|clear_provider_recovery_scores|clear_provider_sleep_scores|insert_batch)\s*\(|time_series_points\s*\.\s*insert\s*\('
calls="$(bash "$SCAN" "$CALLS" -U --vimgrep "${SRC_DIRS[@]}")"
# A repository delegating to its own write (time_series.rs's insert calling
# its insert_batch) is the owner writing, not a second caller.
OWNER_FILES="$(for entry in "${OWNERS[@]}"; do printf '%s:\n' "${entry#* }"; done | sort -u)"
outside="$(printf '%s\n' "$calls" | sed '/^$/d' | grep -v "^$STORE:" \
    | grep -vF "$OWNER_FILES" || true)"
inside="$(printf '%s\n' "$calls" | grep -c "^$STORE:" || true)"
if [ "$inside" -eq 0 ]; then
    echo -e "${RED}❌ $STORE calls no repository wellness write: the writer the gate guards is gone${NC}" >&2
    exit 2
fi
if [ -n "$outside" ]; then
    echo -e "${RED}❌ Repository wellness writes called outside $STORE:${NC}"
    printf '%s\n' "$outside" | sed 's/^/    /'
    echo "   Write through pierre_services::wellness_store::WellnessStore, naming the write mode."
    failed=1
fi

# 3. Uploaded files: the table's writes and drain, and the upload blob key,
# only in the originals store (and, delegating to themselves, their own
# repository file).
UPLOAD_CALLS='(\.|::)\s*(record_uploaded_file|record_uploaded_file_bytes|delete_uploaded_file|pending_upload_bytes|clear_drained_bytes)\s*\(|BlobKey\s*::\s*upload\s*\('
upload_calls="$(bash "$SCAN" "$UPLOAD_CALLS" -U --vimgrep "${SRC_DIRS[@]}")"
upload_inside="$(printf '%s\n' "$upload_calls" | grep -c "^$ORIGINALS:" || true)"
if [ "$upload_inside" -eq 0 ]; then
    echo -e "${RED}❌ $ORIGINALS keys and writes no uploaded file: the store the gate guards is gone${NC}" >&2
    exit 2
fi
upload_outside="$(printf '%s\n' "$upload_calls" | sed '/^$/d' | grep -v "^$ORIGINALS:" \
    | grep -v "^$UPLOAD_OWNER:" || true)"
if [ -n "$upload_outside" ]; then
    echo -e "${RED}❌ Uploaded files written, drained or keyed outside $ORIGINALS:${NC}"
    printf '%s\n' "$upload_outside" | sed 's/^/    /'
    echo "   Keep and read uploaded files through pierre_providers::originals::OriginalsStore."
    failed=1
fi

if [ "$failed" -ne 0 ]; then
    exit 1
fi
echo -e "${GREEN}✅ ${#OWNERS[@]} athlete-data tables each written from one file; wellness writes called only from $STORE ($inside site(s)); uploaded files only through $ORIGINALS ($upload_inside site(s))${NC}"
