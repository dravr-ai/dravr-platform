#!/usr/bin/env bash
# ABOUTME: Pins the lane monitor's alarm decision — which verdicts alarm and which are superseded
# ABOUTME: Runs the workflow's OWN embedded python against stubbed run data, not a copy of it
#
# The monitor decides, per lane, whether a red run still deserves an issue. Two
# rules pull against each other and a regression in either is silent:
#
#   1. A cron red is NEVER buried by a dispatch success. The events are not
#      equivalent coverage, so collapsing to the newest run would hide exactly
#      the verdict this monitor exists to read.
#   2. A non-cron red that a LATER healthy cron has answered is superseded.
#      Nothing re-runs a dispatch on its own, so without this one hand-fired
#      failure pins a lane red forever.
#
# The test extracts the python from the workflow rather than restating it, so a
# copy cannot drift from the thing that runs in CI.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORKFLOW="$ROOT/.github/workflows/monitor-scheduled-lanes.yml"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

[ -f "$WORKFLOW" ] || { echo "FAIL: $WORKFLOW not found"; exit 1; }

python3 - "$WORKFLOW" "$WORK/monitor_body.py" <<'PY'
import re, sys, yaml
doc = yaml.safe_load(open(sys.argv[1]))
steps = [s for j in doc["jobs"].values() for s in j["steps"] if "python3" in (s.get("run") or "")]
if len(steps) != 1:
    sys.exit(f"FAIL: expected exactly one python step in the monitor, found {len(steps)}")
m = re.search(r"python3 - <<'PY'\n(.*?)\n\s*PY\s*$", steps[0]["run"], re.S)
if not m:
    sys.exit("FAIL: could not extract the heredoc python from the monitor step")
open(sys.argv[2], "w").write(m.group(1))
PY

cd "$WORK"
mkdir -p .github/workflows
python3 - <<'PY'
import datetime as dt, json, os, subprocess as sp, sys

NOW = dt.datetime.now(dt.timezone.utc)
def ago(d): return (NOW - dt.timedelta(days=d)).strftime("%Y-%m-%dT%H:%M:%SZ")

# lane -> {event: (conclusion, days_ago)}; EXPECT -> should it open an issue?
CASES = {
    "stale-dispatch.yml": {"schedule": ("success", 1), "workflow_dispatch": ("failure", 10)},
    "cron-red.yml":       {"schedule": ("failure", 1), "workflow_dispatch": ("success", 0)},
    "fresh-dispatch.yml": {"schedule": ("success", 3), "workflow_dispatch": ("failure", 1)},
    "stale-cron.yml":     {"schedule": ("success", 20)},
    "all-green.yml":      {"schedule": ("success", 1), "workflow_dispatch": ("success", 2)},
    "repo-dispatch.yml":  {"schedule": ("success", 1), "repository_dispatch": ("failure", 8)},
    "cancelled-cron.yml": {"schedule": ("cancelled", 1)},
}
EXPECT = {
    "stale-dispatch.yml": False,  # superseded by a later green cron
    "cron-red.yml":       True,   # a cron red is never buried by a dispatch success
    "fresh-dispatch.yml": True,   # dispatch red NEWER than the last cron still alarms
    "stale-cron.yml":     True,   # the cron has stopped firing
    "all-green.yml":      False,
    "repo-dispatch.yml":  False,  # supersession covers repository_dispatch too
    "cancelled-cron.yml": True,   # cancelled counts as red (usually a job timeout)
}

for n in CASES:
    open(f".github/workflows/{n}", "w").write(
        f"name: {n}\non:\n  schedule:\n    - cron: '0 * * * *'\n"
        "jobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - run: 'true'\n")

def served(path):
    if "actions/workflows?per_page" in path:
        if path.endswith("page=1"):
            return {"workflows": [{"path": f".github/workflows/{n}", "state": "active"} for n in CASES]}
        return {"workflows": []}
    for n, evs in CASES.items():
        if f"/workflows/{n}/runs" in path:
            ev = path.split("event=")[1].split("&")[0]
            if ev not in evs:
                return {"workflow_runs": []}
            concl, d = evs[ev]
            return {"workflow_runs": [{"conclusion": concl, "run_started_at": ago(d), "event": ev,
                                       "html_url": f"https://example.invalid/{n}/{ev}", "id": 1}]}
    return {"workflow_runs": []}

class Result:
    def __init__(self, payload):
        self.stdout = json.dumps(payload); self.stderr = ""; self.returncode = 0

real = sp.run
def stub(cmd, **kw):
    if isinstance(cmd, list) and cmd[:2] == ["gh", "api"]:
        return Result(served(cmd[2]))
    raise AssertionError(f"the monitor reached the network in a test: {cmd!r}")
sp.run = stub

os.environ.update({"GITHUB_REPOSITORY": "dravr-ai/dravr-platform", "STALE_AFTER_DAYS": "8",
                   "GITHUB_REF_NAME": "main", "GITHUB_STEP_SUMMARY": os.devnull,
                   "GITHUB_OUTPUT": os.devnull})

g = {"__name__": "__main__"}
exec(compile(open("monitor_body.py").read(), "monitor-scheduled-lanes.yml", "exec"), g)

rows = g.get("rows")
reds = g.get("reds")
if rows is None or reds is None:
    sys.exit("FAIL: the monitor did not produce rows/reds — its structure changed, update this test")
if len(rows) != len(CASES):
    sys.exit(f"FAIL: scanned {len(rows)} lanes, expected {len(CASES)} — the selector stopped seeing fixtures")

alarmed = {r["file"] for r in reds}
bad = [n for n, want in EXPECT.items() if (n in alarmed) != want]
for n, want in EXPECT.items():
    got = n in alarmed
    print(f"  [{'ok ' if got == want else 'FAIL'}] {n:22} alarm expected={want!s:5} got={got}")

superseded = [r for r in rows if "superseded" in str(r[3])]
if len(superseded) != 2:
    sys.exit(f"FAIL: expected 2 lanes to REPORT a superseded run, got {len(superseded)} — "
             "a suppressed verdict must stay visible in the step summary, never vanish")

if bad:
    sys.exit("FAIL: " + ", ".join(bad))
print("\nmonitor-scheduled-lanes: all alarm cases pass")
PY
