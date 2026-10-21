#!/usr/bin/env bash
# ABOUTME: Fixture test for ci-time-report.py — a hand-computed cache of runs and jobs, read offline and through a stand-in gh
# ABOUTME: Pins job-hours, weighted runner wait, both plan targets, the census-aware sample weight and every refusal
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The report is the yardstick carnet#1114's phases are judged by, so a slip in
# its arithmetic would pass or fail a phase on a number nobody measured. The
# fixture is a cache laid out exactly as the script writes one, holding a few
# runs whose every expected figure was computed by hand (comments beside them):
#
#   - job-hours per workflow, Linux and macOS apart, over every attempt, with a
#     skipped job, a job that never got a runner and a skipped run left out
#   - the weighted wait percentiles (first value whose cumulative weight reaches
#     p), a start before creation floored at 0, and the dependency premise
#   - branch Backend p90 (push, non-main, success, attempt 1) and the main
#     verdict p90 (first run of the push -> last verdict workflow), each against
#     its goal, plus the success-or-failure and with-E2E variants
#   - a sampled stratum beside a census window weighs (non-census runs) /
#     (sampled runs), not cnt/fet: 5.00 job-hours here where cnt/fet gives 5.33
#   - an empty window, a cache miss under --offline and a failing gh exit
#     non-zero instead of printing a report
#
# A stand-in `gh` serves the same tree for the fetch path: it proves the cache is
# filled and then read, and that a page holding an unfinished run or job is not
# cached. Offline cases must never reach it.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/ci-time-report.py}"
REPO="example/ci-time-fixture"
WORK="$(mktemp -d)"
OUT="$WORK/out"
trap 'rm -rf "$WORK"' EXIT

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

command -v python3 >/dev/null 2>&1 || { echo "  ❌ python3 is required"; exit 1; }

# --- the stand-in gh ----------------------------------------------------------
# Serves `gh api <endpoint>` from $FAKE_GH_SERVE in the cache's own layout, logs
# every call, and fails like a 502 when FAKE_GH_FAIL is set.
mkdir -p "$WORK/bin"
cat >"$WORK/bin/gh" <<'PY'
#!/usr/bin/env python3
import os
import re
import sys
from urllib.parse import parse_qs, urlsplit

with open(os.environ["FAKE_GH_LOG"], "a") as fh:
    fh.write(" ".join(sys.argv[1:]) + "\n")
if os.environ.get("FAKE_GH_FAIL"):
    sys.stderr.write("gh: HTTP 502: Bad Gateway\n")
    sys.exit(1)
parts = urlsplit(sys.argv[2])
qs = parse_qs(parts.query)
m = re.fullmatch(r"repos/([^/]+)/([^/]+)/actions/runs(?:/(\d+)/jobs)?", parts.path)
rel = f"jobs/{m.group(3)}" if m.group(3) else f"runs/{qs['created'][0]}"
path = os.path.join(os.environ["FAKE_GH_SERVE"], m.group(1), m.group(2), rel, f"page-{qs['page'][0]}.json")
try:
    sys.stdout.write(open(path).read())
except FileNotFoundError:
    sys.stderr.write(f"gh: HTTP 404: no fixture at {path}\n")
    sys.exit(1)
PY
chmod +x "$WORK/bin/gh"
export PATH="$WORK/bin:$PATH"
export FAKE_GH_LOG="$WORK/gh.log"
export FAKE_GH_SERVE="$WORK/serve"
: >"$FAKE_GH_LOG"

# --- the fixture ---------------------------------------------------------------
python3 - "$WORK/serve/$REPO" <<'PY'
import json
import os
import sys

root = sys.argv[1]
D1, D2, DS, DP = "2026-10-19", "2026-10-20", "2026-10-05", "2026-10-06"
DEPLOY = "Deploy 0123456789abcdef0123456789abcdef01234567"


def at(day, hms):
    return f"{day}T{hms}Z" if hms else None


