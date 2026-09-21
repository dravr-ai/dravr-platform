#!/usr/bin/env bash
# ABOUTME: Fixture test for pick-deploy-recovery.sh — a throwaway main with a red tip, proving the
# ABOUTME: green commit behind it is named, and that every doubtful shape names nothing
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# publish-images.yml only runs on main, so the recovery it starts cannot be
# exercised on a branch; this is its verification before it meets a real red
# commit. The case that matters is the first: a red commit displaced a green
# one, and the green one must be named. Every other case must name NOTHING —
# this script starts a deploy nobody asked for, so a wrong answer moves dev.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="$SCRIPT_DIR/pick-deploy-recovery.sh"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

REPO="$(mktemp -d)"
TABLE="$(mktemp)"
STUB="$(mktemp)"
ERR="$(mktemp)"
trap 'rm -rf "$REPO" "$TABLE" "$STUB" "$ERR"' EXIT

# The verdict stub reads "sha conclusion" lines; a sha marked BROKEN fails the read.
cat > "$STUB" <<STUBEOF
#!/usr/bin/env bash
line="\$(grep "^\$1 " "$TABLE" || true)"
verdict="\${line#* }"
[ "\$verdict" = "BROKEN" ] && exit 1
printf '%s\n' "\$verdict"
STUBEOF
chmod +x "$STUB"

git -C "$REPO" init -q
git -C "$REPO" config user.email test@example.com
git -C "$REPO" config user.name test
commit() { echo "$1" > "$REPO/$1"; git -C "$REPO" add -A; git -C "$REPO" commit -q -m "$1"; git -C "$REPO" rev-parse HEAD; }

#   A --- B --- C --- D --- E     (main)       X diverged from A
A="$(commit A)"; B="$(commit B)"; C="$(commit C)"; D="$(commit D)"; E="$(commit E)"
git -C "$REPO" checkout -q -b side "$A"; X="$(commit X)"; git -C "$REPO" checkout -q -

pick() { CI_VERDICT_CMD="$STUB" "$UNDER_TEST" "$1" "$2" "$REPO" 2>"$ERR"; }
verdicts() { : > "$TABLE"; while [ $# -gt 0 ]; do echo "$1 $2" >> "$TABLE"; shift 2; done; }
expect() { if [ "$2" = "$3" ]; then pass "$1"; else fail "$1 (got '$2', wanted '$3')"; sed 's/^/       /' "$ERR"; fi; }

echo "pick-deploy-recovery.sh"

verdicts "$A" success "$B" success "$C" success "$D" success "$E" failure
expect "a red tip names the green commit right behind it" "$(pick "$E" "$B")" "$D"

verdicts "$A" success "$B" success "$C" success "$D" failure "$E" failure
expect "two reds in a row: the walk goes past both" "$(pick "$E" "$B")" "$C"

verdicts "$A" success "$B" success "$C" success "$E" failure
expect "a commit with no CI run (path-filtered) is skipped, never deployed unvalidated" "$(pick "$E" "$B")" "$C"

verdicts "$A" success "$B" success "$C" success "$D" success "$E" failure
expect "dev already serves the newest green commit: nothing" "$(pick "$E" "$D")" ""
expect "dev serves something newer than the green commit: nothing" "$(pick "$D" "$E")" ""
expect "what dev serves is unknown: nothing, no deploy on a guess" "$(pick "$E" "")" ""
expect "what dev serves is not a commit git knows: nothing" "$(pick "$E" "deadbeefdeadbeef")" ""
expect "dev serves a diverged commit: nothing" "$(pick "$E" "$X")" ""

verdicts "$A" failure "$B" failure "$C" failure "$D" failure "$E" failure
expect "no green commit behind the refused one: nothing" "$(pick "$E" "$A")" ""

verdicts "$A" success "$B" success "$C" success "$D" BROKEN "$E" failure
expect "an unreadable verdict is not read as red and skipped: nothing" "$(pick "$E" "$B")" ""

code=0; CI_VERDICT_CMD="$STUB" "$UNDER_TEST" "not-a-sha" "$B" "$REPO" >/dev/null 2>&1 || code=$?
expect "a refused sha that is not one is misuse (exit 2)" "$code" "2"

echo
if [ "$failures" -gt 0 ]; then echo "❌ $failures case(s) failed"; exit 1; fi
echo "✅ all cases passed"
