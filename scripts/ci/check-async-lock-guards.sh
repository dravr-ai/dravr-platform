#!/usr/bin/env bash
# ABOUTME: Compile-free async-lock check — fails a push that can self-deadlock a tokio RwLock/Mutex
# ABOUTME: The one concurrency bug class this repo has no other gate for; see carnet#399's follow-up
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists:
#   `tokio::sync::RwLock` is documented as fair / write-preferring, with a FIFO
#   queue: "a read lock will not be given out until all write lock requests that
#   were queued before it have been acquired and released." So a task that holds
#   a read guard and then awaits `write()` on the SAME lock never completes — it
#   is waiting for itself — and because the pending writer sits at the head of
#   the queue, every later reader and writer queues behind it. The lock is dead
#   for the life of the process, not just for that request.
#
#   In deployment that reads as a hang, not a crash: Cloud Run kills each piled
#   up request at `request_timeout` (300s) while /health keeps answering, so the
#   poisoned instance stays in rotation serving 504s on the affected paths.
#
#   Nothing else here catches it. The compiler's !Send rule and
#   clippy::await_holding_lock both cover a *std* guard held across an await —
#   holding a *tokio* guard across an await is the entire point of tokio locks,
#   so neither fires. Static deadlock analysers do not model async guards at
#   all: lockbud returns None for every one of them (carnet#399), and tokio's
#   own maintainers state the general problem is undecidable, because locking
#   the same mutex in both branches of a `tokio::join!` is indistinguishable
#   from a genuine double lock. What remains is a test that exercises the path
#   and hangs against a job timeout — but a branch push runs 8 of 515 server
#   test files, so an untested path reaches main unexamined.
#
#   This gate is therefore deliberately narrow. It does not attempt the
#   undecidable case. It refuses the two shapes that are decidable from the text
#   and are always wrong.
#
# What it refuses:
#   1. SAME-LOCK RE-ACQUISITION. A named guard is bound from `X.read().await` or
#      `X.write().await`, and the same receiver expression `X` is locked again
#      before that guard is dropped or leaves scope. Always a deadlock; there is
#      no legitimate form of this.
#   2. RE-ENTRANT CALL. A named guard is live across an awaited call on the same
#      receiver the lock came from (`self.foo().await` while holding a guard on
#      `self.bar`). That callee is the likeliest place to re-take the lock, and
#      the re-entry is invisible to scan 1 because it is inter-procedural.
#
# What it deliberately does NOT flag, because each is ordinary correct code:
#   - a temporary guard (`self.cache.read().await.len()`), dropped at end of
#     statement;
#   - a guard held across an await that is not a call on the lock's own
#     receiver — the normal reason to reach for a tokio lock at all;
#   - two DIFFERENT locks held at once. Lock-order (ABBA) cycles need the whole
#     call graph, which is the undecidable half; this scan stays out of it.
#
# Escaping a finding: drop the guard first (`drop(guard);`), or close its scope,
# before the second acquisition or the call. That is the fix, not a workaround —
# it is what every multi-lock site in this workspace already does.
#
# Whole-tree, not diff-driven: the tree had zero findings when this landed, so
# there is no legacy stock to grandfather and a ratchet costs nothing.
#
# Usage:  check-async-lock-guards.sh [ROOT]
#   ROOT defaults to the repository root; a fixture tree may be passed instead,
#   which is how check-async-lock-guards.test.sh exercises every branch.
# Exit 0  no guard is re-acquired or held across a re-entrant call
# Exit 1  at least one is (each printed with file:line), or the scan parsed zero
#         async lock acquisitions — a scan that verified nothing must not pass

set -euo pipefail

SCAN_ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
if [[ ! -d "$SCAN_ROOT" ]]; then
    echo "usage: $0 [ROOT]" >&2
    echo "cannot read directory '${SCAN_ROOT}'" >&2
    exit 1
fi
cd "$SCAN_ROOT"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'

echo "Async lock guard check (compile-free)"
echo "-------------------------------------"

REPORT="$(python3 - <<'PY'
import re, sys, subprocess

files = subprocess.run(
    ["bash", "-c",
     r"grep -rl --include='*.rs' -E '\.(read|write|lock)\(\)\.await' crates/*/src || true"],
    capture_output=True, text=True).stdout.split()

# `let [mut] NAME = RECEIVER.read()/.write()/.lock().await;`
BIND = re.compile(
    r'^\s*let\s+(?:mut\s+)?(\w+)\s*=\s*(.+?)\.(read|write|lock)\(\)\s*\.await')
ACQ = re.compile(r'\.(read|write|lock)\(\)\s*\.await')

