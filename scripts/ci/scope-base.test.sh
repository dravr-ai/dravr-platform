#!/usr/bin/env bash
# ABOUTME: Fixture test for scope-base.sh — a stub gh serves canned main runs and jobs, and each case asserts which commit
# ABOUTME: a scoped lane diffs against: main's fork point on a branch, the newest green ancestor on main, or none.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# A base that never passed the lane lets a scoped run skip tests over a red it
# inherited: a green nobody earned. Every rule that keeps a commit from being
# the base has a case here.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/scope-base.sh}"

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT
R="$ROOT/repo"
FIX="$ROOT/fixtures"
mkdir -p "$R" "$FIX" "$ROOT/bin"

git -C "$R" init -q -b main
git -C "$R" config user.email test@example.com
git -C "$R" config user.name test
git -C "$R" config commit.gpgsign false
commit() { echo "$1" >>"$R/f"; git -C "$R" add -A; git -C "$R" commit -qm "$1"; git -C "$R" rev-parse HEAD; }
C1="$(commit c1)"
C2="$(commit c2)"
git -C "$R" checkout -q -b side
SIDE="$(commit side)"
git -C "$R" checkout -q main
C3="$(commit c3)"
C4="$(commit c4)"
git -C "$R" checkout -q -b feature/x "$C2"
commit feat >/dev/null
git -C "$R" checkout -q main

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
    echo "  ❌ $1 — got '$2', want '$3'"
    failures=$((failures + 1))
  fi
}
# run EVENT REF [JOB...]: the base printed, then ":<exit code>".
run() {
  local event="$1" ref="$2" out code=0
  shift 2
  [ $# -gt 0 ] || set -- Lane
  out="$(cd "$R" && PATH="$ROOT/bin:$PATH" FIX="$FIX" GITHUB_REPOSITORY=o/r EVENT_NAME="$event" PUSHED_REF="$ref" \
         MAIN_REF=main "$UNDER_TEST" w.yml "$@" 2>/dev/null)" || code=$?
  echo "${out}:${code}"
}
runs() { # each argument: id:sha[:event[:branch]], listed newest first by created_at
  local items=() id sha event branch n=100
  for r in "$@"; do
    IFS=: read -r id sha event branch <<<"$r"
    items+=("{\"id\":$id,\"head_sha\":\"$sha\",\"event\":\"${event:-push}\",\"head_branch\":\"${branch:-main}\",\"created_at\":\"2026-10-30T10:$((n--)):00Z\"}")
  done
  local IFS=,
  printf '{"workflow_runs":[%s]}\n' "${items[*]}" >"$FIX/runs.json"
}
jobs() { # $1 = run id, then name=conclusion pairs
  local id="$1" items=() pair
  shift
  for pair in "$@"; do items+=("{\"name\":\"${pair%=*}\",\"conclusion\":\"${pair##*=}\"}"); done
  local IFS=,
  printf '{"jobs":[%s]}\n' "${items[*]}" >"$FIX/jobs-$id.json"
}

echo "=== usage and events"
expect "no job named exits 2" "$(cd "$R" && c=0; "$UNDER_TEST" w.yml >/dev/null 2>&1 || c=$?; echo $c)" 2
expect "a schedule run has no base" "$(run schedule refs/heads/main)" ":0"
expect "a dispatch run has no base" "$(run workflow_dispatch refs/heads/main)" ":0"
expect "a pull request has no base" "$(run pull_request refs/pull/1/merge)" ":0"

echo "=== a branch push"
git -C "$R" checkout -q feature/x
expect "a branch diffs against its fork point from main" "$(run push refs/heads/feature/x)" "$C2:0"
git -C "$R" checkout -q main

echo "=== a main push"
jobs 9 Lane=success
jobs 8 Lane=success
jobs 7 Lane=failure
jobs 6 Lane=skipped
jobs 5 Lane=success Other=failure
jobs 4 Lane=success
runs "9:$C4" "8:$SIDE" "7:$C3" "6:$C3" "5:$C2"
expect "HEAD's own run, a non-ancestor, a red and a skipped job are passed over" "$(run push refs/heads/main)" "$C2:0"
runs "9:$C4" "7:$C3" "4:$C1"
expect "a job it does not name may be red" "$(run push refs/heads/main)" "$C1:0"
expect "every named job must have passed" "$(run push refs/heads/main Lane Other)" ":0"
runs "4:$C3:pull_request" "5:$C2:push:feature/x" "6:$C1:schedule"
jobs 4 Lane=success
jobs 5 Lane=success
jobs 6 Lane=success
expect "a pull request or branch run is never a base; a schedule run is" "$(run push refs/heads/main)" "$C1:0"
runs "7:$C3" "6:$C2"
jobs 6 Lane=skipped
expect "no green ancestor means no base" "$(run push refs/heads/main)" ":0"
runs "4:0000000000000000000000000000000000000001" "5:$C2"
jobs 5 Lane=success
expect "a sha this checkout lacks is passed over" "$(run push refs/heads/main)" "$C2:0"

echo "=== lookups that fail"
echo runs >"$FIX/fail"
expect "a failed run list exits 3" "$(run push refs/heads/main)" ":3"
runs "5:$C2"
echo jobs-5 >"$FIX/fail"
expect "a failed job list exits 3" "$(run push refs/heads/main)" ":3"
rm -f "$FIX/fail"

echo ""
if [ "$failures" -gt 0 ]; then
  echo "❌ scope-base.test.sh: $failures case(s) failed"
  exit 1
fi
echo "✅ scope-base.test.sh: every case passed"
