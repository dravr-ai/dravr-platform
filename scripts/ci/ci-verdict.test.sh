#!/usr/bin/env bash
# ABOUTME: Fixture test for ci-verdict.sh — recorded runs JSON against a throwaway repo's workflow filters,
# ABOUTME: covering green, red, a missing run, pending, a path-filtered lane excluded, and ci-verdict.yml's trigger list.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Each case runs the real script inside a throwaway git repo whose
# .github/workflows hold five lanes, with a `gh` on PATH that answers the two
# REST calls the script makes from a case directory of recorded JSON:
#   CI: Source   on.push branches [main], paths ['src/**']
#   CI: Docs     on.push paths ['docs/**']             — path-filtered out of a src push
#   CI: Always   on.push with no filters
#   CI: NoDocs   on.push paths-ignore ['docs/**']
#   Release      on.push tags only                     — never a branch push
#   Deploy       workflow_run only                     — scored once it has run
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/ci-verdict.sh}"
REAL_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

REPO="$WORK/repo"
mkdir -p "$REPO/.github/workflows" "$REPO/src" "$REPO/docs" "$WORK/bin"
git -C "$REPO" init -q -b main
git -C "$REPO" config user.email test@example.com
git -C "$REPO" config user.name test
wf() { printf '%s\n' "$2" >"$REPO/.github/workflows/$1"; }
wf source.yml 'name: "CI: Source"
on:
  push:
    branches: [ main ]
    paths: [ "src/**" ]'
wf docs.yml 'name: "CI: Docs"
on:
  push:
    paths: [ "docs/**" ]'
wf always.yml 'name: "CI: Always"
on: [push]'
wf nodocs.yml 'name: "CI: NoDocs"
on:
  push:
    paths-ignore: [ "docs/**" ]'
wf release.yml 'name: "Release"
on:
  push:
    tags: [ "v*" ]'
wf deploy.yml 'name: "Deploy"
on:
  workflow_run:
    workflows: ["CI: Source"]
    types: [completed]'
wf ci-verdict.yml 'name: "CI: Verdict"
on:
  workflow_run:
    workflows: ["CI: Source"]
    types: [completed]'
echo a >"$REPO/docs/a.md"
echo a >"$REPO/src/a.rs"
git -C "$REPO" add -A
git -C "$REPO" commit -qm base
BASE="$(git -C "$REPO" rev-parse HEAD)"
echo b >"$REPO/docs/a.md"
git -C "$REPO" commit -qam "docs"
DOCS="$(git -C "$REPO" rev-parse HEAD)"
echo b >"$REPO/src/a.rs"
git -C "$REPO" commit -qam "src"
SHA="$(git -C "$REPO" rev-parse HEAD)"

# The gh stub: `gh api <endpoint>` answers from $CASE/runs.json or
# $CASE/activity.json; a missing file is a failed call, as a refused one is.
cat >"$WORK/bin/gh" <<'STUB'
#!/bin/sh
[ "$1" = api ] || exit 1
case "$2" in
  *actions/runs*) f="$CASE/runs.json" ;;
  *activity*) f="$CASE/activity.json" ;;
  *) exit 1 ;;
esac
[ -f "$f" ] || { echo "gh: HTTP 403" >&2; exit 1; }
cat "$f"
STUB
chmod +x "$WORK/bin/gh"

# One run object. $1 workflow file, $2 status, $3 conclusion (or null), $4 event.
run() {
    local concl="null"
    [ "$3" = null ] || concl="\"$3\""
    printf '{"name":"x","path":".github/workflows/%s","event":"%s","head_branch":"main","status":"%s","conclusion":%s,"created_at":"2026-10-20T10:00:00Z","html_url":"https://example.test/%s","display_title":"t"}' \
        "$1" "${4:-push}" "$2" "$concl" "$1"
}
# Writes a case directory: $1 name, rest = run objects.
make_case() {
    local dir="$WORK/cases/$1" body
    shift
    mkdir -p "$dir"
    body="$(IFS=,; echo "$*")"
    printf '{"total_count":%s,"workflow_runs":[%s]}\n' "$#" "$body" >"$dir/runs.json"
    printf '[{"before":"%s","after":"%s","ref":"refs/heads/main"}]\n' "$DOCS" "$SHA" >"$dir/activity.json"
    printf '%s\n' "$dir"
}

OUT="$WORK/out"
# $1 label, $2 case dir, $3 expected exit, $4.. literals the output must carry.
expect() {
    local label="$1" case_dir="$2" want="$3" code=0 lit ok=1
    shift 3
    local extra=()
    [ -z "${EXTRA:-}" ] || read -r -a extra <<< "$EXTRA"
    ( cd "$REPO" && CASE="$case_dir" PATH="$WORK/bin:$PATH" "$UNDER_TEST" "$SHA" ${extra[@]+"${extra[@]}"} ) >"$OUT" 2>&1 || code=$?
    [ "$code" = "$want" ] || ok=0
    for lit in "$@"; do grep -qF -- "$lit" "$OUT" || ok=0; done
    if [ "$ok" = 1 ]; then pass "$label"; else
        fail "$label (exit $code, wanted $want)"
        sed 's/^/      /' "$OUT"
    fi
}
expect_absent() { # $1 label, $2 literal
    if grep -qF -- "$2" "$OUT"; then fail "$1 (output carries: $2)"; sed 's/^/      /' "$OUT"; else pass "$1"; fi
}

echo "ci-verdict.sh fixture tests"

