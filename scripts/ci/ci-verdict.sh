#!/usr/bin/env bash
# ABOUTME: One verdict per pushed commit: every workflow its push should have run, read from each on.push filter,
# ABOUTME: against the runs GitHub recorded for that sha — green only when all succeeded, red on any failure or missing run.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Usage:
#   ci-verdict.sh <sha> [--branch <name>] [--before <sha>] [--repo <owner/name>]
#
# Exit status is the verdict, so a caller gates on it and never parses text:
#   0  green    every expected workflow concluded success (or skipped/neutral)
#   1  red      a run failed, was cancelled or timed out, an expected workflow
#               has no run at all, or nothing verified the sha
#   3  pending  no red yet, and a run is still queued or in progress
#   2  error    the inputs could not be read (bad sha, API failure, no branch)
#
# Our lanes are separate workflows, so no `needs:` can join them into one
# check the way a single-workflow repo's final job does. This script is that
# join, done after the fact:
#
#   1. The push. --before, else the repository activity API's push whose
#      `after` is the sha, else the commit's own parent. An all-zero before (a
#      new branch) diffs from the merge-base with origin/main, which is what
#      GitHub's path filter compares a new branch against.
#   2. Expected workflows. Every .github/workflows file AT THAT SHA (what GitHub
#      evaluated for the push) whose `on.push` matches the branch and, through
#      paths / paths-ignore, at least one file the push changed. A tags-only
#      push trigger never fires on a branch push.
#   3. Actual runs: GET actions/runs?head_sha=<sha>, push and workflow_run
#      events on that branch, the newest run per workflow file. A workflow_run
#      lane (the deploy, its post-deploy check) cannot be predicted from the
#      push, so it is scored only once it has run — it then counts like any
#      other lane, because a red deploy is exactly what a "CI:"-only reading
#      missed for twelve hours. GitHub stamps such a run with the branch tip at
#      the moment it fired, so its title is printed to show what it acted on.
#
# A run of this script's own workflow is never scored, so its status cannot
# feed back into itself.

set -euo pipefail

usage() {
    echo "usage: ci-verdict.sh <sha> [--branch <name>] [--before <sha>] [--repo <owner/name>]" >&2
    exit 2
}

SHA=""
BRANCH=""
BEFORE=""
REPO="${CI_VERDICT_REPO:-dravr-ai/dravr-platform}"
while [ $# -gt 0 ]; do
    case "$1" in
        --branch) [ $# -ge 2 ] || usage; BRANCH="$2"; shift 2 ;;
        --before) [ $# -ge 2 ] || usage; BEFORE="$2"; shift 2 ;;
        --repo) [ $# -ge 2 ] || usage; REPO="$2"; shift 2 ;;
        -h|--help) usage ;;
        -*) usage ;;
        *) [ -z "$SHA" ] || usage; SHA="$1"; shift ;;
    esac
done
[ -n "$SHA" ] || usage

command -v gh >/dev/null 2>&1 || { echo "ci-verdict.sh: gh is required" >&2; exit 2; }
command -v python3 >/dev/null 2>&1 || { echo "ci-verdict.sh: python3 is required" >&2; exit 2; }
if ! python3 -c 'import yaml' 2>/dev/null; then
    echo "ci-verdict.sh: python3 cannot import yaml (PyYAML) — cannot read the workflow filters" >&2
    exit 2
fi

# The sha must be a commit this checkout holds: the workflow files and the
# diff are read from it. A shallow CI checkout fetches it on demand.
if ! git cat-file -e "$SHA^{commit}" 2>/dev/null; then
    git fetch --quiet --no-tags --depth=200 origin "$SHA" 2>/dev/null || true
fi
if ! FULL_SHA="$(git rev-parse --verify --quiet "$SHA^{commit}")"; then
    echo "ci-verdict.sh: $SHA is not a commit in this checkout" >&2
    exit 2
fi

export CI_VERDICT_SHA="$FULL_SHA" CI_VERDICT_BRANCH="$BRANCH" CI_VERDICT_BEFORE="$BEFORE" CI_VERDICT_REPO="$REPO"
exec python3 - <<'PY'
import json, os, re, subprocess, sys
import yaml

SHA = os.environ["CI_VERDICT_SHA"]
BRANCH = os.environ["CI_VERDICT_BRANCH"]
BEFORE = os.environ["CI_VERDICT_BEFORE"]
REPO = os.environ["CI_VERDICT_REPO"]
SELF = ".github/workflows/ci-verdict.yml"
ZERO = "0" * 40
OK = {"success", "skipped", "neutral"}

