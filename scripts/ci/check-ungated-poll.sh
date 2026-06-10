#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: WARNS when a mobile React Query refetchInterval poll is not gated by app foreground state
# ABOUTME: Ungated background polls keep the serverless backend warm and defeat scale-to-zero

# A React Query `refetchInterval` fires a network refetch on a timer. On the
# web, React Query pauses interval refetches when the tab loses focus by
# default (visibilitychange). React Native has no such default — without a
# `focusManager` <-> AppState bridge (or a manual AppState gate, or
# `refetchIntervalInBackground: false` paired with focus wiring), a mobile
# poll keeps pinging the backend from a backgrounded app, holding the
# Cloud Run instance warm and defeating scale-to-zero. The mobile /health
# poll (useServerStatus) had exactly this problem before it was AppState-gated.
#
# Distinguishing a network-calling timer from a UI countdown is inherently
# heuristic, so this gate is WARNING-ONLY: it lists mobile hooks that declare
# `refetchInterval` without any AppState / focusManager / refetchIntervalInBackground
# reference in the same file. It never fails the build — it surfaces candidates
# for a human to confirm.

# No `set -e`: advisory gate, always exits 0; rg legitimately exits nonzero
# when a pattern has no matches.
set -uo pipefail

YELLOW='\033[1;33m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"
cd "$PROJECT_ROOT"

echo -e "${BLUE}==== Ungated mobile poll detection (advisory) ====${NC}"

MOBILE_SRC="frontend-mobile/src"
if [ ! -d "$MOBILE_SRC" ]; then
    echo -e "${GREEN}✅ No $MOBILE_SRC — nothing to check${NC}"
    exit 0
fi

# A central focusManager<->AppState bridge gates EVERY refetchInterval poll
# app-wide (React Query pauses interval refetches for unfocused queries, and the
# bridge marks queries unfocused when the app backgrounds). If one exists, no
# per-hook gating is needed — the whole class is handled. Detect it: a file that
# wires AppState into focusManager.setFocused.
if rg -l 'focusManager\.setFocused' "$MOBILE_SRC" 2>/dev/null | while IFS= read -r b; do
    rg -q 'AppState' "$b" && echo "$b"; done | grep -q .; then
    echo -e "${GREEN}✅ Central focusManager<->AppState bridge present — interval polls pause on background app-wide${NC}"
    exit 0
fi

UNGATED=""
COUNT=0

# Files declaring a refetchInterval that do NOT reference any foreground gate.
while IFS= read -r f; do
    [ -z "$f" ] && continue
    if ! rg -q 'AppState|focusManager|refetchIntervalInBackground' "$f" 2>/dev/null; then
        UNGATED="${UNGATED}${f}\n"
        COUNT=$((COUNT + 1))
    fi
done < <(rg -l 'refetchInterval' "$MOBILE_SRC" 2>/dev/null || true)

if [ "$COUNT" -gt 0 ]; then
    echo -e "${YELLOW}⚠️  $COUNT mobile hook(s) with an apparently un-foreground-gated refetchInterval:${NC}"
    echo -e "$UNGATED" | sed '/^$/d' | sed 's/^/    /'
    echo -e "${YELLOW}   Confirm the poll pauses when the app backgrounds (AppState gate, a${NC}"
    echo -e "${YELLOW}   focusManager<->AppState bridge, or refetchIntervalInBackground:false).${NC}"
    echo -e "${YELLOW}   Advisory only — not failing the build.${NC}"
else
    echo -e "${GREEN}✅ No un-foreground-gated mobile refetchInterval polls detected${NC}"
fi

exit 0
