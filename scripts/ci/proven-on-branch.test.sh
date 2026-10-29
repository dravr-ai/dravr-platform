#!/usr/bin/env bash
# ABOUTME: Fixture test for proven-on-branch.sh — a stub gh serves canned run and job lists, and each case asserts
# ABOUTME: which jobs count as proven: same tree, full-lane branch, green run, and only the jobs that passed.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# A job wrongly reported proven is skipped on main with nobody having run it
# on those bytes: a green verdict nobody earned. Every rule that keeps a run
# or a job out of the proof has a case here.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/proven-on-branch.sh}"

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT
REPO_DIR="$ROOT/repo"
FIX="$ROOT/fixtures"
mkdir -p "$REPO_DIR" "$FIX" "$ROOT/bin"

git -C "$REPO_DIR" init -q
git -C "$REPO_DIR" config user.email test@example.com
git -C "$REPO_DIR" config user.name test
git -C "$REPO_DIR" config commit.gpgsign false
printf 'a\n' >"$REPO_DIR/a"; git -C "$REPO_DIR" add -A; git -C "$REPO_DIR" commit -qm a
TREE="$(git -C "$REPO_DIR" rev-parse 'HEAD^{tree}')"
OTHER=0000000000000000000000000000000000000001

# The stub answers the two endpoints the script calls from files in $FIX,
# applies --jq when given, and fails when $FIX/fail names the endpoint.
cat >"$ROOT/bin/gh" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[ "$1" = api ] || { echo "stub gh: unexpected $*" >&2; exit 1; }
url="$2"; shift 2
filter=""
while [ $# -gt 0 ]; do
  case "$1" in --jq) filter="$2"; shift 2 ;; *) shift ;; esac
