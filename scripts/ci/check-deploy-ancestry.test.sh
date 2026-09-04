#!/usr/bin/env bash
# ABOUTME: Fixture test for check-deploy-ancestry.sh — builds a throwaway history and plants a
# ABOUTME: rollback to prove the guard fires, then every fail-open shape to prove it stays open.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# publish-images.yml only ever runs on main, so the guard it calls cannot be
# exercised on a branch; this test is the guard's only verification before it
# meets a real deploy. The case that matters is the fired one: a deploy that
# would put an ancestor of the serving commit on dev must be refused. Every
# other case is the guard staying out of the way — an absent label, a value
# that is not a sha, a commit git has never seen, a diverged commit — because
# a guard that blocks a legitimate deploy is worse than no guard (carnet#262).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="$SCRIPT_DIR/check-deploy-ancestry.sh"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

# Output lands in a fixed file, not a variable: run_guard is called inside a
# command substitution, so any variable it sets dies with that subshell.
OUT="$(mktemp)"
REPO="$(mktemp -d)"
NOREPO="$(mktemp -d)"
trap 'rm -rf "$OUT" "$REPO" "$NOREPO"' EXIT

run_guard() { # $1 = resolved, $2 = live, $3 = repo; echoes exit code, leaves output in $OUT
  local code=0
  "$UNDER_TEST" "$1" "$2" "$3" >"$OUT" 2>&1 || code=$?
  printf '%s\n' "$code"
}

expect() { # $1 = label, $2 = actual, $3 = wanted
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1 (exit $2, wanted $3)"; sed 's/^/       /' "$OUT"
  fi
}

# A history shaped like main during a push burst: three commits in a line,
# plus one that diverged from the first and never merged.
#
#   A --- B --- C      (main)
#    \
#     D                (diverged)
git -C "$REPO" init -q
git -C "$REPO" config user.email test@example.com
git -C "$REPO" config user.name test
commit() { # $1 = message; echoes the new sha
  echo "$1" > "$REPO/$1"
  git -C "$REPO" add -A
  git -C "$REPO" commit -q -m "$1"
  git -C "$REPO" rev-parse HEAD
}
A="$(commit A)"
B="$(commit B)"
C="$(commit C)"
git -C "$REPO" checkout -q "$A"
D="$(commit D)"
git -C "$REPO" checkout -q -
UNKNOWN="0000000000000000000000000000000000000001"

echo "check-deploy-ancestry.sh"

# --- 1. the planted rollback: dev serves C, the deploy resolved B ---------------
# The 2026-09-04 00:56Z shape. Live is a DESCENDANT of resolved, so shipping
# resolved would move dev backwards. This is the one case the guard exists for.
code="$(run_guard "$B" "$C" "$REPO")"
expect "rollback fires: resolved B behind live C" "$code" 1
grep -q "$B" "$OUT" || fail "fired verdict must name the resolved sha"
grep -q "$C" "$OUT" || fail "fired verdict must name the live sha"
grep -q "^skip:" "$OUT" || fail "fired verdict must start with 'skip:'"

# --- 2. the identical commit: a redeploy of what dev serves --------------------
code="$(run_guard "$C" "$C" "$REPO")"
expect "same commit fires: dev already serves it" "$code" 1
grep -q "^skip:" "$OUT" || fail "same-commit verdict must start with 'skip:'"

# --- 3. the ordinary deploy: resolved C is newer than live B --------------------
code="$(run_guard "$C" "$B" "$REPO")"
expect "forward deploy passes: resolved C ahead of live B" "$code" 0
grep -q "^deploy:" "$OUT" || fail "forward verdict must start with 'deploy:'"

# --- 4. two commits ahead, not just one ----------------------------------------
expect "forward deploy passes across several commits" "$(run_guard "$C" "$A" "$REPO")" 0

# --- 5. no live label at all -----------------------------------------------------
# The first workflow_run deploy after the label starts being stamped, or any
# service whose revision carries no commit-sha. Nothing to compare: deploy.
code="$(run_guard "$B" "" "$REPO")"
expect "empty live sha passes" "$code" 0
grep -q "^deploy:" "$OUT" || fail "empty-live verdict must start with 'deploy:'"

# --- 6. a live label that is not a sha ------------------------------------------
expect "non-hex live label passes" "$(run_guard "$B" "not-a-sha" "$REPO")" 0
expect "live label shaped like an option passes" "$(run_guard "$B" "--version" "$REPO")" 0

# --- 7. a live sha this repository has never seen -------------------------------
# A label stamped from a rewound branch, or a repo history that was fetched
# too shallow to contain it. Unprovable, so deploy.
code="$(run_guard "$B" "$UNKNOWN" "$REPO")"
expect "unknown live sha passes" "$code" 0
grep -q "not in this repository's history" "$OUT" || fail "unknown-live verdict must say the commit is unknown"

# --- 8. a live commit that diverged: neither ancestor nor descendant -------------
# D forked from A and never reached main. Deploying B is not a rollback of D.
expect "diverged live commit passes" "$(run_guard "$B" "$D" "$REPO")" 0

# --- 9. the resolved commit is unknown here -------------------------------------
# The build job's own checkout decides whether it exists; the guard cannot.
expect "unknown resolved sha passes" "$(run_guard "$UNKNOWN" "$C" "$REPO")" 0

# --- 10. an abbreviated sha still relates ---------------------------------------
expect "short resolved sha still fires on a rollback" "$(run_guard "${B:0:12}" "$C" "$REPO")" 1

# --- 11. misuse is refused, never read as a verdict -----------------------------
expect "missing resolved sha is misuse" "$(run_guard "" "$C" "$REPO")" 2
expect "malformed resolved sha is misuse" "$(run_guard "main" "$C" "$REPO")" 2
expect "a path with no repository is misuse" "$(run_guard "$B" "$C" "$NOREPO")" 2

echo
if [ "$failures" -gt 0 ]; then
  echo "❌ $failures case(s) failed"
  exit 1
fi
echo "✅ all cases passed"
