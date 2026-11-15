#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Guard — the history walk keeps one source order, one live-read order, one Coverage and no history import
# ABOUTME: A spawn in the budgeted read, a history_import, history_keys or coverage_note symbol, a second order or a folded coverage type fails
#
# The history walk (carnet#1482) replaced the history import and folded every
# answer about a served window into one Coverage type. Each of these came back
# once as a second copy beside the new one, so this gate keeps them single:
#
#   1. budgeted_read.rs spawns nothing: the budgeted read answers inside the
#      caller's budget, and work it cannot finish is a durable walk or live
#      read job, never a detached task (`spawn`, `spawn_blocking`,
#      `spawn_local`, whichever path names it).
#   2. No identifier holding `history_import` or `HistoryImport`, or opening
#      with `HISTORY_IMPORT`, in any Rust file under crates/, tests included,
#      and no file named after it. Text inside a string literal is a wire
#      value and passes: `'history_import'` rows an old deploy wrote, which a
#      migration test seeds, and a contremaitre key such as
#      `"hosted.picker.tagHistoryImporting"`, whose constant only ends in
#      `…_HISTORY_IMPORTING`.
#   3. No `history_keys` symbol: which keys hold a provider's history is read
#      from the one source table (`history_store_keys`, beside
#      `history_sources_for`). No identifier holding `coverage_note` either:
#      the note a model reads is Coverage's own, built by `activity_coverage`.
#   4. None of the coverage types the history walk folded into
#      pierre_core::models::Coverage comes back: no struct, enum, type alias,
#      trait or union named `HistoryCoverage`, `VolumeCoverage` or
#      `StatsCoverage` anywhere, and none named `Coverage` outside
#      pierre-core's coverage.rs. Every served window answers in that one
#      Coverage; a type named for something else that happens to contain
#      `Coverage` (`DossierCoverage`, `CoverageGap`) is not a second one.
#   5. One history source order and one live-read order:
#      - `history_source_order`, `history_sources_for`, `history_store_keys`,
#        `walk_source` and the `HistorySource` / `HistorySourceRole` types are
#        defined only in pierre-services training_history_read/source_order.rs;
#      - `ReadPlan` and the `read_plan` that returns it are defined only in
#        pierre-providers backend_resolver.rs, and production code outside it
#        never builds a `ReadPlan` literal or a function returning a bare one;
#      - no production function named `*source_order*`, `*sources_for*`,
#        `*read_order*` or `*store_keys*` is defined anywhere else (a test
#        named after what it pins is not one).
#
# Rules 1 to 4 read every Rust file under crates/, tests included; rule 5's
# production rules drop each file's trailing #[cfg(test)] module
# (prod-src-scan.sh, ADR-027).
#
# Exits 2 — never a pass — when a file or definition the rules anchor on is
# missing, or a scan cannot run. Every pattern is ripgrep's default engine:
# CI installs ripgrep from apt, whose PCRE2 support this gate does not assume.
#
# Usage: check-history-walk-single-source.sh [ROOT]   (ROOT defaults to the repo root)
#        check-history-walk-single-source.sh --self-test

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ "${1:-}" = "--self-test" ]; then
    exec bash "$SCRIPT_DIR/check-history-walk-single-source.test.sh"
fi
ROOT="${1:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
SCAN="${PROD_SRC_SCAN:-$SCRIPT_DIR/prod-src-scan.sh}"

BUDGETED_READ="crates/pierre-tool-runtime/src/activity_fetch/budgeted_read.rs"
SOURCE_ORDER="crates/pierre-services/src/training_history_read/source_order.rs"
RESOLVER="crates/pierre-providers/src/backend_resolver.rs"
COVERAGE="crates/pierre-core/src/models/coverage.rs"

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}==== History walk: one source order, one live-read order, one Coverage, no history import ====${NC}"

cd "$ROOT"
if [ ! -d crates ]; then
    echo -e "${RED}❌ No crates/ under $ROOT: the scan would pass on nothing${NC}" >&2
    exit 2
fi
for file in "$BUDGETED_READ" "$SOURCE_ORDER" "$RESOLVER" "$COVERAGE"; do
    if [ ! -f "$file" ]; then
        echo -e "${RED}❌ $file is missing: it moved without this gate${NC}" >&2
        exit 2
    fi
done

# rg over the tree: exit 1 is "no match" and a clean result, above it the
# scan crashed and the gate fails closed.
scan() {
    local rc=0 out
    out="$(rg -n --with-filename "$@")" || rc=$?
    if [ "$rc" -gt 1 ]; then
        echo -e "${RED}❌ rg failed (exit $rc) scanning: $*${NC}" >&2
        exit 2
    fi
    printf '%s' "$out"
}

# The anchors every order rule is read against. Every scan is assigned
# before it is read, so a crashed one stops the gate under set -e rather than
# reading as an empty match list.
anchor() { # file pattern what
    local found
    found="$(scan "$2" "$1")"
    if [ -z "$found" ]; then
        echo -e "${RED}❌ $1 no longer defines $3: the rule this gate guards moved${NC}" >&2
        exit 2
    fi
}
anchor "$SOURCE_ORDER" '\bfn\s+history_source_order\b' "history_source_order"
anchor "$SOURCE_ORDER" '\bfn\s+history_sources_for\b' "history_sources_for"
anchor "$SOURCE_ORDER" '\bfn\s+history_store_keys\b' "history_store_keys"
anchor "$RESOLVER" '\bstruct\s+ReadPlan\b' "ReadPlan"
anchor "$RESOLVER" '\bfn\s+read_plan\b' "read_plan"
anchor "$COVERAGE" '\bstruct\s+Coverage\b' "Coverage"

