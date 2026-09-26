#!/usr/bin/env bash
# ABOUTME: Pins the lane monitor's alarm decision and its issue bookkeeping — which verdicts alarm,
# ABOUTME: which are superseded, and which issues it opens, re-files, comments on or closes
#
# The monitor decides, per lane, whether a red run still deserves an issue. The
# rules pull against each other and a regression in any of them is silent:
#
#   1. A cron red is NEVER buried by a dispatch success. The events are not
#      equivalent coverage, so collapsing to the newest run would hide exactly
#      the verdict this monitor exists to read.
#   2. A non-cron red that a LATER healthy cron has answered is superseded.
#      Nothing re-runs a dispatch on its own, so without this one hand-fired
#      failure pins a lane red forever.
#   3. A non-cron red is also answered by a LATER healthy run that re-ran and
#      passed every job the red run failed — never by one that skipped them,
#      never when the red run has no failed job to compare.
#   4. The newest run ANY read returns wins: a lagging filtered index must not
#      turn a daily cron into "stale" (carnet#544).
#
# Then the issue step: it must not re-file a run a closed issue records, must
# comment only on a new run, and must close issues whose lane is healthy or
# gone — but never on a scan that found no lane at all.
#
# Both halves run the workflow's OWN embedded code (the python heredoc and the
# github-script body) against stubbed data, so a copy cannot drift from the
# thing that runs in CI.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORKFLOW="$ROOT/.github/workflows/monitor-scheduled-lanes.yml"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

[ -f "$WORKFLOW" ] || { echo "FAIL: $WORKFLOW not found"; exit 1; }

python3 - "$WORKFLOW" "$WORK/monitor_body.py" "$WORK/issues_body.js" <<'PY'
import re, sys, yaml
doc = yaml.safe_load(open(sys.argv[1]))
steps = [s for j in doc["jobs"].values() for s in j["steps"]]
py = [s for s in steps if "python3" in (s.get("run") or "")]
if len(py) != 1:
    sys.exit(f"FAIL: expected exactly one python step in the monitor, found {len(py)}")
m = re.search(r"python3 - <<'PY'\n(.*?)\n\s*PY\s*$", py[0]["run"], re.S)
if not m:
    sys.exit("FAIL: could not extract the heredoc python from the monitor step")
open(sys.argv[2], "w").write(m.group(1))
js = [s for s in steps if str(s.get("uses", "")).startswith("actions/github-script")]
if len(js) != 1:
    sys.exit(f"FAIL: expected exactly one github-script step in the monitor, found {len(js)}")
open(sys.argv[3], "w").write(js[0]["with"]["script"])
PY

cd "$WORK"
mkdir -p .github/workflows

echo "── scan: alarm decision"
python3 - <<'PY'
import datetime as dt, json, os, subprocess as sp, sys
from urllib.parse import parse_qs, urlparse

NOW = dt.datetime.now(dt.timezone.utc)
def ago(d): return (NOW - dt.timedelta(days=d)).strftime("%Y-%m-%dT%H:%M:%SZ")

# lane -> runs. `jobs` is what the run's jobs endpoint answers; `hidden` keeps a
# run out of the status-filtered read, the way GitHub's lagging index did on
# 2026-09-24; `branch` defaults to main.
def run(event, concl, days, jobs=None, hidden=False, branch="main"):
    return {"event": event, "concl": concl, "days": days, "hidden": hidden,
            "branch": branch, "jobs": jobs if jobs is not None else {"a": concl}}