total_acq = 0
bound = 0
findings = []

for path in files:
    try:
        lines = open(path, encoding='utf-8').read().split('\n')
    except OSError:
        continue
    total_acq += sum(1 for l in lines if ACQ.search(l))

    for i, line in enumerate(lines):
        m = BIND.match(line)
        if not m:
            continue
        var, recv = m.group(1), m.group(2).strip()
        bound += 1

        # The receiver's owning object: `self.system` -> `self`. Used only by
        # scan 2, to decide whether an awaited call could re-enter this lock.
        owner = recv.rsplit('.', 1)[0] if '.' in recv else None

        # Track the guard's scope by BRACE DEPTH, not indentation. An `if` or
        # `match` arm inside the same block closes at the binding's own indent,
        # so an indent rule ends the scope early and misses the very shape this
        # gate exists for — read guard, `if …{ return }`, then write on the
        # same lock. Depth < 0 means the block holding the binding has closed.
        depth = 0
        for j in range(i + 1, len(lines)):
            nxt = lines[j]
            s = nxt.strip()
            if not s or s.startswith('//'):
                continue

            # explicit release ends the scope wherever it appears
            if re.search(r'\bdrop\s*\(\s*%s\s*\)' % re.escape(var), nxt):
                break

            # Scan 1: the same receiver locked again while the guard is live.
            if re.search(re.escape(recv) + r'\.(read|write|lock)\(\)', nxt):
                findings.append((path, j + 1, 'same-lock re-acquisition',
                                 f'`{var}` (from {recv}) is still live', s))
                break

            # Scan 2: an awaited call on the guard's own receiver-owner.
            if owner:
                call = re.search(
                    re.escape(owner) + r'\.(\w+)\s*\([^;]*\)\s*\.await', nxt)
                if call and not ACQ.search(nxt):
                    findings.append((path, j + 1, 're-entrant call',
                                     f'`{var}` (from {recv}) is still live '
                                     f'across {owner}.{call.group(1)}().await',
                                     s))
                    break

            # Brace bookkeeping last, so a line that both closes the scope and
            # re-acquires is still judged while the guard is live. Strings are
            # stripped so a brace inside one cannot unbalance the count.
            bare = re.sub(r'"(?:[^"\\]|\\.)*"', '""', nxt)
            depth += bare.count('{') - bare.count('}')
            if depth < 0:
                break

print(f"__STATS__ {len(files)} {total_acq} {bound}")
for path, ln, kind, why, src in findings:
    print(f"__FINDING__ {path}:{ln}\t{kind}\t{why}\t{src[:110]}")
PY
)"

STATS_LINE="$(printf '%s\n' "$REPORT" | grep '^__STATS__' || true)"
if [[ -z "$STATS_LINE" ]]; then
    echo -e "${RED}❌ The scan produced no summary — it did not run.${NC}"
    exit 1
fi

read -r _ N_FILES N_ACQ N_BOUND <<<"$STATS_LINE"

# Fail closed: a scan that parsed nothing has verified nothing. This repo has
# always held async lock acquisitions; zero means the pattern or the layout
# moved and the check is stale, not that the code became safe.
if [[ "$N_ACQ" -eq 0 ]]; then
    echo -e "${RED}❌ Parsed zero async lock acquisitions across ${N_FILES} file(s).${NC}"
    echo "   This scan is stale — fix the pattern rather than trusting the pass."
    exit 1
fi

FINDINGS="$(printf '%s\n' "$REPORT" | grep '^__FINDING__' || true)"

if [[ -z "$FINDINGS" ]]; then
    echo -e "${GREEN}✅ ${N_BOUND} named async guard(s) across ${N_ACQ} acquisition(s) in ${N_FILES} file(s):${NC}"
    echo -e "${GREEN}   none re-acquires its own lock or is held across a re-entrant call.${NC}"
    exit 0
fi

echo -e "${RED}❌ Async lock guard held into a second acquisition:${NC}"
echo ""
printf '%s\n' "$FINDINGS" | sed 's/^__FINDING__ //' | while IFS=$'\t' read -r loc kind why src; do
    echo -e "  ${YELLOW}${loc}${NC}  ${kind}"
    echo "      ${why}"
    echo "      ${src}"
    echo ""
done
echo "tokio's RwLock is write-preferring with a FIFO queue, so a task that holds"
echo "a guard and asks for the same lock waits for itself — and every later"
echo "reader and writer queues behind it. The lock stays dead for the life of"
echo "the process."
echo ""
echo "Fix: drop the guard (drop(<guard>);) or close its scope before the second"
echo "acquisition, as every other multi-lock site in this workspace does."
exit 1