def run(rid, name, event, branch, sha, day, created, updated, conclusion="success", attempt=1, status="completed"):
    return {"id": rid, "name": name, "event": event, "head_branch": branch, "head_sha": sha,
            "created_at": at(day, created), "updated_at": at(day, updated), "conclusion": conclusion,
            "run_attempt": attempt, "status": status}


def job(day, created, started, completed, mac=False, conclusion="success", runner="GitHub Actions 7", attempt=1, status="completed"):
    return {"created_at": at(day, created), "started_at": at(day, started), "completed_at": at(day, completed),
            "conclusion": conclusion, "runner_name": runner, "labels": ["macos-15"] if mac else ["ubuntu-latest"],
            "run_attempt": attempt, "status": status}


# Main pushes. m1: first run 09:59, last verdict CI 10:50 -> 51.0, with iOS 11:11 -> 72.0.
# m2: 90.0 / 90.0 (a failed Backend still ends the verdict). m3: 30.0 / 50.0. m4 has no verdict workflow.
RUNS = [
    run(104, "Lint: Docs", "push", "main", "m1", D1, "09:59:00", "10:01:00"),
    run(101, "CI: Backend (Rust)", "push", "main", "m1", D1, "10:00:00", "10:50:00"),
    run(102, "CI: Web Frontend", "push", "main", "m1", D1, "10:00:00", "10:20:00"),
    run(103, "CI: Mobile E2E (iOS)", "push", "main", "m1", D1, "10:00:00", "11:11:00"),
    run(111, "CI: Backend (Rust)", "push", "main", "m2", D1, "20:00:00", "21:30:00", "failure"),
    run(112, "CI: Backend (PostgreSQL)", "push", "main", "m2", D1, "20:00:00", "20:40:00"),
    run(121, "CI: TypeScript SDK", "push", "main", "m3", D2, "12:30:00", "13:00:00"),
    run(122, "CI: Mobile E2E (Android)", "push", "main", "m3", D2, "12:30:00", "13:20:00"),
    run(131, DEPLOY, "push", "main", "m4", D2, "18:00:00", "18:30:00"),
    # Branch Backend walls: 20, 25, 28 count; 40 (failure) only in the variant; the
    # second attempt, the dispatch and the cancelled run count in neither.
    run(201, "CI: Backend (Rust)", "push", "feature/a", "b1", D1, "13:00:00", "13:20:00"),
    run(202, "CI: Backend (Rust)", "push", "feature/a", "b2", D1, "15:00:00", "15:25:00"),
    run(203, "CI: Backend (Rust)", "push", "feature/b", "b3", D2, "01:00:00", "01:40:00", "failure"),
    run(204, "CI: Backend (Rust)", "push", "feature/b", "b4", D2, "03:00:00", "03:10:00", attempt=2),
    run(205, "CI: Backend (Rust)", "workflow_dispatch", "feature/c", "b5", D2, "05:00:00", "05:31:00"),
    run(206, "CI: Backend (Rust)", "push", "feature/c", "b6", D2, "14:00:00", "14:28:00"),
    run(207, "CI: Backend (Rust)", "push", "feature/c", "b7", D2, "16:00:00", "16:05:00", "cancelled"),
    # Skipped: never fetched (it has no jobs file, so an offline fetch would fail).
    run(208, "CI: Web Frontend", "push", "feature/c", "b7", D2, "16:00:00", "16:00:30", "skipped"),
    # The sampled day: 301 is inside the census window, 302-304 form one stratum
    # of which 2 are sampled at weight 3/2 -> 2.0 + 2 x 1.0 x 1.5 = 5.00 job-hours.
    run(301, "CI: Web Frontend", "push", "feature/w", "w1", DS, "02:00:00", "04:00:00"),
    run(302, "CI: Web Frontend", "push", "feature/w", "w2", DS, "08:00:00", "09:01:00"),
    run(303, "CI: Web Frontend", "push", "feature/w", "w3", DS, "13:00:00", "14:01:00"),
    run(304, "CI: Web Frontend", "push", "feature/w", "w4", DS, "15:00:00", "16:01:00"),
    # The unfinished day: nothing about 401 may be cached.
    run(401, "CI: Web Frontend", "push", "feature/p", "p1", DP, "09:00:00", "09:05:00", None, status="in_progress"),
]