def die(msg):
    print(f"ci-verdict.sh: {msg}", file=sys.stderr)
    sys.exit(2)

def git(*args, check=True):
    proc = subprocess.run(["git", *args], capture_output=True, text=True)
    if check and proc.returncode != 0:
        die(f"git {args[0]} failed: {proc.stderr.strip()}")
    return proc

def gh_json(endpoint):
    # Gated on its own exit status before anything parses the output: a failed
    # call must stop the verdict, never read as "no runs".
    proc = subprocess.run(["gh", "api", endpoint], capture_output=True, text=True)
    if proc.returncode != 0:
        return None, proc.stderr.strip() or f"gh exited {proc.returncode}"
    try:
        return json.loads(proc.stdout), None
    except json.JSONDecodeError as exc:
        return None, f"unparseable response: {exc}"

def have_commit(rev):
    return git("cat-file", "-e", f"{rev}^{{commit}}", check=False).returncode == 0

# --- 3 first: the runs say which branch pushed this sha when none is given.
runs, page = [], 1
while True:
    body, err = gh_json(f"repos/{REPO}/actions/runs?head_sha={SHA}&per_page=100&page={page}")
    if body is None:
        die(f"cannot list runs for {SHA[:10]}: {err}")
    batch = body.get("workflow_runs")
    if not isinstance(batch, list):
        die("the runs response carries no workflow_runs list")
    runs += batch
    if len(batch) < 100 or len(runs) >= body.get("total_count", 0):
        break
    page += 1

def wf_path(run):
    return (run.get("path") or "").split("@", 1)[0]

push_runs = [r for r in runs if r.get("event") == "push"]
branch = BRANCH
if not branch:
    names = {r.get("head_branch") for r in push_runs}
    if "main" in names:
        branch = "main"
    elif push_runs:
        branch = max(push_runs, key=lambda r: r.get("created_at") or "")["head_branch"]
    else:
        die(f"no push run names a branch for {SHA[:10]}; pass --branch")

# --- 1. The push range.
before, range_note = BEFORE, "given"
if not before:
    body, err = gh_json(f"repos/{REPO}/activity?ref=refs/heads/{branch}&activity_type=push&per_page=100")
    if body is None:
        range_note = f"commit only — activity API unavailable ({err})"
    else:
        hit = next((a for a in body if isinstance(a, dict) and a.get("after") == SHA), None)
        if hit:
            before, range_note = hit.get("before") or "", "push"
        else:
            range_note = "commit only — the push is not in the activity API window"
if before == ZERO:
    if not have_commit("origin/main"):
        git("fetch", "--quiet", "--no-tags", "--depth=200", "origin", "main", check=False)
    base = git("merge-base", "origin/main", SHA, check=False).stdout.strip()
    before, range_note = base, "new branch, from its merge-base with origin/main"
if before and not have_commit(before):
    git("fetch", "--quiet", "--no-tags", "--depth=200", "origin", before, check=False)
if not before or not have_commit(before):
    parent = git("rev-parse", "--verify", "--quiet", f"{SHA}^", check=False).stdout.strip()
    if before:
        range_note = f"commit only — {before[:10]} is not reachable here"
    before = parent
if before:
    changed = git("diff", "--name-only", before, SHA).stdout.split("\n")
else:
    # A root commit: every file it holds is new.
    changed = git("ls-tree", "-r", "--name-only", SHA).stdout.split("\n")
changed = [p for p in changed if p]

message = git("log", "-1", "--format=%B", SHA).stdout
skip_ci = re.search(r"\[(skip ci|ci skip|no ci|skip actions|actions skip)\]", message, re.I)

# --- 2. Expected workflows, from the definitions at the sha.
def glob_re(pattern):
    # GitHub filter syntax: * stays inside one segment, ** crosses them, and
    # ? and + quantify the preceding character.
    out, i = [], 0
    while i < len(pattern):
        c = pattern[i]
        if pattern.startswith("**", i):
            out.append(".*"); i += 2; continue
        if c == "*":
            out.append("[^/]*")
        elif c in "?+":
            out.append(c)
        elif c == "[":
            j = pattern.find("]", i)
            if j == -1:
                out.append(re.escape(c))
            else:
                out.append(pattern[i:j + 1]); i = j + 1; continue
        else:
            out.append(re.escape(c))
        i += 1
    return re.compile("".join(out) + r"\Z")