# The push is DOCS..SHA (the activity API's before), so it touched src/ only:
# Source, Always, NoDocs are expected; Docs is path-filtered out.
green="$(make_case green "$(run source.yml completed success)" "$(run always.yml completed success)" \
    "$(run nodocs.yml completed success)")"
expect "every expected lane succeeded: green" "$green" 0 "verdict: green (3 lanes)" "CI: Docs (paths filter)" \
    "Release (tags only)" "range ${DOCS:0:10}..${SHA:0:10} (push)"
expect_absent "a path-filtered lane is not reported missing" "MISSING"

red="$(make_case red "$(run source.yml completed failure)" "$(run always.yml completed success)" \
    "$(run nodocs.yml completed success)")"
expect "a failed lane: red" "$red" 1 "FAILURE    CI: Source" "verdict: red (1 of 3 lanes)"

cancelled="$(make_case cancelled "$(run source.yml completed cancelled)" "$(run always.yml completed success)" \
    "$(run nodocs.yml completed success)")"
expect "a cancelled lane: red" "$cancelled" 1 "CANCELLED  CI: Source"

missing="$(make_case missing "$(run source.yml completed success)" "$(run always.yml completed success)")"
expect "an expected lane with no run: red" "$missing" 1 "MISSING    CI: NoDocs" "verdict: red (1 of 3 lanes)"

pending="$(make_case pending "$(run source.yml in_progress null)" "$(run always.yml completed success)" \
    "$(run nodocs.yml completed success)")"
expect "a lane still running: pending" "$pending" 3 "pending    CI: Source" "verdict: pending (1 of 3 lanes still running)"

pending_red="$(make_case pending-red "$(run source.yml in_progress null)" "$(run always.yml completed failure)" \
    "$(run nodocs.yml completed success)")"
expect "a failure beside a running lane: red, not pending" "$pending_red" 1 "verdict: red"

downstream="$(make_case downstream "$(run source.yml completed success)" "$(run always.yml completed success)" \
    "$(run nodocs.yml completed success)" "$(run deploy.yml completed failure workflow_run)")"
expect "a failed downstream deploy: red" "$downstream" 1 "FAILURE    Deploy (downstream)"

self="$(make_case self "$(run source.yml completed success)" "$(run always.yml completed success)" \
    "$(run nodocs.yml completed success)" "$(run ci-verdict.yml completed failure workflow_run)")"
expect "its own workflow's run is never scored" "$self" 0 "verdict: green (3 lanes)"

schedule="$(make_case schedule "$(run source.yml completed success)" "$(run always.yml completed success)" \
    "$(run nodocs.yml completed success)" "$(run docs.yml completed failure schedule)")"
expect "a scheduled run on the sha is not the push's verdict" "$schedule" 0 "verdict: green (3 lanes)"

# The activity API refused: the range falls back to the commit itself, which
# also touched src/ only, and the verdict says so.
no_activity="$(make_case no-activity "$(run source.yml completed success)" "$(run always.yml completed success)" \
    "$(run nodocs.yml completed success)")"
rm "$no_activity/activity.json"
expect "no activity API: commit-only range, still judged" "$no_activity" 0 "commit only — activity API unavailable"

# --before widens the range over the docs commit: CI: Docs becomes expected,
# and its absence is a missing run.
EXTRA="--before $BASE" expect "a wider push range expects the docs lane too" "$green" 1 "MISSING    CI: Docs"

no_runs_dir="$WORK/cases/no-runs-file"
mkdir -p "$no_runs_dir"
expect "a failed runs call is an error, never green" "$no_runs_dir" 2 "cannot list runs"

empty="$(make_case empty)"
EXTRA="--branch main" expect "no runs at all: every expected lane missing" "$empty" 1 "MISSING    CI: Source"
expect "no runs and no branch given: an error" "$empty" 2 "pass --branch"

# ci-verdict.yml must listen to every workflow a branch push can fire, or the
# last lane to finish never refreshes the status. One system, read two ways:
# the trigger list against the workflow names on.push declares.
if missing_names="$(python3 - "$REAL_ROOT/.github/workflows" <<'PY'
import glob, os, sys
import yaml
root = sys.argv[1]
names, listened = set(), None
for path in sorted(glob.glob(os.path.join(root, "*.yml"))):
    doc = yaml.safe_load(open(path)) or {}
    on = doc.get("on", doc.get(True))
    if isinstance(on, list):
        on = {k: None for k in on}
    if os.path.basename(path) == "ci-verdict.yml":
        listened = set(on["workflow_run"]["workflows"])
        continue
    if isinstance(on, dict) and "push" in on:
        trig = on["push"] or {}
        if ("tags" in trig or "tags-ignore" in trig) and not ("branches" in trig or "branches-ignore" in trig):
            continue
        names.add(doc["name"])
if listened is None:
    sys.exit("ci-verdict.yml has no workflow_run trigger")
if not names:
    sys.exit("no push-triggered workflow found — the scan verified nothing")
print("\n".join(sorted(names - listened)))
PY
)"; then
    if [ -z "$missing_names" ]; then pass "ci-verdict.yml listens to every branch-push workflow"; else
        fail "ci-verdict.yml does not listen to: $(tr '\n' ';' <<< "$missing_names")"
    fi
else
    fail "could not read the workflow trigger lists"
fi

echo ""
if [ "$failures" -gt 0 ]; then
    echo "❌ $failures case(s) failed"
    exit 1
fi
echo "✅ all ci-verdict.sh cases passed"
