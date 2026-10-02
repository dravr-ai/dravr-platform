#!/usr/bin/env bash
# ABOUTME: Fixture test for pick-newest-green.sh — a throwaway main where CI finished out of order,
# ABOUTME: proving the newest green commit is named and that every doubtful candidate is ignored
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# publish-images.yml only runs on main, so the picker cannot be exercised on a
# branch; this is its verification before it meets a real deploy. The case that
# matters is the first: an older commit's late CI started the deploy, and the
# newer green commit must be built instead. Every other case must fall back
# toward the trigger, never past it and never off main's line.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="$SCRIPT_DIR/pick-newest-green.sh"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

REPO="$(mktemp -d)"
NOREPO="$(mktemp -d)"
ERR="$(mktemp)"
trap 'rm -rf "$REPO" "$NOREPO" "$ERR"' EXIT

git -C "$REPO" init -q
git -C "$REPO" config user.email test@example.com
git -C "$REPO" config user.name test
commit() { echo "$1" > "$REPO/$1"; git -C "$REPO" add -A; git -C "$REPO" commit -q -m "$1"; git -C "$REPO" rev-parse HEAD; }

#   A --- B --- C --- D     (main, HEAD)   X diverged from A
#                      \
#                       Y   (dropped)      descends from D but is not on main
A="$(commit A)"; B="$(commit B)"; C="$(commit C)"; D="$(commit D)"
git -C "$REPO" checkout -q -b side "$A"; X="$(commit X)"; git -C "$REPO" checkout -q -
git -C "$REPO" checkout -q -b dropped "$D"; Y="$(commit Y)"; git -C "$REPO" checkout -q -

pick() { local t="$1"; shift; printf '%s\n' "$@" | "$UNDER_TEST" "$t" "$REPO" 2>"$ERR"; }
expect() { if [ "$2" = "$3" ]; then pass "$1"; else fail "$1 (got '$2', wanted '$3')"; sed 's/^/       /' "$ERR"; fi; }
expect_misuse() {
  local code=0
  printf '%s\n' "$C" | "$UNDER_TEST" "$2" "$3" >/dev/null 2>"$ERR" || code=$?
  if [ "$code" = "2" ]; then pass "$1"; else fail "$1 (exit $code, wanted 2)"; fi
}

echo "pick-newest-green.sh"

expect "a late older trigger builds the newest green commit" "$(pick "$B" "$D" "$C" "$B")" "$D"
expect "input order does not matter" "$(pick "$B" "$B" "$C" "$D")" "$D"
expect "a red newest commit (absent from the green list) is skipped" "$(pick "$B" "$B" "$C")" "$C"
expect "no green candidate at all builds the trigger" "$(pick "$C")" "$C"
expect "an older green commit never wins over the trigger" "$(pick "$C" "$A" "$B")" "$C"
expect "a green commit on another line of history is ignored" "$(pick "$B" "$X")" "$B"
expect "a green commit main no longer holds is ignored" "$(pick "$B" "$Y" "$D")" "$D"
expect "an unknown sha is ignored" "$(pick "$B" "0123456789abcdef0123456789abcdef01234567" "$C")" "$C"
expect "malformed lines are ignored" "$(pick "$B" "not-a-sha" "" "$C")" "$C"
expect "an abbreviated trigger resolves to the full sha" "$(pick "${B:0:9}" "$D")" "$D"

expect_misuse "an empty trigger is misuse" "" "$REPO"
expect_misuse "a malformed trigger is misuse" "XYZ" "$REPO"
expect_misuse "a trigger unknown to the repository is misuse" "0123456789abcdef0123456789abcdef01234567" "$REPO"
expect_misuse "no repository is misuse" "$B" "$NOREPO"

if [ "$failures" -gt 0 ]; then
  echo "$failures case(s) failed"
  exit 1
fi
echo "✅ all cases passed"
