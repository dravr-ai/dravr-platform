#!/usr/bin/env bash
# ABOUTME: Fails CI when Terraform adds standing synthetic traffic (uptime check, Cloud Scheduler job) with no Idle-cost note
# ABOUTME: A probe that fires inside Cloud Run's idle window keeps an instance up all day; the cost must be stated before it lands
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists
# ---------------
# Cloud Run scales a service to zero only after about 15 minutes with no
# requests. Anything that calls it more often than that keeps one instance up
# around the clock, and both dev front services bill CPU even while idle
# (cpu_idle = false): the api's 2 vCPU (ADR-019) and the frontend's 1 vCPU,
# whose min_instances = 0 was what kept that cheap. The api is internal-only,
# so every outside probe enters through the frontend and keeps it warm too.
# check-cpu-idle-adr.sh guards cpu_idle; this gate guards the other half, the
# traffic that keeps the instance alive.
#
# It happened: five sign-in uptime checks (147f8c5, carnet#1152) hit dev about
# every 5 seconds from Oct 23 2026. Neither the api nor the frontend scaled to
# zero again, Cloud Run CPU went from ~88k to ~261k vCPU-s a day, and dev cost
# rose about CA$6 a day until they were removed (97e8666). The change had been
# reviewed for rate limits and log volume, never for what it would bill.
#
# An uptime check can never be gentle enough: its slowest period is 15 minutes
# and it runs from at least 3 regions, so every one of them keeps its target
# warm. A Cloud Scheduler job wakes whatever it calls on every tick.
#
# The rule
# --------
# Every `resource "google_monitoring_uptime_check_config"` and
# `resource "google_cloud_scheduler_job"` in infra/**/*.tf needs, in the
# comment lines just above it, a line of the form
#
#   # Idle-cost: <what it keeps warm and roughly what that costs, or why nothing>
#
# for example "Idle-cost: none, it starts a Cloud Run job through the :run API
# and the job calls no service", or "Idle-cost: keeps the frontend and api up
# 24/7, ~CA$6/day in dev; accepted because …". What the job itself calls counts
# too. A probe that would keep a service warm usually
# belongs after the change that can break it instead (a deploy or an apply
# step, as scripts/ci/first-party-sign-in-smoke.sh does), not on a timer.
#
# Usage: scripts/ci/check-keep-warm-traffic.sh [repo-root]
#        (the root defaults to this repository; the self-test passes a fixture)

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
ROOT="${1:-$( cd "$SCRIPT_DIR/../.." && pwd )}"
cd "$ROOT"

echo -e "${BLUE}==== Standing synthetic traffic Idle-cost gate ====${NC}"

if [ ! -d infra ]; then
    echo -e "${GREEN}✅ No infra/ directory — nothing to check${NC}"
    exit 0
fi

# How many lines above the resource line to search for the note. The note
# belongs to the resource's own comment block, not to a section banner.
CONTEXT_LINES=10

FAILED=0
CHECKED=0

while IFS=: read -r file line text; do
    [ -z "$file" ] && continue
    CHECKED=$((CHECKED + 1))
    type=$(sed -E 's/^resource "([a-z_]+)".*/\1/' <<<"$text")
    start=$(( line - CONTEXT_LINES ))
    [ "$start" -lt 1 ] && start=1
    # Reversed with awk, not tac: macOS has no tac, and a missing tool here
    # would fail every resource on a developer's pre-push.
    # Only the comment block directly above counts: walking up from the
    # resource, the first code line (a `}` closing the previous resource, a
    # string holding the words) ends it, so a neighbour's note or an
    # Idle-cost inside a heredoc is not a note on this resource.
    if sed -n "${start},$(( line - 1 ))p" "$file" \
        | awk '{ l[NR] = $0 } END { for (i = NR; i > 0; i--) print l[i] }' \
        | awk '/^[[:space:]]*$/ || /^[[:space:]]*(#|\/\/)/ { print; next } { exit }' \
        | grep -qE '^[[:space:]]*(#|//)[[:space:]]*Idle-cost:[[:space:]]*[^[:space:]]'; then
        echo -e "${GREEN}  OK — $file:$line ($type) states its idle cost${NC}"
    else
        echo -e "${RED}  ❌ $file:$line ($type) has no '# Idle-cost:' note within ${CONTEXT_LINES} lines above${NC}"
        FAILED=1
    fi
done < <(grep -rnE '^resource "(google_monitoring_uptime_check_config|google_cloud_scheduler_job)"' infra --include='*.tf' 2>/dev/null || true)

if [ "$FAILED" -ne 0 ]; then
    echo -e "${RED}❌ Standing synthetic traffic must say what it keeps warm and what that costs.${NC}"
    echo -e "${RED}   Cloud Run scales to zero only after ~15 min idle, and the dev api and frontend bill CPU while idle:${NC}"
    echo -e "${RED}   an uptime check at any period keeps them up 24/7 (~CA\$6/day in dev, Oct 2026).${NC}"
    echo -e "${RED}   Add '# Idle-cost: <what it keeps warm, the cost or why none>' above the resource,${NC}"
    echo -e "${RED}   or run the probe after the deploy/apply that can break it instead of on a timer.${NC}"
    exit 1
fi

echo -e "${GREEN}✅ ${CHECKED} uptime check / scheduler resource(s), each states its idle cost${NC}"
exit 0
