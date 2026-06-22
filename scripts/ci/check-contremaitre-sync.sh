#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Compile-free static check that platform code stays in sync with the
# ABOUTME: dravr-contremaitre catalogues (messaging locales + notify events).

# WHY THIS EXISTS
# ---------------
# The integration tests that police platform<->contremaitre coupling
# (contremaitre_test, notify_catalogue_test) live in pierre-server/tests and run
# ONLY in the full suite (cron SQLite shards + full-on-main ci-postgres +
# coverage). Because features merge via local squash with no PR, those tests
# first run AFTER the squash is on main — so drift false-greens the branch and
# reds main post-merge. This grep-only check makes the two highest-frequency
# drifts PREVENTIVE at pre-push time, with no Rust compile.
#
# Covered:
#   1. Locale invariant — every messaging-string key ships all 5 locales
#      (fr/en/es/de/pt). Mirrors contremaitre_test
#      test_messaging_registry_seeds_all_compiled_locales (entry == keys * 5).
#   2. Notify catalogue — every info!(target:"notify", event="X") emitted in
#      platform src exists as a key in contremaitre's notify-events.yaml.
#      Mirrors notify_catalogue_test.
#
# NOT covered (on purpose): the EXPECTED_TOOLS list. Tool names are produced by
# name() methods, not string literals at a greppable call site, so a static scan
# cannot enumerate them reliably; that drift stays guarded by the EXPECTED_TOOLS
# full-suite test. See AGENTS.md.

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"
cd "$PROJECT_ROOT"

echo -e "${BLUE}==== Contremaitre Coupling Sync (static) ====${NC}"

FAILED=false

# ---------------------------------------------------------------------------
# Check 1: messaging-string locale invariant (5 locales per key)
# ---------------------------------------------------------------------------
MS="crates/pierre-contremaitre/src/messaging_strings.rs"

if [[ ! -f "$MS" ]]; then
    echo -e "${RED}❌ messaging_strings.rs not found at $MS — this check is stale.${NC}"
    FAILED=true
else
    # Extract the COMPILED_IN (KEY, "locale", VALUE) table, collapse multi-line
    # tuples by joining on spaces, then pull (KEY, locale) pairs. The only quoted
    # literals inside the table are the locale codes, so this is unambiguous.
    BLOCK="$(awk '/const COMPILED_IN/{f=1} f{print} f&&/^\];/{exit}' "$MS")"
    PAIRS="$(printf '%s' "$BLOCK" | tr '\n' ' ' \
        | grep -oE '\(\s*KEY_[A-Z0-9_]+\s*,\s*"(fr|en|es|de|pt)"' \
        | sed -E 's/.*(KEY_[A-Z0-9_]+)[^"]*"(fr|en|es|de|pt)".*/\1 \2/' || true)"

    PAIR_COUNT="$(printf '%s\n' "$PAIRS" | grep -c . || true)"
    if [[ "$PAIR_COUNT" -eq 0 ]]; then
        echo -e "${RED}❌ Could not parse the COMPILED_IN locale table — structure changed; update this check.${NC}"
        FAILED=true
    else
        # Per-key locale audit: any key without exactly the 5 locales is drift.
        MISSING="$(printf '%s\n' "$PAIRS" | awk '
            { have[$1] = have[$1] " " $2 }
            END {
                split("fr en es de pt", want, " ")
                for (k in have)
                    for (i = 1; i <= 5; i++)
                        if (index(have[k], " " want[i]) == 0)
                            print k " missing:" want[i]
            }' | sort)"
        if [[ -n "$MISSING" ]]; then
            KEYS_AFFECTED="$(printf '%s\n' "$MISSING" | awk '{print $1}' | sort -u | grep -c .)"
            echo -e "${RED}❌ Messaging locale drift: ${KEYS_AFFECTED} key(s) do not ship all 5 locales (fr/en/es/de/pt):${NC}"
            printf '%s\n' "$MISSING" | sed 's/^/   /'
            echo -e "${YELLOW}   Add the missing locale const(s) in ${MS} so every key has fr/en/es/de/pt.${NC}"
            echo -e "${YELLOW}   (This is the recurrence behind commits d73eec36f and a60209307.)${NC}"
            FAILED=true
        else
            KEYS="$(printf '%s\n' "$PAIRS" | awk '{print $1}' | sort -u | grep -c . || true)"
            echo -e "${GREEN}✅ Locale invariant: ${KEYS} keys × 5 locales = ${PAIR_COUNT} entries, all complete.${NC}"
        fi
    fi
