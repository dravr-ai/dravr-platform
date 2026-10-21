#!/usr/bin/env bash
# ABOUTME: Fixture test for resolve-push-base.sh — builds a throwaway origin and shallow CI-style checkouts
# ABOUTME: and proves each event resolves the base its mode promises, or fails when there is none.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Four ci-backend.yml steps take their diff base from this script. A wrong base
# fails silent: preflight-clippy diffed main against itself for months and
# reported green while linting nothing. Each case clones the way
# actions/checkout does (only the pushed ref, shallow, no origin/main) and
# asserts the exact sha printed, so a mode that drifts to the other's rule, or
# a fetch shape that loses the fork point, fails here.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/resolve-push-base.sh}"
ZERO=0000000000000000000000000000000000000000

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT
ERR="$ROOT/stderr"

failures=0
expect() { # $1 = label, $2 = actual, $3 = expected
  if [ "$2" = "$3" ]; then
    echo "  ✅ $1"
  else
    echo "  ❌ $1 — got '$2', want '$3'"
    failures=$((failures + 1))
  fi
}

# The origin: main has six commits; features/x forks at main~3 and adds two
# pushes; features/orphan shares no history with main.
WORK="$ROOT/work"
ORIGIN="$ROOT/origin.git"
git init -q --bare "$ORIGIN"
git -C "$ORIGIN" config uploadpack.allowReachableSHA1InWant true
git init -q "$WORK"
git -C "$WORK" config user.email test@example.com
git -C "$WORK" config user.name test
git -C "$WORK" config commit.gpgsign false
git -C "$WORK" config push.negotiate false
commit() { printf '%s\n' "$2" >"$WORK/$2"; git -C "$WORK" add -A; git -C "$WORK" commit -qm "$2"; }
git -C "$WORK" checkout -qb main
for i in 1 2 3; do commit "$WORK" "m$i"; done
FORK="$(git -C "$WORK" rev-parse HEAD)"
for i in 4 5 6; do commit "$WORK" "m$i"; done
MAIN="$(git -C "$WORK" rev-parse HEAD)"
MAIN_BEFORE="$(git -C "$WORK" rev-parse HEAD~1)"
MAIN_DEEP="$(git -C "$WORK" rev-parse HEAD~4)"
git -C "$WORK" checkout -qb features/x "$FORK"
commit "$WORK" "b1"
FEAT_BEFORE="$(git -C "$WORK" rev-parse HEAD)"
commit "$WORK" "b2"
FEAT="$(git -C "$WORK" rev-parse HEAD)"
git -C "$WORK" checkout -q --orphan features/orphan
git -C "$WORK" rm -rq --cached . && git -C "$WORK" clean -fdq
commit "$WORK" "o1"
ORPHAN="$(git -C "$WORK" rev-parse HEAD)"
git -C "$WORK" remote add origin "file://$ORIGIN"
git -C "$WORK" push -q origin main features/x features/orphan

# $1 = branch, $2 = sha, $3 = depth. Mirrors actions/checkout: the pushed sha
# alone, as refs/remotes/origin/<branch>, detached.
checkout() {
  local dir="$ROOT/co"
  rm -rf "$dir"
  git init -q "$dir"
  git -C "$dir" remote add origin "file://$ORIGIN"
  git -C "$dir" fetch -q --no-tags --depth="$3" origin "+$2:refs/remotes/origin/$1"
  git -C "$dir" checkout -q --detach "$2"
}

# $1 = mode, $2 = event, $3 = ref, $4 = before, $5 = PR base. Prints the
# resolved sha, or "exit <n>" when the script fails.
resolve() {
  local out code=0
  out="$(cd "$ROOT/co" && PR_BASE="$5" BEFORE="$4" EVENT_NAME="$2" PUSHED_REF="$3" \
    "$UNDER_TEST" "$1" 2>"$ERR")" || code=$?
  if [ "$code" -ne 0 ]; then echo "exit $code"; else echo "$out"; fi
}