CASES = {
    "stale-dispatch.yml": [run("schedule", "success", 1), run("workflow_dispatch", "failure", 10)],
    "cron-red.yml":       [run("schedule", "failure", 1), run("workflow_dispatch", "success", 0)],
    "fresh-dispatch.yml": [run("schedule", "success", 3), run("workflow_dispatch", "failure", 1)],
    "stale-cron.yml":     [run("schedule", "success", 20)],
    "all-green.yml":      [run("schedule", "success", 1), run("workflow_dispatch", "success", 2)],
    "repo-dispatch.yml":  [run("schedule", "success", 1), run("repository_dispatch", "failure", 8)],
    "cancelled-cron.yml": [run("schedule", "cancelled", 1)],
    "lagged-index.yml":   [run("schedule", "success", 20), run("schedule", "success", 1, hidden=True)],
    "covered-dispatch.yml": [
        run("schedule", "success", 6),
        run("repository_dispatch", "failure", 2,
            {"resolve": "success", "bump": "failure", "stall": "success"}),
        run("workflow_dispatch", "success", 1,
            {"resolve": "success", "bump": "success", "stall": "skipped"}),
    ],
    "uncovered-dispatch.yml": [
        run("schedule", "success", 6),
        run("repository_dispatch", "failure", 2, {"live": "failure", "asserters": "success"}),
        run("workflow_dispatch", "success", 1, {"live": "skipped", "asserters": "success"}),
    ],
    "jobless-red.yml": [
        run("schedule", "success", 6),
        run("repository_dispatch", "startup_failure", 2, {}),
        run("workflow_dispatch", "success", 1),
    ],
    "earlier-green.yml": [
        run("schedule", "success", 6),
        run("repository_dispatch", "failure", 1, {"bump": "failure"}),
        run("workflow_dispatch", "success", 2, {"bump": "success"}),
    ],
    "branch-red.yml": [run("schedule", "success", 1),
                       run("workflow_dispatch", "failure", 0, branch="feature/x")],
    "disabled.yml": [run("schedule", "failure", 1)],
}
EXPECT = {
    "stale-dispatch.yml": False,  # superseded by a later green cron
    "cron-red.yml":       True,   # a cron red is never buried, even by a run that passed its job
    "fresh-dispatch.yml": True,   # dispatch red NEWER than the last cron still alarms
    "stale-cron.yml":     True,   # the cron has stopped firing
    "all-green.yml":      False,
    "repo-dispatch.yml":  False,  # supersession covers repository_dispatch too
    "cancelled-cron.yml": True,   # cancelled counts as red (usually a job timeout)
    "lagged-index.yml":   False,  # the fresh cron the index hid still counts (carnet#544)
    "covered-dispatch.yml": False,  # a later run passed every job the red one failed
    "uncovered-dispatch.yml": True, # the later run SKIPPED the failed job: nothing answered
    "jobless-red.yml":    True,   # no failed job to compare: never answered by coverage
    "earlier-green.yml":  True,   # a green that started BEFORE the red answers nothing
    "branch-red.yml":     False,  # a feature-branch dispatch is never main's verdict
    "disabled.yml":       False,  # a disabled lane is listed, never alarmed
}
STATE = {"disabled.yml": "disabled_manually"}

RUNS, JOBS = {}, {}
for li, (n, runs) in enumerate(CASES.items()):
    open(f".github/workflows/{n}", "w").write(
        f"name: {n}\non:\n  schedule:\n    - cron: '0 * * * *'\n"
        "jobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - run: 'true'\n")
    for ri, r in enumerate(runs):
        rid = li * 100 + ri + 1
        JOBS[rid] = [{"name": k, "conclusion": v} for k, v in r["jobs"].items()]
        RUNS.setdefault(n, []).append({
            "id": rid, "event": r["event"], "status": "completed", "conclusion": r["concl"],
            "head_branch": r["branch"], "created_at": ago(r["days"]),
            "run_started_at": ago(r["days"]), "hidden": r["hidden"],
            "html_url": f"https://example.invalid/{n}/{rid}"})