failed=0
report() { # title lines remedy...
    local title="$1" lines
    lines="$(printf '%s\n' "$2" | sed '/^$/d')"
    shift 2
    [ -z "$lines" ] && return 0
    echo -e "${RED}❌ $title:${NC}"
    printf '%s\n' "$lines" | sed 's/^/    /'
    for line in "$@"; do echo "   $line"; done
    failed=1
}
outside() { # file-to-drop lines
    printf '%s\n' "$2" | sed '/^$/d' | grep -v "^$1:" || true
}

# 1. Nothing detached from the budgeted read.
spawns="$(scan '\bspawn(_blocking|_local)?\b' "$BUDGETED_READ")"
report "budgeted_read.rs spawns a task" "$spawns" \
    "Work the read cannot finish inside its budget is a durable job (live read or walk)."

# 2. The retired history import, as an identifier or a file name.
# Every line naming it in any case, then each line's code outside its
# string literals: a double-quoted string, and a single-quoted one without
# whitespace or code punctuation (`'history_import'` in a seeded SQL row,
# never a `'a, …, 'b` lifetime list). What is left must hold no identifier
# containing `history_import` or `HistoryImport`, nor one opening with
# `HISTORY_IMPORT`.
candidates="$(scan -g '*.rs' '(?i)history_?import' crates)"
imports="$(printf '%s\n' "$candidates" | sed '/^$/d' | perl -ne '
    my ($location, $code) = /^([^:]+:\d+:)(.*)$/s or next;
    $code =~ s/"(?:[^"\\]|\\.)*"//g;
    $code =~ s/'"'"'[^'"'"'\s,<>&()]{2,}'"'"'//g;
    print if $code =~ /\w*(?:history_import|HistoryImport)\w*|\bHISTORY_IMPORT\w*/;
')"
listed="$(rg --files crates)"
import_files="$(printf '%s\n' "$listed" | grep -iE 'history_?import' || true)"
report "the retired history import is named" "$imports"$'\n'"$import_files" \
    "The history walk replaced it (carnet#1482); a quoted 'history_import' wire value is fine."

# 3. A second table of the keys a provider's history is held under.
keys="$(scan -w -g '*.rs' 'history_keys' crates)"
report "history_keys is named" "$keys" \
    "Read the keys from pierre_services::training_history_read::history_store_keys."
notes="$(scan -g '*.rs' 'coverage_note' crates)"
report "the retired coverage_note is named" "$notes" \
    "The note is Coverage's own: name it after Coverage (activity_coverage, coverage_*)."

# 4. A coverage type the history walk folded into Coverage, come back.
folded="$(scan -g '*.rs' '\b(struct|enum|type|trait|union)\s+(HistoryCoverage|VolumeCoverage|StatsCoverage)\b' crates)"
report "a coverage type folded into pierre_core::models::Coverage is back" "$folded" \
    "Answer the window with pierre_core::models::Coverage (decision B7)."
second="$(scan -g '*.rs' '\b(struct|enum|type|trait|union)\s+Coverage\b' crates)"
report "a second Coverage beside pierre_core::models::Coverage" \
    "$(outside "$COVERAGE" "$second")" \
    "There is one Coverage, in $COVERAGE; import it."

# 5. One history source order, one live-read order.
history_order="$(scan -g '*.rs' '\b(fn\s+(history_source_order|history_sources_for|history_store_keys|walk_source)|(struct|enum)\s+(HistorySource|HistorySourceRole))\b' crates)"
report "the history source order is defined outside $SOURCE_ORDER" \
    "$(outside "$SOURCE_ORDER" "$history_order")" \
    "There is one order the backends holding a provider's history come in."

plan_types="$(scan -g '*.rs' '\b(struct|enum|type)\s+ReadPlan\b' crates)"
report "the live-read order is defined outside $RESOLVER" \
    "$(outside "$RESOLVER" "$plan_types")" \
    "A live read follows backend_resolver::read_plan (carnet#1494)."

shopt -s nullglob
SRC_DIRS=(crates/*/src)
shopt -u nullglob
# A literal or a bare return type; an `impl … ReadPlan {` block is neither.
literals="$(bash "$SCAN" '\bReadPlan\s*\{|->\s*ReadPlan\b' "${SRC_DIRS[@]}")"
built="$(printf '%s\n' "$literals" | grep -vE '\b(impl|for)\s+ReadPlan\s*\{' || true)"
report "a ReadPlan is built outside $RESOLVER" "$(outside "$RESOLVER" "$built")" \
    "Ask backend_resolver::read_plan for the athlete's plan; never assemble one."

orders="$(bash "$SCAN" '\bfn\s+\w*(source_order|sources_for|read_order|store_keys)\w*\b' "${SRC_DIRS[@]}")"
report "a second source or read order is defined" \
    "$(outside "$SOURCE_ORDER" "$(outside "$RESOLVER" "$orders")")" \
    "History reads history_source_order; live reads read_plan. There is no third."

if [ "$failed" -ne 0 ]; then
    exit 1
fi
echo -e "${GREEN}✅ budgeted read spawns nothing; no history import, history_keys or coverage_note; one Coverage and no folded coverage type; one history source order and one live-read order${NC}"