# Minutes per job are in the comments as duration / wait.
JOBS = {
    101: [job(D1, "10:00:00", "10:02:00", "10:32:00"),                      # 30 / 2
          job(D1, "10:32:00", "10:41:00", "10:50:00"),                      # 9 / 9, dependent on the first
          job(D1, "10:32:00", "10:32:00", "10:32:00", conclusion="skipped", runner="")],
    102: [job(D1, "10:00:00", "10:01:00", "10:19:00")],                     # 18 / 1
    103: [job(D1, "10:00:00", "10:00:00", "10:30:00"),                      # 30 / 0 Linux
          job(D1, "10:30:00", "10:30:30", "11:10:30", mac=True),            # 40 / 0.5 macOS
          job(D1, "10:30:00", "10:29:30", "10:37:30", mac=True)],           # 8 / -0.5 -> 0 macOS
    104: [job(D1, "09:59:00", "09:59:30", "10:00:30")],                     # 1 / 0.5
    111: [job(D1, "20:00:00", "20:30:00", "21:30:00")],                     # 60 / 30
    112: [job(D1, "20:00:00", "20:05:00", "20:40:00")],                     # 35 / 5
    121: [job(D2, "12:30:00", "12:30:00", "13:00:00")],                     # 30 / 0
    122: [job(D2, "12:30:00", "12:32:00", "13:20:00")],                     # 48 / 2
    131: [job(D2, "18:00:00", "18:00:00", "18:30:00")],                     # 30 / 0
    201: [job(D1, "13:00:00", "13:04:00", "13:20:00")],                     # 16 / 4
    202: [job(D1, "15:00:00", "15:03:00", "15:25:00"),                      # 22 / 3 (page 1 of 2)
          job(D1, "15:00:00", "15:00:00", "15:06:00")],                     # 6 / 0 (page 2 of 2)
    203: [job(D2, "01:00:00", "01:10:00", "01:40:00")],                     # 30 / 10
    204: [job(D2, "03:00:00", "03:01:00", "03:04:00", conclusion="failure"),  # 3 / 1, attempt 1
          job(D2, "03:05:00", "03:05:00", "03:10:00", attempt=2)],          # 5 / 0, attempt 2
    205: [job(D2, "05:00:00", "05:00:00", "05:31:00")],                     # 31 / 0
    206: [job(D2, "14:00:00", "14:06:00", "14:28:00"),                      # 22 / 6
          job(D2, "14:10:00", "14:12:00", "14:14:00")],                     # 2 / 2, dependent, no sibling ended at 14:10
    207: [job(D2, "16:00:00", "16:01:00", "16:05:00", conclusion="cancelled"),  # 4 / 1
          job(D2, "16:05:00", "16:05:00", "16:05:00", conclusion="cancelled", runner="")],
    301: [job(DS, "02:00:00", "02:00:00", "04:00:00")],                     # 120
    302: [job(DS, "08:00:00", "08:01:00", "09:01:00")],                     # 60
    303: [job(DS, "13:00:00", "13:01:00", "14:01:00")],                     # 60
    304: [job(DS, "15:00:00", "15:01:00", "16:01:00")],                     # 60
    401: [job(DP, "09:00:00", "09:01:00", None, status="in_progress")],
}
# Pages smaller than the API's 100 so both paginators walk past page 1.
RUN_PAGE = {f"{D1}T00:00:00Z..{D1}T11:59:59Z": 3}
JOB_PAGE = {202: 1}


