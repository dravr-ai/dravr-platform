#!/usr/bin/env bash
# ABOUTME: Rewrites .github/ci-postgres-test-durations.tsv from one green full-suite ci-postgres run's log
# ABOUTME: A top-level suite weighs its binary's run; an `it` module weighs its share of the `it` run's wall clock
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

# The PostgreSQL lane packs suites onto shards by these weights, and the lane
# costs whatever its slowest shard costs. A top-level suite is its own binary,
# so its weight is that binary's run: from its `Running` line to its
# `test result:` line. An `it` module shares one process with the rest of its
# shard, and libtest reports no per-module time, but it prints each test as it
# completes and the log stamps every line. The gap before a completion is wall
# clock the shard spent until that test ended, so a module's weight is the sum
# of the gaps before its own tests' completions, and a shard's module weights
# add up to its `it` wall clock. With four test threads a gap can belong to a
# test still running in another module; summed over a module's tests that
# evens out, and the packer only needs relative weights.
#
# The log declares what each shard ran (`=== Shard N/M (full suite): K of T
# suites ===` and the K names after it), and the table is checked against that
# declaration: every one of the M shards must be in the log, and every declared
# suite must come out with a weight. An `it` suite whose filter completed no
# test (its module is compiled out by a feature the lane does not enable)
# weighs 0.0 and is named on stdout; a top-level suite with no `test result:`
# fails the run.
#
# Usage: refresh-pg-shard-weights.sh <run-id>
#   <run-id>  a green "CI: Backend (PostgreSQL)" run of the full suite (main,
#             schedule or dispatch), whose every shard ran

set -euo pipefail

RUN_ID="${1:?usage: refresh-pg-shard-weights.sh <run-id>}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="$ROOT/.github/ci-postgres-test-durations.tsv"
REPO="${GITHUB_REPOSITORY:-dravr-ai/dravr-platform}"
WORKFLOW="CI: Backend (PostgreSQL)"

if ! meta="$(gh run view "$RUN_ID" -R "$REPO" --json workflowName,conclusion,headSha,headBranch \
    -q '[.workflowName, .conclusion, .headSha, .headBranch] | @tsv')"; then
    echo "refresh-pg-shard-weights.sh: could not read run $RUN_ID from $REPO" >&2
    exit 1
fi
IFS=$'\t' read -r workflow conclusion head_sha head_branch <<<"$meta"
if [ "$workflow" != "$WORKFLOW" ]; then
    echo "refresh-pg-shard-weights.sh: run $RUN_ID is '$workflow', not '$WORKFLOW'" >&2
    exit 1
fi
if [ "$conclusion" != success ]; then
    echo "refresh-pg-shard-weights.sh: run $RUN_ID concluded '$conclusion'; weights come from a green run only" >&2
    exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
LOG="$TMP/run.log"
SUITES="$TMP/suites.tsv"
gh run view "$RUN_ID" -R "$REPO" --log >"$LOG"
"$ROOT/scripts/ci/server-suites.sh" list >"$SUITES"

python3 - "$LOG" "$OUT" "$RUN_ID" "${head_sha:0:9}" "$head_branch" "$SUITES" <<'PY'
import os, re, sys
from collections import defaultdict
from datetime import datetime, timezone

log, out, run_id, sha, branch, suites_file = sys.argv[1:7]
LINE = re.compile(
    r"^Database Tests \(PostgreSQL (\d+)/(\d+)\)\t[^\t]*\t"
    r"(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d+)Z(?: (.*))?$"
)
HEADER = re.compile(r"^=== Shard (\d+)/(\d+) \(full suite\): (\d+) of (\d+) suites ===$")
ANNOUNCED = re.compile(r"^  ([a-z0-9_]+)$")
RUNNING = re.compile(r"Running crates/([a-z0-9_-]+)/tests/(?:([a-z0-9_]+)\.rs|it/main\.rs)")
TEST = re.compile(r"test ([a-z0-9_]+)::\S+ \.\.\. (ok|FAILED|ignored)")

def fail(message):
    sys.exit(f"refresh-pg-shard-weights.sh: run {run_id}: {message}")

# The log stamps every line in UTC; reading it as naive local time would put
# an hour into any gap that spans a daylight-saving change on this machine.
def when(stamp):
    return datetime.fromisoformat(stamp[:26]).replace(tzinfo=timezone.utc).timestamp()

per_shard = defaultdict(list)
totals = set()
with open(log, encoding="utf8", errors="replace") as handle:
    for raw in handle:
        m = LINE.match(raw.rstrip("\n"))
        if not m:
            continue
        totals.add(int(m.group(2)))
        per_shard[int(m.group(1))].append((when(m.group(3)), m.group(4) or ""))

