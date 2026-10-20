#!/usr/bin/env bash
# ABOUTME: Rewrites .github/ci-postgres-test-durations.tsv from one green full-suite ci-postgres run's log
# ABOUTME: A top-level suite weighs its binary's run; an `it` module weighs its share of the `it` run's wall clock
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

# The PostgreSQL lane packs suites onto shards by these weights, and the lane
# costs whatever its slowest shard costs. A top-level suite is its own binary,
# so its weight is that binary's run: from its `Running` line to the next one.
# An `it` module shares one process with the rest of its shard, and libtest
# reports no per-module time, but it prints each test as it completes and the
# log stamps every line. The gap before a completion is wall clock the shard
# spent until that test ended, so a module's weight is the sum of the gaps
# before its own tests' completions, and a shard's module weights add up to
# its `it` wall clock. With four test threads a gap can belong to a test
# still running in another module; summed over a module's tests that evens
# out, and the packer only needs relative weights.
#
# Usage: refresh-pg-shard-weights.sh <run-id>
#   <run-id>  a green "CI: Backend (PostgreSQL)" run of the full suite (main,
#             schedule or dispatch), whose every shard ran

set -euo pipefail

RUN_ID="${1:?usage: refresh-pg-shard-weights.sh <run-id>}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="$ROOT/.github/ci-postgres-test-durations.tsv"
REPO="${GITHUB_REPOSITORY:-dravr-ai/dravr-platform}"

read -r conclusion head_sha < <(
    gh run view "$RUN_ID" -R "$REPO" --json conclusion,headSha -q '.conclusion+" "+.headSha'
)
if [ "$conclusion" != success ]; then
    echo "refresh-pg-shard-weights.sh: run $RUN_ID concluded '$conclusion'; weights come from a green run only" >&2
    exit 1
fi

LOG="$(mktemp)"
trap 'rm -f "$LOG"' EXIT
gh run view "$RUN_ID" -R "$REPO" --log >"$LOG"

python3 - "$LOG" "$OUT" "$RUN_ID" "${head_sha:0:9}" <<'PY'
import re, sys
from collections import defaultdict
from datetime import datetime

log, out, run_id, sha = sys.argv[1:5]
STAMP = re.compile(r"\t(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d+)Z ")
SHARD = re.compile(r"PostgreSQL (\d)/(\d)")
RUNNING = re.compile(r"Running crates/([a-z0-9_-]+)/tests/(?:([a-z0-9_]+)\.rs|it/main\.rs)")
TEST = re.compile(r"test ([a-z0-9_]+)::\S+ \.\.\. (ok|FAILED|ignored)")
SHARD_LINE = re.compile(r"=== Shard \d/\d \(full suite\)")

def when(line):
    m = STAMP.search(line)
    return datetime.fromisoformat(m.group(1)[:26]).timestamp() if m else None

per_shard = defaultdict(list)
full = set()
with open(log, encoding="utf8", errors="replace") as handle:
    for line in handle:
        m = SHARD.search(line)
        if not m:
            continue
        per_shard[m.group(1)].append(line)
        if SHARD_LINE.search(line):
            full.add(m.group(1))

if not per_shard or len(full) != len(per_shard):
    sys.exit(f"run {run_id} is not a full-suite run on every shard (full: {sorted(full)})")

weights = {}
for shard, lines in sorted(per_shard.items()):
    current, started, in_it, last = None, None, False, None
    for line in lines:
        at = when(line)
        if at is None:
            continue
        running = RUNNING.search(line)
        if running:
            if current and started is not None:
                weights[current] = weights.get(current, 0.0) + (at - started)
            current, started = None, None
            in_it = running.group(2) is None and running.group(1) == "pierre-server"
            if running.group(1) == "pierre-server" and running.group(2):
                current, started = running.group(2), at
            last = at
            continue
        if in_it:
            test = TEST.search(line)
            if test:
                module = test.group(1)
                weights[module] = weights.get(module, 0.0) + (at - last)
                last = at
            elif "test result:" in line:
                in_it = False
        elif current and "test result:" in line:
            weights[current] = weights.get(current, 0.0) + (at - started)
            current, started = None, None

if len(weights) < 100:
    sys.exit(f"only {len(weights)} suites measured in run {run_id}; refusing to write a partial table")

header = f"""# ABOUTME: Measured wall-clock seconds per pierre-server test suite, for the ci-postgres shard packing.
# ABOUTME: Written by scripts/ci/refresh-pg-shard-weights.sh from one green full-suite run; do not edit by hand.
#
# Source: run {run_id} (main, {sha}, green, full suite).
#
# WEIGHTS ONLY. The set of suites that runs always comes from
# `scripts/ci/server-suites.sh list`, so a suite absent here still runs — it is
# weighted at the p90 of this table so a new test is never assumed to be free.
# A stale entry costs balance, never coverage.
#
# A top-level suite weighs its own binary's run; the packer adds a per-file
# BUILD cost to it (0.61s, fitted from run 33917276134). An `it` module weighs
# its share of the shared `it` run's wall clock and carries no build cost of its
# own. Refresh with: scripts/ci/refresh-pg-shard-weights.sh <run-id>
"""
with open(out, "w", encoding="utf8") as handle:
    handle.write(header)
    for name in sorted(weights):
        handle.write(f"{name}\t{weights[name]:.1f}\n")
print(f"wrote {len(weights)} weights from run {run_id} to {out}")
PY