def write(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as fh:
        json.dump(data, fh)


def pages(items, key, base, size):
    chunks = [items[i:i + size] for i in range(0, len(items), size)] or [[]]
    for n, chunk in enumerate(chunks, 1):
        write(f"{base}/page-{n}.json", {"total_count": len(items), key: chunk})


for day in ("2026-10-01", DS, DP, D1, D2):
    for lo, hi in (("00:00:00", "11:59:59"), ("12:00:00", "23:59:59")):
        window = f"{day}T{lo}Z..{day}T{hi}Z"
        listed = [r for r in RUNS if f"{day}T{lo}Z" <= r["created_at"] <= f"{day}T{hi}Z"]
        listed.sort(key=lambda r: r["created_at"], reverse=True)  # the API lists newest first
        pages(listed, "workflow_runs", f"{root}/runs/{window}", RUN_PAGE.get(window, 100))
for rid, jobs in JOBS.items():
    pages(jobs, "jobs", f"{root}/jobs/{rid}", JOB_PAGE.get(rid, 100))
PY

# --- helpers -------------------------------------------------------------------
run_report() { # args -> prints the exit status; output, spaces squeezed, lands in $OUT
  local code=0
  python3 "$UNDER_TEST" --repo "$REPO" "$@" >"$WORK/raw" 2>&1 || code=$?
  tr -s ' ' <"$WORK/raw" >"$OUT"
  echo "$code"
}

expect() { # $1 = label, $2 = actual, $3 = expected
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1 (got '$2', expected '$3')"
    sed 's/^/      /' "$OUT"
  fi
}

expect_out() { # $1 = label, $2 = fixed string the output must contain
  if grep -qF -- "$2" "$OUT"; then pass "$1"; else
    fail "$1: missing '$2'"
    sed 's/^/      /' "$OUT"
  fi
}

line_after() { # $1 = fixed string; prints the line after the first line containing it
  awk -v h="$1" 'found { print; exit } index($0, h) { found = 1 }' "$OUT"
}

report_body() { # the report without the lines that name the cache or count calls
  grep -v -e 'API calls' -e 'fetching jobs' "$OUT"
}

echo "ci-time-report.py fixture tests"

echo "── offline, census of a two-day window"
expect "the report exits 0" "$(run_report --since 2026-10-19 --until 2026-10-20 --cache-dir "$WORK/serve" --offline)" 0
expect_out "runs are counted, the skipped one apart" "runs: 17 listed, 1 skipped, 0 not completed"
expect_out "a small window is a census" "jobs: census, all 16 runs"
expect_out "no API call is made" "API calls: 0"
expect "the largest workflow comes first" "$(line_after "workflow linux macOS total share")" " CI: Backend (Rust) 4.00 0.00 4.00 50.0%"
expect_out "macOS job-hours are split from Linux" "CI: Mobile E2E (iOS) 0.50 0.80 1.30"
expect_out "a skipped run adds nothing" "CI: Web Frontend 0.30 0.00 0.30"
expect_out "deploys share one name" "Deploy (sha) 0.50 0.00 0.50"
expect_out "the total sums every job that ran" "TOTAL 7.20 0.80 8.00"
expect_out "the wait over every job" "all jobs jobs=22 p50=1.0 p90=9.0 p99=30.0"
expect_out "the Linux wait" "Linux jobs=20 p50=1.0 p90=9.0 p99=30.0"
expect_out "the macOS wait floors a start before creation at 0" "macOS jobs=2 p50=0.0 p90=0.5 p99=0.5"
expect_out "the Backend wait" "CI: Backend (Rust) jobs=13 p50=2.0 p90=10.0 p99=30.0"
expect_out "the dependency premise is re-checked" "4 of 5 dependent jobs were created within 3 s"
expect_out "branch Backend p90 passes its goal" 'branch "CI: Backend (Rust)" (push to a non-main branch, success, attempt 1): p90=27.4 min, goal <= 30 min: PASS (n=3 p50=25.0 p90=27.4 p99=27.9)'
expect_out "the success-or-failure variant is shown apart" "success or failure (printed by the baseline too, not the target): n=4 p50=26.5 p90=36.4 p99=39.6"
expect_out "main verdict p90 misses its goal" "main verdict (main push -> last verdict CI workflow): p90=82.2 min, goal <= 45 min: MISS (n=3 p50=51.0 p90=82.2 p99=89.2)"
expect_out "the with-E2E verdict is shown" "including mobile E2E (iOS, Android), no goal of its own: n=3 p50=72.0 p90=86.4 p99=89.6"
report_body >"$WORK/offline.body"

echo "── offline, a sampled stratum beside a census window"
expect "the report exits 0" "$(run_report --since 2026-10-05 --until 2026-10-05 --census-max 0 --census-window 2026-10-05T00..2026-10-05T06 --cache-dir "$WORK/serve" --offline)" 0
expect_out "the census window is fetched in full and the rest sampled" "jobs: 1 census + 2 sampled of 3 other runs (strata 1, rate 0.12, seed 42)"
expect_out "a sampled run weighs non-census over sampled, not cnt/fet" "CI: Web Frontend 5.00 0.00 5.00 100.0%"
expect "a target with no qualifying run says NO DATA instead of a number" "$(grep -c "NO DATA" "$OUT")" 2

echo "── offline refusals"
expect "an empty window exits 1" "$(run_report --since 2026-10-01 --until 2026-10-01 --cache-dir "$WORK/serve" --offline)" 1
expect_out "it refuses to report the empty window" "refusing to report an empty window"
expect "a cache miss exits 1" "$(run_report --since 2026-10-02 --until 2026-10-02 --cache-dir "$WORK/serve" --offline)" 1
expect_out "it names the missing response" "is not in the cache"
expect "--until before --since exits 2" "$(run_report --since 2026-10-20 --until 2026-10-19 --cache-dir "$WORK/serve" --offline)" 2
expect "no offline case reached gh" "$(wc -l <"$FAKE_GH_LOG" | tr -d ' ')" 0

echo "── through the stand-in gh"
expect "a cold cache exits 0" "$(run_report --since 2026-10-19 --until 2026-10-20 --cache-dir "$WORK/cache")" 0
# 5 runs pages (one window holds two) + 17 jobs pages (one run holds two).
expect_out "every page is fetched once" "API calls: 22"
report_body >"$WORK/online.body"
expect "the fetched report equals the offline one" "$(diff "$WORK/offline.body" "$WORK/online.body" >/dev/null && echo same || echo different)" same
expect "the rerun reads the cache it filled" "$(run_report --since 2026-10-19 --until 2026-10-20 --cache-dir "$WORK/cache" --offline)" 0
expect_out "the rerun makes no call" "API calls: 0"

expect "an unfinished run is reported" "$(run_report --since 2026-10-06 --until 2026-10-06 --cache-dir "$WORK/cache")" 0
expect_out "it is counted as not completed" "runs: 1 listed, 0 skipped, 1 not completed"
UNFINISHED="$WORK/cache/$REPO/runs/2026-10-06T00:00:00Z..2026-10-06T11:59:59Z/page-1.json"
expect "its runs page is not cached" "$([ -e "$UNFINISHED" ] && echo cached || echo absent)" absent
expect "its jobs page is not cached" "$([ -e "$WORK/cache/$REPO/jobs/401/page-1.json" ] && echo cached || echo absent)" absent
expect "a finished page beside it is cached" "$([ -e "$WORK/cache/$REPO/runs/2026-10-06T12:00:00Z..2026-10-06T23:59:59Z/page-1.json" ] && echo cached || echo absent)" cached

expect "a failing gh exits 1" "$(FAKE_GH_FAIL=1 run_report --since 2026-10-05 --until 2026-10-05 --cache-dir "$WORK/cold")" 1
expect_out "it says how gh failed" "failed 3 times: gh: HTTP 502: Bad Gateway"

if [ "$failures" -gt 0 ]; then
  echo "❌ $failures ci-time-report.py case(s) failed"
  exit 1
fi
echo "✅ ci-time-report.py fixture tests passed"