# Branch filters are deliberately IGNORED here: the monitor must hold the
# default-branch line itself, not trust the server to.
def served(path):
    url = urlparse(path)
    q = {k: v[0] for k, v in parse_qs(url.query).items()}
    if url.path.endswith("actions/workflows"):
        if q.get("page") == "1":
            return {"workflows": [{"path": f".github/workflows/{n}", "state": STATE.get(n, "active")}
                                  for n in CASES]}
        return {"workflows": []}
    if "/actions/runs/" in url.path and url.path.endswith("/jobs"):
        return {"jobs": JOBS[int(url.path.split("/")[-2])]}
    for n in CASES:
        if url.path.endswith(f"/workflows/{n}/runs"):
            got = [r for r in RUNS[n]
                   if ("event" not in q or r["event"] == q["event"])
                   and not (q.get("status") == "completed" and r["hidden"])]
            got.sort(key=lambda r: r["created_at"], reverse=True)
            return {"workflow_runs": got[: int(q.get("per_page", 30))]}
    raise AssertionError(f"unexpected API path in a test: {path}")

class Result:
    def __init__(self, payload):
        self.stdout = json.dumps(payload); self.stderr = ""; self.returncode = 0

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

rows, reds = g.get("rows"), g.get("reds")
scheduled, healthy = g.get("scheduled"), g.get("healthy")
if None in (rows, reds, scheduled, healthy):
    sys.exit("FAIL: the monitor did not produce rows/reds/scheduled/healthy — its structure changed, update this test")
if len(rows) != len(CASES):
    sys.exit(f"FAIL: scanned {len(rows)} lanes, expected {len(CASES)} — the selector stopped seeing fixtures")

alarmed = {r["file"] for r in reds}
bad = [n for n, want in EXPECT.items() if (n in alarmed) != want]
for n, want in EXPECT.items():
    got = n in alarmed
    print(f"  [{'ok ' if got == want else 'FAIL'}] {n:24} alarm expected={want!s:5} got={got}")
if bad:
    sys.exit("FAIL: " + ", ".join(bad))

note = {r[1].split("/")[-1]: str(r[3]) for r in rows}
superseded = sorted(n for n, v in note.items() if "superseded" in v)
want_superseded = ["covered-dispatch.yml", "repo-dispatch.yml", "stale-dispatch.yml"]
if superseded != want_superseded:
    sys.exit(f"FAIL: superseded lanes {superseded}, expected {want_superseded} — a suppressed "
             "verdict must stay visible in the step summary, never vanish")
if "passed every job it failed" not in note["covered-dispatch.yml"]:
    sys.exit(f"FAIL: covered-dispatch should say what answered it, got: {note['covered-dispatch.yml']}")
if "index lag" not in note["lagged-index.yml"]:
    sys.exit(f"FAIL: a lagging filtered read must be printed, got: {note['lagged-index.yml']}")

if sorted(scheduled) != sorted(CASES):
    sys.exit(f"FAIL: scheduled lanes {sorted(scheduled)} != every fixture (disabled lanes included)")
want_healthy = sorted(n for n, alarm in EXPECT.items() if not alarm and n != "disabled.yml")
if sorted(healthy) != want_healthy:
    sys.exit(f"FAIL: healthy lanes {sorted(healthy)}, expected {want_healthy}")
print("  scan: all alarm cases pass")
PY

echo "── issues: open, comment, acknowledge, close"
node - "$WORK/issues_body.js" "$ROOT" <<'JS'
const fs = require('fs');
const path = require('path');
const script = fs.readFileSync(process.argv[2], 'utf8');
const AsyncFunction = Object.getPrototypeOf(async () => {}).constructor;
// github-script resolves a relative require against the workspace, where the
// checkout put the repository; the root stands in for it here.
const workspaceRequire = (id) => require(id.startsWith('.') ? path.resolve(process.argv[3], id) : id);
const prefix = '[platform] scheduled lane unhealthy: ';
const red = (name) => ({
  name, file: `${name}.yml`, conclusion: 'schedule: failure', verdict: 'red on schedule (failure)',
  html_url: `https://example.invalid/runs/${name}`, run_started_at: '2026-09-24T00:00:00Z', event: 'schedule',
});
const issue = (number, state, name, body, title) => ({
  number, state, body, title: title ?? `${prefix}${name}`,
});

