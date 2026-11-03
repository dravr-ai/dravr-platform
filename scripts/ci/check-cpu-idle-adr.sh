#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Fails CI when a Cloud Run cpu_idle=false (instance-based billing) has no adjacent ADR or stated reason
# ABOUTME: Instance-based billing keeps CPU always-on (costly) — it must cite its ADR or say why in a cpu-idle-ok line

# Cloud Run `cpu_idle = false` switches a service from request-based to
# instance-based billing: CPU stays allocated between requests, so the
# service bills 24/7 even at zero traffic. That is a deliberate, expensive
# tradeoff (it once drove a ~$210/mo dev bill). It is only acceptable when
# justified by an ADR — anyone setting it must point at the decision record.
#
# This gate finds every real `cpu_idle = false` (or `cpu_throttling = false`)
# assignment in infra/**/*.tf and requires, in the comment lines immediately
# above it, either an `ADR-<n>` reference to the decision that owns it, or a
# `# cpu-idle-ok: <reason>` line for a service no ADR covers (a cost measurement,
# say). The reason has to be there: an empty `cpu-idle-ok:` fails. Citing an
# unrelated ADR only to pass this gate is what the escape hatch replaces.
# Comment lines that merely *mention* cpu_idle=false (e.g. cost notes) are
# ignored — only actual assignments are checked.
#
# Usage: check-cpu-idle-adr.sh [root] — root defaults to the repository.

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="${1:-$( cd "$SCRIPT_DIR/../.." && pwd )}"
cd "$PROJECT_ROOT"

echo -e "${BLUE}==== Cloud Run instance-based billing ADR gate ====${NC}"

# Fail closed: a root with no infra/ is a mistyped root or a broken checkout,
# never a pass.
if [ ! -d infra ]; then
    echo -e "${RED}❌ No infra/ directory under $PROJECT_ROOT — nothing was checked${NC}"
    exit 1
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
    elif echo "$window" | grep -qE '^[[:space:]]*#[[:space:]]*cpu-idle-ok:[[:space:]]*[^[:space:]]'; then
        echo -e "${GREEN}  OK — $file:$line states its reason (cpu-idle-ok)${NC}"
    else
        echo -e "${RED}  ❌ $file:$line sets instance-based billing with no ADR reference or cpu-idle-ok reason within ${CONTEXT_LINES} lines above${NC}"
        FAILED=1
    fi
done < <(grep -rnE '^[[:space:]]*(cpu_idle|cpu_throttling)[[:space:]]*=[[:space:]]*false' infra --include='*.tf' 2>/dev/null || true)

if [ "$FAILED" -ne 0 ]; then
    echo -e "${RED}❌ Instance-based Cloud Run billing must cite its ADR (e.g. 'ADR-019') or a '# cpu-idle-ok: <reason>' line in an adjacent comment.${NC}"
    echo -e "${RED}   cpu_idle=false bills CPU 24/7 — document the decision or switch to request-based billing.${NC}"
    exit 1
fi

echo -e "${GREEN}✅ All instance-based billing settings cite an ADR or state their reason${NC}"
exit 0