def matches(patterns, value):
    hit = False
    for pat in patterns:
        if pat.startswith("!"):
            if glob_re(pat[1:]).match(value):
                hit = False
        elif glob_re(pat).match(value):
            hit = True
    return hit

def expects(trigger):
    """None when this push fires the workflow, else why it does not."""
    if trigger is None:
        trigger = {}
    if not isinstance(trigger, dict):
        return None
    has_branch = "branches" in trigger or "branches-ignore" in trigger
    has_tag = "tags" in trigger or "tags-ignore" in trigger
    if has_tag and not has_branch:
        return "tags only"
    if "branches" in trigger and not matches(trigger["branches"] or [], branch):
        return "branch filter"
    if "branches-ignore" in trigger and matches(trigger["branches-ignore"] or [], branch):
        return "branches-ignore"
    if "paths" in trigger and not any(matches(trigger["paths"] or [], p) for p in changed):
        return "paths filter"
    if "paths-ignore" in trigger and all(matches(trigger["paths-ignore"] or [], p) for p in changed):
        return "paths-ignore"
    return None

expected, excluded, names = {}, [], {}
listing = git("ls-tree", "--name-only", f"{SHA}:.github/workflows", check=False)
if listing.returncode != 0:
    die(f"{SHA[:10]} has no .github/workflows directory")
for name in sorted(listing.stdout.split()):
    if not name.endswith((".yml", ".yaml")):
        continue
    path = f".github/workflows/{name}"
    if path == SELF:
        continue
    text = git("show", f"{SHA}:{path}").stdout
    try:
        doc = yaml.safe_load(text) or {}
    except yaml.YAMLError as exc:
        die(f"{path} at {SHA[:10]} is not valid YAML: {exc}")
    # YAML 1.1 reads a bare `on` key as the boolean true.
    on = doc.get("on", doc.get(True)) if isinstance(doc, dict) else None
    if isinstance(on, str):
        on = {on: None}
    elif isinstance(on, list):
        on = {k: None for k in on}
    title = (doc.get("name") if isinstance(doc, dict) else None) or path
    names[path] = title
    if not isinstance(on, dict) or "push" not in on:
        continue
    why = "[skip ci] in the commit message" if skip_ci else expects(on["push"])
    if why:
        excluded.append((title, why))
    else:
        expected[path] = title

# --- Join.
latest = {}
for run in runs:
    path = wf_path(run)
    if path == SELF or run.get("event") not in ("push", "workflow_run"):
        continue
    if run.get("head_branch") != branch:
        continue
    if path not in latest or (run.get("created_at") or "") > (latest[path].get("created_at") or ""):
        latest[path] = run

rows, red, pending = [], 0, 0
for path in sorted(set(expected) | set(latest)):
    run = latest.get(path)
    title = names.get(path) or (run or {}).get("name") or path
    downstream = path not in expected
    tag = " (downstream)" if downstream else ""
    if run is None:
        note = ""
        if len(changed) > 300:
            # GitHub path filters read only the first 300 changed files.
            note = f" (the push changed {len(changed)} files; GitHub's path filter reads 300)"
        rows.append(("MISSING", title, f"{path}: on.push expects a run and none exists{note}"))
        red += 1
        continue
    url = run.get("html_url", "")
    if run.get("status") != "completed":
        rows.append(("pending", title + tag, f"{run.get('status')} {url}"))
        pending += 1
        continue
    conclusion = run.get("conclusion") or "none"
    detail = url
    if downstream and run.get("display_title") not in (None, "", title):
        detail = f"'{run.get('display_title')}' {url}"
    if conclusion in OK:
        rows.append((conclusion, title + tag, detail))
    else:
        rows.append((conclusion.upper(), title + tag, detail))
        red += 1

span = f"{before[:10]}..{SHA[:10]}" if before else f"root {SHA[:10]}"
print(f"ci-verdict {SHA[:10]} on {branch} — range {span} ({range_note}), {len(changed)} files changed")
for state, title, detail in rows:
    print(f"  {state:<10} {title:<44} {detail}")
if excluded:
    print("  not expected: " + "; ".join(f"{t} ({w})" for t, w in excluded))

if red:
    print(f"verdict: red ({red} of {len(rows)} lanes)")
    sys.exit(1)
if not rows:
    print("verdict: red (nothing verified this sha: no lane was expected and none ran)")
    sys.exit(1)
if pending:
    print(f"verdict: pending ({pending} of {len(rows)} lanes still running)")
    sys.exit(3)
print(f"verdict: green ({len(rows)} lanes)")
sys.exit(0)
PY