async function scenario({ reds, scheduled, healthy, issues, comments = {} }) {
  const calls = { created: [], commented: [], closed: [], failed: null };
  const github = {
    rest: {
      issues: {
        listForRepo: () => {},
        listComments: () => {},
        create: async (p) => { calls.created.push(p.title); return { data: { number: 1000 + calls.created.length } }; },
        createComment: async (p) => { calls.commented.push(p.issue_number); },
        update: async (p) => { calls.closed.push(`${p.issue_number}:${p.state_reason}`); },
      },
    },
    paginate: async (fn, params) => {
      if (fn === github.rest.issues.listForRepo) {
        if (params.state !== 'all') throw new Error(`issues must be read with state=all, got ${params.state}`);
        return issues;
      }
      return (comments[params.issue_number] ?? []).map((body) => ({ body }));
    },
  };
  const core = { info: () => {}, setFailed: (m) => { calls.failed = m; } };
  Object.assign(process.env, {
    REDS: JSON.stringify(reds), SCHEDULED: JSON.stringify(scheduled), HEALTHY: JSON.stringify(healthy),
    GITHUB_SERVER_URL: 'https://github.com', GITHUB_REPOSITORY: 'dravr-ai/dravr-platform', GITHUB_RUN_ID: '1',
  });
  await new AsyncFunction('require', 'github', 'core', script)(workspaceRequire, github, core);
  return calls;
}

const failures = [];
const expect = (label, got, want) => {
  const ok = JSON.stringify(got) === JSON.stringify(want);
  console.log(`  [${ok ? 'ok ' : 'FAIL'}] ${label}: ${JSON.stringify(got)}`);
  if (!ok) failures.push(`${label}: got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
};

(async () => {
  const main = await scenario({
    reds: [red('A'), red('B'), red('C'), red('D'), red('E')],
    scheduled: ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H'],
    healthy: ['F', 'G'],
    issues: [
      issue(1, 'open', 'A', 'see https://example.invalid/runs/A'),      // already records this run
      issue(2, 'open', 'B', 'see https://example.invalid/runs/B-old'),  // a NEW run -> comment
      issue(3, 'closed', 'C', 'old body'),                               // records it in a comment -> acknowledged
      issue(4, 'closed', 'D', 'see https://example.invalid/runs/D-old'), // closed on an OLDER run -> re-file
      issue(6, 'open', 'F', 'x'),                                        // lane healthy -> close
      issue(7, 'open', 'Gone', 'x'),                                     // no such lane -> close not_planned
      issue(8, 'open', 'H', 'x'),                                        // scheduled, not healthy (disabled) -> stays
      issue(9, 'closed', 'G', 'x'),                                      // already closed -> untouched
      issue(10, 'open', null, 'x', '[platform] something else entirely'), // not a lane issue -> untouched
    ],
    comments: { 3: ['acknowledged https://example.invalid/runs/C'] },
  });
  expect('opened', main.created, [`${prefix}D`, `${prefix}E`]);
  expect('commented', main.commented, [2, 6, 7]);
  expect('closed', main.closed, ['6:completed', '7:not_planned']);
  expect('failed', main.failed, null);

  const empty = await scenario({
    reds: [], scheduled: [], healthy: [],
    issues: [issue(6, 'open', 'F', 'x'), issue(7, 'open', 'Gone', 'x')],
  });
  expect('empty scan closes nothing', empty.closed, []);
  expect('empty scan fails the step', typeof empty.failed, 'string');

  if (failures.length) {
    console.error(`FAIL:\n  ${failures.join('\n  ')}`);
    process.exit(1);
  }
  console.log('  issues: all bookkeeping cases pass');
})().catch((e) => { console.error(`FAIL: ${e.stack}`); process.exit(1); });
JS

echo ""
echo "monitor-scheduled-lanes: all cases pass"