done
case "$url" in
  */actions/workflows/*/runs\?*) file="$FIX/runs.json"; ep=runs ;;
  */actions/runs/*/jobs\?*) id="${url#*/actions/runs/}"; id="${id%%/*}"; file="$FIX/jobs-$id.json"; ep="jobs-$id" ;;
  *) echo "stub gh: unknown endpoint $url" >&2; exit 1 ;;
esac
if [ -f "$FIX/fail" ] && grep -qx "$ep" "$FIX/fail"; then echo "HTTP 500" >&2; exit 1; fi
if [ -n "$filter" ]; then jq -r "$filter" "$file"; else cat "$file"; fi
STUB
chmod +x "$ROOT/bin/gh"

failures=0
expect() { # $1 = label, $2 = actual, $3 = expected
  if [ "$2" = "$3" ]; then
    echo "  ✅ $1"
  else
    echo "  ❌ $1 — got '$(printf '%s' "$2" | tr '\n' '|')', want '$(printf '%s' "$3" | tr '\n' '|')'"
    failures=$((failures + 1))
  fi
}
run() { (cd "$REPO_DIR" && PATH="$ROOT/bin:$PATH" FIX="$FIX" GITHUB_REPOSITORY=o/r "$UNDER_TEST" "$@"); }
run_code() { local c=0; run "$@" >/dev/null 2>&1 || c=$?; echo "$c"; }
runs() { # each argument: id:branch:tree[:event[:conclusion]], event push and conclusion success by default
  local items=() id branch tree event concl
  for r in "$@"; do
    IFS=: read -r id branch tree event concl <<<"$r"
    items+=("{\"id\":$id,\"event\":\"${event:-push}\",\"conclusion\":\"${concl:-success}\",\"head_branch\":\"$branch\",\"head_commit\":{\"tree_id\":\"$tree\"}}")
  done
  local IFS=,
  printf '{"workflow_runs":[%s]}\n' "${items[*]}" >"$FIX/runs.json"
}
jobs() { # $1 = run id, then name=conclusion pairs, each optionally followed by |step=conclusion entries
  local id="$1"; shift
  local items=() job name concl steps step
  for pair in "$@"; do
    job="${pair%%|*}"; name="${job%=*}"; concl="${job##*=}"; steps=()
    if [ "$job" != "$pair" ]; then
      IFS='|' read -ra rest <<<"${pair#*|}"
      for step in "${rest[@]}"; do steps+=("{\"name\":\"${step%=*}\",\"conclusion\":\"${step##*=}\"}"); done
    fi
    local IFS=,
    items+=("{\"name\":\"$name\",\"conclusion\":\"$concl\",\"steps\":[${steps[*]}]}")
    unset IFS
  done
  local IFS=,
  printf '{"jobs":[%s]}\n' "${items[*]}" >"$FIX/jobs-$id.json"
}

echo "=== usage"
expect "no workflow exits 2" "$(run_code)" 2
expect "no GITHUB_REPOSITORY exits 2" "$(cd "$REPO_DIR" && c=0; PATH="$ROOT/bin:$PATH" GITHUB_REPOSITORY='' "$UNDER_TEST" w.yml >/dev/null 2>&1 || c=$?; echo $c)" 2

echo "=== proof"
runs "11:feature/x:$TREE"
jobs 11 "Clippy=success" "Leaf Tests=success"
expect "a green feature/ run on the same tree proves its passing jobs" "$(run w.yml)" "Clippy
Leaf Tests"

jobs 11 "Clippy=success" "Leaf Tests=skipped" "Dev Tests=failure"
expect "a job the branch run skipped or failed is not proven" "$(run w.yml)" "Clippy"

runs "11:feature/x:$OTHER"
expect "another tree proves nothing" "$(run w.yml)" ""

runs "11:fix/y:$TREE"
expect "a fix/ branch (smoke lane) proves nothing" "$(run w.yml)" ""

runs "11:main:$TREE"
expect "a main run proves nothing" "$(run w.yml)" ""

runs "11:feature/x:$TREE:push:failure"
expect "a run that did not conclude success proves nothing" "$(run w.yml)" ""

runs "11:feature/x:$TREE:pull_request"
expect "a pull_request run proves nothing" "$(run w.yml)" ""

runs "11:feature/x:$TREE"
jobs 11 "Clippy=success|Detect=success|Lint=skipped" "Tests=success|Run=success" "Audit=failure|Scan=success"
expect "a step is proven only when it and its job passed" "$(run w.yml)" "Clippy
Clippy -> Detect
Tests
Tests -> Run"

runs "11:claude/z:$TREE" "12:features/w:$TREE"
jobs 11 "A=success"
jobs 12 "B=success" "A=success"
expect "claude/ and features/ count, and their jobs are unioned" "$(run w.yml)" "A
B"

runs "11:claude/z:$TREE" "12:claude/z:$TREE" "13:claude/z:$TREE" "14:claude/z:$TREE"
jobs 11 "A=success"; jobs 12 "A=success"; jobs 13 "A=success"; jobs 14 "Old=success"
expect "only the newest three runs are read" "$(run w.yml)" "A"

echo "=== failures cost minutes, never coverage"
runs "11:feature/x:$TREE"
jobs 11 "Clippy=success"
echo runs >"$FIX/fail"
expect "a run list the API refuses exits 3" "$(run_code w.yml)" 3
echo jobs-11 >"$FIX/fail"
expect "a job list the API refuses exits 3" "$(run_code w.yml)" 3
rm -f "$FIX/fail"
printf '{"message":"Not Found"}\n' >"$FIX/runs.json"
expect "a run list with no workflow_runs exits 3" "$(run_code w.yml)" 3
runs "11:feature/x:$TREE"
printf '{"message":"Not Found"}\n' >"$FIX/jobs-11.json"
expect "a job list with no jobs array exits 3" "$(run_code w.yml)" 3
expect "no git checkout exits 3" "$(cd "$ROOT" && c=0; PATH="$ROOT/bin:$PATH" FIX="$FIX" GITHUB_REPOSITORY=o/r GIT_CEILING_DIRECTORIES="$ROOT" "$UNDER_TEST" w.yml >/dev/null 2>&1 || c=$?; echo $c)" 3

if [ "$failures" -gt 0 ]; then
  echo "proven-on-branch.test.sh: ${failures} failure(s)"
  exit 1
fi
echo "proven-on-branch.test.sh: all passed"