if len(totals) != 1:
    fail(f"expected one shard count in the log, found {sorted(totals)}")
shards = totals.pop()
if set(per_shard) != set(range(1, shards + 1)):
    fail(f"the log holds shards {sorted(per_shard)} of {shards}; every shard must have run")

weights = {}
announced = {}
it_ran = set()
for shard, lines in sorted(per_shard.items()):
    headers = [i for i, (_, body) in enumerate(lines) if HEADER.match(body)]
    if len(headers) != 1:
        fail(f"shard {shard} printed {len(headers)} full-suite headers, expected exactly one")
    header = HEADER.match(lines[headers[0]][1])
    count, total = int(header.group(3)), int(header.group(4))
    names = []
    for _, body in lines[headers[0] + 1:]:
        name = ANNOUNCED.match(body)
        if not name:
            break
        names.append(name.group(1))
    if len(names) != count:
        fail(f"shard {shard} declared {count} suites but listed {len(names)}")
    for name in names:
        if name in announced:
            fail(f"suite {name} is declared on shards {announced[name]} and {shard}")
        announced[name] = shard
    announced_total = total

    current, started, in_it, last = None, None, False, None
    for at, body in lines[headers[0] + 1 + count:]:
        running = RUNNING.search(body)
        if running:
            if current:
                fail(f"shard {shard}: {current} has no `test result:` before the next binary started")
            if in_it:
                fail(f"shard {shard}: the it run has no `test result:` before the next binary started")
            in_it = running.group(2) is None and running.group(1) == "pierre-server"
            if in_it:
                it_ran.add(shard)
            if running.group(1) == "pierre-server" and running.group(2):
                current, started = running.group(2), at
            last = at
            continue
        if in_it:
            test = TEST.search(body)
            if test:
                module = test.group(1)
                weights[module] = weights.get(module, 0.0) + (at - last)
                last = at
            elif "test result:" in body:
                in_it = False
        elif current and "test result:" in body:
            weights[current] = weights.get(current, 0.0) + (at - started)
            current, started = None, None
    if current or in_it:
        fail(f"shard {shard}: {current or 'the it run'} has no `test result:` before the log ends")

if len(announced) != announced_total:
    fail(f"the shards declared {len(announced)} distinct suites, not the {announced_total} they run between them")

kinds = {}
with open(suites_file, encoding="utf8") as handle:
    for row in handle:
        name, kind = row.rstrip("\n").split("\t")
        kinds[name] = kind

empty = []
for name, shard in sorted(announced.items()):
    if name in weights:
        continue
    # A declared suite with no measurement is only honest as 0.0 when it is an
    # `it` module on a shard whose `it` run finished: its filter selected
    # nothing. Anything else is a log this parser did not understand.
    if kinds.get(name) != "it" or shard not in it_ran:
        fail(f"suite {name} was declared on shard {shard} but never measured")
    weights[name] = 0.0
    empty.append(name)

collateral = sorted(set(weights) - set(announced))
if collateral:
    fail(f"tests of modules no shard declared completed in an it run: {collateral}")

header = f"""# ABOUTME: Measured wall-clock seconds per pierre-server test suite, for the ci-postgres shard packing.
# ABOUTME: Written by scripts/ci/refresh-pg-shard-weights.sh from one green full-suite run; do not edit by hand.
#
# Source: run {run_id} ({branch}, {sha}, green, full suite).
#
# WEIGHTS ONLY. The set of suites that runs always comes from
# `scripts/ci/server-suites.sh list`, so a suite absent here still runs — it is
# weighted at the p90 of this table so a new test is never assumed to be free.
# A stale entry costs balance, never coverage.
#
# A top-level suite weighs its own binary's run; the packer adds a per-file
# BUILD cost to it (0.61s, fitted from run 33917276134). An `it` module weighs
# its share of the shared `it` run's wall clock and carries no build cost of its
# own; one whose filter completed no test in that run weighs 0.0.
# Refresh with: scripts/ci/refresh-pg-shard-weights.sh <run-id>
"""
partial = f"{out}.partial"
with open(partial, "w", encoding="utf8") as handle:
    handle.write(header)
    for name in sorted(weights):
        handle.write(f"{name}\t{weights[name]:.1f}\n")
os.replace(partial, out)
if empty:
    print(f"it suites that completed no test, weighted 0.0: {' '.join(empty)}")
print(f"wrote {len(weights)} weights for the {len(announced)} suites {shards} shards declared in run {run_id} to {out}")
PY