fi

# ---------------------------------------------------------------------------
# Check 2: notify events emitted in src must be catalogued in notify-events.yaml
# ---------------------------------------------------------------------------
# Resolve the catalogue from the pinned dravr-contremaitre rev (Cargo.lock) via
# cargo metadata; fall back to a sibling checkout; warn-skip if unresolvable so
# an offline machine is not blocked from pushing.
YAML=""
MANIFEST="$(cargo metadata --format-version 1 2>/dev/null \
    | python3 -c 'import json,sys; d=json.load(sys.stdin); print(next((p["manifest_path"] for p in d["packages"] if p["name"]=="dravr-contremaitre"), ""))' 2>/dev/null || true)"
if [[ -n "$MANIFEST" && -f "${MANIFEST%Cargo.toml}schemas/notify-events.yaml" ]]; then
    YAML="${MANIFEST%Cargo.toml}schemas/notify-events.yaml"
elif [[ -f "../dravr-contremaitre/schemas/notify-events.yaml" ]]; then
    YAML="../dravr-contremaitre/schemas/notify-events.yaml"
fi

if [[ -z "$YAML" ]]; then
    echo -e "${YELLOW}⚠️  notify-events.yaml could not be resolved (offline?) — skipping the notify check.${NC}"
else
    CATALOGUED="$(grep -E '^\s*- name:' "$YAML" | sed -E 's/.*- name:[[:space:]]*//' | tr -d ' "' | sort -u)"
    # Emitted events: each .rs under crates/*/src (not tests) that targets
    # "notify"; grab event = "literal" within a window around the target line.
    # Dynamically-built (non-literal) event names are a known blind spot.
    EMITTED="$(grep -rl 'target: "notify"' crates/*/src 2>/dev/null | grep -v '/tests/' \
        | while IFS= read -r f; do
            grep -B2 -A10 'target: "notify"' "$f" \
                | grep -oE 'event[[:space:]]*=[[:space:]]*"[^"]+"' \
                | sed -E 's/.*"([^"]+)".*/\1/'
          done | sort -u || true)"
    DRIFT="$(comm -23 <(printf '%s\n' "$EMITTED" | grep -v '^$') <(printf '%s\n' "$CATALOGUED" | grep -v '^$') || true)"
    if [[ -n "$DRIFT" ]]; then
        echo -e "${RED}❌ Notify catalogue drift: event(s) emitted in src but absent from notify-events.yaml:${NC}"
        printf '%s\n' "$DRIFT" | sed 's/^/   /'
        echo -e "${YELLOW}   Register the event in dravr-contremaitre notify-events.yaml (or reuse an existing one),${NC}"
        echo -e "${YELLOW}   then bump the contremaitre rev — else notify_catalogue_test reds main post-merge.${NC}"
        FAILED=true
    else
        EMIT_COUNT="$(printf '%s\n' "$EMITTED" | grep -c . || true)"
        echo -e "${GREEN}✅ Notify catalogue: all ${EMIT_COUNT} emitted events are catalogued.${NC}"
    fi
fi

if [[ "$FAILED" == "true" ]]; then
    echo -e "${RED}❌ CONTREMAITRE COUPLING CHECK FAILED${NC}"
    echo -e "${RED}Fix the drift above before pushing — these reds otherwise land on main post-merge.${NC}"
    exit 1
fi
echo -e "${GREEN}✅ Contremaitre coupling in sync.${NC}"
exit 0
