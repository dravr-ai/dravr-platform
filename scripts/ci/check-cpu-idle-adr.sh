#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Fails CI when a Cloud Run cpu_idle=false (instance-based billing) lacks an adjacent ADR reference
# ABOUTME: Instance-based billing keeps CPU always-on (costly) — it must be a documented, ADR-justified choice

# Cloud Run `cpu_idle = false` switches a service from request-based to
# instance-based billing: CPU stays allocated between requests, so the
# service bills 24/7 even at zero traffic. That is a deliberate, expensive
# tradeoff (it once drove a ~$210/mo dev bill). It is only acceptable when
# justified by an ADR — anyone setting it must point at the decision record.
#
# This gate finds every real `cpu_idle = false` (or `cpu_throttling = false`)
# assignment in infra/**/*.tf and requires an `ADR-<n>` reference in the
# comment lines immediately above it. Comment lines that merely *mention*
# cpu_idle=false (e.g. cost notes) are ignored — only actual assignments
# are checked.

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"
cd "$PROJECT_ROOT"

echo -e "${BLUE}==== Cloud Run instance-based billing ADR gate ====${NC}"

if [ ! -d infra ]; then
    echo -e "${GREEN}✅ No infra/ directory — nothing to check${NC}"
    exit 0
fi

# How many comment lines above the assignment to scan for an ADR reference.
CONTEXT_LINES=8

FAILED=0

# Find actual assignments (not comments): a line where cpu_idle / cpu_throttling
# is set to false, whose first non-space character is not '#'.
while IFS=: read -r file line _; do
    [ -z "$file" ] && continue
    # Gather the CONTEXT_LINES comment lines above plus the assignment line itself.
    start=$(( line - CONTEXT_LINES ))
    [ "$start" -lt 1 ] && start=1
    window=$(sed -n "${start},${line}p" "$file")
    if echo "$window" | grep -qE 'ADR-[0-9]+'; then
        echo -e "${GREEN}  OK — $file:$line references an ADR${NC}"
    else
        echo -e "${RED}  ❌ $file:$line sets instance-based billing with no ADR reference within ${CONTEXT_LINES} lines above${NC}"
        FAILED=1
    fi
done < <(grep -rnE '^[[:space:]]*(cpu_idle|cpu_throttling)[[:space:]]*=[[:space:]]*false' infra --include='*.tf' 2>/dev/null || true)

if [ "$FAILED" -ne 0 ]; then
    echo -e "${RED}❌ Instance-based Cloud Run billing must cite an ADR (e.g. 'ADR-019') in an adjacent comment.${NC}"
    echo -e "${RED}   cpu_idle=false bills CPU 24/7 — document the decision or switch to request-based billing.${NC}"
    exit 1
fi

echo -e "${GREEN}✅ All instance-based billing settings reference an ADR${NC}"
exit 0
