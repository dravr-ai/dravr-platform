#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Compile-free detection of phantom surfaces — declared capabilities
# ABOUTME: with no implementor (Rust traits) and no caller (api-client methods).

# WHY THIS EXISTS
# ---------------
# The dual rule in AGENTS.md: a capability predicate, enum variant, or trait
# method whose only callers are tests is a phantom surface. CI already enforces
# that for the canot messaging surface (supports_*/max_* predicates,
# MessageContent variants). It did not cover the two cheapest cases:
#
#   1. A Rust trait declared with ZERO implementors anywhere in the workspace.
#      Recurrence: pierre-chat-pipeline's QuotaGate and UsageRecorder — both
#      have live call sites in the pipeline (`if let Some(gate) = ...`) and
#      doc comments describing behaviour in the present tense, but nothing has
#      ever implemented them, so the branches are unreachable and the real
#      quota enforcement lives elsewhere (pierre-services/src/usage_counter.rs).
#   2. A @pierre/api-client domain method with ZERO production call sites.
#      Recurrence: authApi.refreshToken(), which posts grant_type=refresh_token
#      to an endpoint whose handler rejects that grant — dormant because no
#      caller has ever exercised it.
#
# Both were found by a manual cold read months after they were written. Neither
# is visible to a regression test: nothing broke, because nothing ran.
#
# MODES
#   check-phantom-surfaces.sh              — report the standing stock, exit 0
#   check-phantom-surfaces.sh <BASE_REF>   — additionally FAIL when the diff
#                                            against BASE_REF introduces a NEW
#                                            phantom surface
#
# The diff-scoped mode is the gate: it stops the stock from growing at the
# moment of authoring, which is the only moment the author has the context to
# fix it. Clearing the existing stock is a deletion decision per surface (some
# are superseded scaffolding, some are unfinished features) and is tracked in
# the dravr-ai/carnet register (carnet#17), not blessed by a list in this script.

set -uo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"
cd "$PROJECT_ROOT" || exit 1

BASE_REF="${1:-}"
FAILED=false

echo -e "${BLUE}==== Phantom Surface Detection (static) ====${NC}"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# ---------------------------------------------------------------------------
# Scan 1: Rust traits with zero implementors
# ---------------------------------------------------------------------------
# Declared traits. `^\s*(pub…)?trait Name` only matches a declaration site —
# `-> impl Trait`, `dyn Trait` and `where T: Trait` carry no `trait` keyword,
# and doc comments are excluded by anchoring the optional visibility modifier.
grep -rhoE '^[[:space:]]*(pub([[:space:]]*\([^)]*\))?[[:space:]]+)?(unsafe[[:space:]]+)?trait[[:space:]]+[A-Za-z0-9_]+' \
    crates/*/src --include='*.rs' 2>/dev/null \
    | grep -oE '[A-Za-z0-9_]+$' | sort -u > "$TMP/declared.txt"

# Implemented traits: the trait path in `impl [<generics>] Path[<args>] for`.
# Take the last `::` segment so `impl pierre_x::Foo for Bar` counts for `Foo`.
grep -rhoE '^[[:space:]]*impl([[:space:]]*<[^>]*>)?[[:space:]]+[A-Za-z0-9_:]+([[:space:]]*<[^>]*>)?[[:space:]]+for[[:space:]]' \
    crates --include='*.rs' 2>/dev/null \
    | sed -E 's/[[:space:]]*for[[:space:]]*$//' \
    | grep -oE '[A-Za-z0-9_:]+([[:space:]]*<[^>]*>)?$' \
    | sed -E 's/[[:space:]]*<.*//; s/.*:://' \
    | sort -u > "$TMP/implemented.txt"

comm -23 "$TMP/declared.txt" "$TMP/implemented.txt" > "$TMP/orphan_traits.txt"

DECLARED_N=$(grep -c . < "$TMP/declared.txt" || true)
ORPHAN_TRAIT_N=$(grep -c . < "$TMP/orphan_traits.txt" || true)

if [[ "$DECLARED_N" -eq 0 ]]; then
    echo -e "${RED}❌ Parsed zero trait declarations — this scan is stale.${NC}"
    FAILED=true
elif [[ "$ORPHAN_TRAIT_N" -eq 0 ]]; then
    echo -e "${GREEN}✅ Rust traits: all ${DECLARED_N} declared traits have an implementor.${NC}"