echo "push to main"
checkout main "$MAIN" 50
for mode in before fork; do
  expect "$mode: a main push diffs against its before-sha" \
    "$(resolve "$mode" push refs/heads/main "$MAIN_BEFORE" "")" "$MAIN_BEFORE"
done

echo "branch re-push: the two modes differ"
checkout features/x "$FEAT" 50
expect "before: the commits this push added" \
  "$(resolve before push refs/heads/features/x "$FEAT_BEFORE" "")" "$FEAT_BEFORE"
expect "fork: everything since the branch forked" \
  "$(resolve fork push refs/heads/features/x "$FEAT_BEFORE" "")" "$FORK"

echo "a branch's first push (all-zero before-sha)"
for mode in before fork; do
  expect "$mode: the fork point" \
    "$(resolve "$mode" push refs/heads/features/x "$ZERO" "")" "$FORK"
done

echo "the fork point lies beyond a depth-1 checkout"
checkout features/x "$FEAT" 1
expect "fork: found by fetching HEAD and main 200 deep" \
  "$(resolve fork push refs/heads/features/x "$FEAT_BEFORE" "")" "$FORK"

echo "schedule or dispatch (no before-sha)"
expect "before: a branch dispatch uses the fork point" \
  "$(resolve before workflow_dispatch refs/heads/features/x "" "")" "$FORK"
checkout main "$MAIN" 50
expect "before: on main the fork point is HEAD itself" \
  "$(resolve before schedule refs/heads/main "" "")" "$MAIN"

echo "pull request"
checkout features/x "$FEAT" 50
for mode in before fork; do
  expect "$mode: the pull request's base" \
    "$(resolve "$mode" pull_request refs/pull/7/merge "" "$MAIN")" "$MAIN"
done

echo "a before-sha outside the checkout"
checkout main "$MAIN" 1
expect "before: printed" "$(resolve before push refs/heads/main "$MAIN_DEEP" "")" "$MAIN_DEEP"
if git -C "$ROOT/co" cat-file -e "${MAIN_DEEP}^{commit}" 2>/dev/null; then
  expect "before: fetched on its own so the diff can read it" yes yes
else
  expect "before: fetched on its own so the diff can read it" no yes
fi
BOGUS=deadbeefdeadbeefdeadbeefdeadbeefdeadbeef
expect "before: an unfetchable sha is still printed for the caller to judge" \
  "$(resolve before push refs/heads/main "$BOGUS" "")" "$BOGUS"

echo "no base at all"
checkout features/orphan "$ORPHAN" 50
expect "fork: a branch with no fork point fails" \
  "$(resolve fork push refs/heads/features/orphan "$ORPHAN" "")" "exit 1"
expect "before: a first push with no fork point fails" \
  "$(resolve before push refs/heads/features/orphan "$ZERO" "")" "exit 1"
if grep -q "no fork point from main" "$ERR"; then
  expect "the failure says why" yes yes
else
  expect "the failure says why" "$(cat "$ERR")" "no fork point from main"
fi

echo "the fork point cannot be looked for (origin unreachable)"
checkout features/x "$FEAT" 50
git -C "$ROOT/co" remote set-url origin "file://$ROOT/gone.git"
expect "fork: a failed fetch is a crash, not \"no fork point\"" \
  "$(resolve fork push refs/heads/features/x "$FEAT_BEFORE" "")" "exit 3"
expect "before: a first push whose lookup fails is a crash" \
  "$(resolve before push refs/heads/features/x "$ZERO" "")" "exit 3"
if grep -q "could not look for the fork point" "$ERR"; then
  expect "the crash says what failed" yes yes
else
  expect "the crash says what failed" "$(cat "$ERR")" "could not look for the fork point"
fi
expect "an unknown mode is a usage error" \
  "$(resolve sideways push refs/heads/main "$MAIN_BEFORE" "")" "exit 2"

echo ""
if [ "$failures" -gt 0 ]; then
  echo "❌ resolve-push-base.test.sh: $failures failure(s)"
  exit 1
fi
echo "✅ resolve-push-base.test.sh: every case passed"