else
    echo -e "${YELLOW}⚠️  Rust traits with zero implementors (${ORPHAN_TRAIT_N} of ${DECLARED_N}):${NC}"
    while read -r t; do
        [[ -z "$t" ]] && continue
        SITE=$(grep -rnE "trait[[:space:]]+${t}\b" crates/*/src --include='*.rs' 2>/dev/null | head -1 | cut -d: -f1,2)
        echo "   $t  ($SITE)"
    done < "$TMP/orphan_traits.txt"
fi

# ---------------------------------------------------------------------------
# Scan 2: @pierre/api-client domain methods with zero production call sites
# ---------------------------------------------------------------------------
DOMAINS="packages/api-client/src/domains"

if [[ ! -d "$DOMAINS" ]]; then
    echo -e "${RED}❌ ${DOMAINS} not found — this scan is stale.${NC}"
    FAILED=true
else
    grep -rhoE '^[[:space:]]{4}async [a-zA-Z0-9_]+\(' "$DOMAINS"/*.ts 2>/dev/null \
        | sed -E 's/^[[:space:]]*async //; s/\($//' | sort -u > "$TMP/methods.txt"

    # Production consumers only: both apps, the shared packages and the SDK,
    # excluding test files and the definition files themselves. A method whose
    # only callers are tests is precisely what the dual rule calls phantom.
    find frontend/src frontend-mobile/src packages/*/src sdk/src \
        \( -name '*.ts' -o -name '*.tsx' \) 2>/dev/null \
        | grep -vE '__tests__|\.test\.|\.spec\.' \
        | grep -v "^${DOMAINS}/" > "$TMP/prod_files.txt"

    PROD_N=$(grep -c . < "$TMP/prod_files.txt" || true)
    METHOD_N=$(grep -c . < "$TMP/methods.txt" || true)

    if [[ "$PROD_N" -eq 0 || "$METHOD_N" -eq 0 ]]; then
        echo -e "${RED}❌ Parsed zero api-client methods or zero production files — this scan is stale.${NC}"
        FAILED=true
    else
        # One pass: every method name that appears as a whole word anywhere in
        # production sources. Matching the bare identifier (not `.name(`) keeps
        # by-reference uses — `queryFn: api.user.getProfile` — out of the
        # report. That costs sensitivity for names that collide with local
        # variables, and buys a report with no false positives.
        # NUL-delimited xargs from stdin: `xargs -a` is GNU-only and BSD xargs
        # rejects it, which silently produced an empty match set on macOS and
        # reported every method as an orphan.
        tr '\n' '\0' < "$TMP/prod_files.txt" \
            | xargs -0 grep -ohwFf "$TMP/methods.txt" 2>/dev/null \
            | sort -u > "$TMP/used.txt"
        comm -23 "$TMP/methods.txt" "$TMP/used.txt" > "$TMP/orphan_methods.txt"

        USED_N=$(grep -c . < "$TMP/used.txt" || true)
        ORPHAN_METHOD_N=$(grep -c . < "$TMP/orphan_methods.txt" || true)

        # Completeness tripwire: both apps call login/logout/getStatus and many
        # more, so a near-empty match set means the scan itself broke, not that
        # the codebase calls nothing. Fail loudly rather than emit a wall of
        # false orphans that reads like a real finding.
        if [[ "$USED_N" -lt $(( METHOD_N / 4 )) ]]; then
            echo -e "${RED}❌ Usage scan implausible: only ${USED_N} of ${METHOD_N} methods matched in ${PROD_N} files.${NC}"
            echo -e "${YELLOW}   The scan machinery is broken (grep/xargs behaviour), not the codebase.${NC}"
            FAILED=true
            ORPHAN_METHOD_N=-1
        fi
        if [[ "$ORPHAN_METHOD_N" -eq -1 ]]; then
            : # scan broken; already reported
        elif [[ "$ORPHAN_METHOD_N" -eq 0 ]]; then
            echo -e "${GREEN}✅ api-client: all ${METHOD_N} domain methods have a production caller.${NC}"
        else
            echo -e "${YELLOW}⚠️  api-client methods with zero production callers (${ORPHAN_METHOD_N} of ${METHOD_N}):${NC}"
            while read -r m; do
                [[ -z "$m" ]] && continue
                SITE=$(grep -rnE "async ${m}\(" "$DOMAINS"/*.ts 2>/dev/null | head -1 | cut -d: -f1,2)
                echo "   $m  ($SITE)"
            done < "$TMP/orphan_methods.txt"
        fi
    fi
fi

# ---------------------------------------------------------------------------
# Gate: fail when THIS change introduces a new phantom surface
# ---------------------------------------------------------------------------
if [[ -n "$BASE_REF" ]]; then
    echo ""
    echo -e "${BLUE}---- New phantom surfaces in this change (vs ${BASE_REF}) ----${NC}"

    ADDED="$(git diff -U0 "${BASE_REF}...HEAD" -- 'crates/*/src/*.rs' "$DOMAINS" 2>/dev/null \
        | grep '^+' | grep -v '^+++' || true)"

    NEW_FOUND=false
    while read -r t; do
        [[ -z "$t" ]] && continue
        if printf '%s\n' "$ADDED" | grep -qE "trait[[:space:]]+${t}\b"; then
            echo -e "${RED}❌ New trait with no implementor: ${t}${NC}"
            NEW_FOUND=true
        fi
    done < "$TMP/orphan_traits.txt"

    if [[ -f "$TMP/orphan_methods.txt" ]]; then
        while read -r m; do
            [[ -z "$m" ]] && continue
            if printf '%s\n' "$ADDED" | grep -qE "async ${m}\("; then
                echo -e "${RED}❌ New api-client method with no production caller: ${m}${NC}"
                NEW_FOUND=true
            fi
        done < "$TMP/orphan_methods.txt"
    fi

    if [[ "$NEW_FOUND" == "true" ]]; then
        echo ""
        echo -e "${RED}A capability that nothing implements or calls is a phantom surface.${NC}"
        echo -e "${YELLOW}Wire a production consumer in this same change, or register the gap with a${NC}"
        echo -e "${YELLOW}LIMITATION(registre#issue): marker naming the item. Do not ship it bare —${NC}"
        echo -e "${YELLOW}nothing breaks when a phantom is wrong, so no test will ever find it.${NC}"
        FAILED=true
    else
        echo -e "${GREEN}✅ This change adds no unimplemented trait and no uncalled api-client method.${NC}"
    fi
fi

if [[ "$FAILED" == "true" ]]; then
    echo -e "${RED}❌ PHANTOM SURFACE CHECK FAILED${NC}"
    exit 1
fi
exit 0
