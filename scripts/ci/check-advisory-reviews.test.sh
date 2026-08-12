#!/usr/bin/env bash
# ABOUTME: Fixture test for check-advisory-reviews.sh — proves each branch, so the
# ABOUTME: gate cannot silently pass every deny.toml it is handed.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="$SCRIPT_DIR/check-advisory-reviews.sh"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

PAST="2020-01-01"
FUTURE="2099-12-31"

run_case() { # $1 = fixture body -> echoes exit code
  local f code=0
  f="$(mktemp)"
  printf '%s\n' "$1" >"$f"
  "$UNDER_TEST" "$f" >/tmp/advisory-reviews-test.out 2>&1 || code=$?
  rm -f "$f"
  echo "$code"
}

expect() { # $1 label, $2 actual, $3 expected
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1 (exit $2, expected $3)"
    sed 's/^/      /' /tmp/advisory-reviews-test.out
  fi
}

echo "check-advisory-reviews.sh"

# 1. Dated in the future → passes.
expect "future review date passes" "$(run_case "[advisories]
ignore = [
    # Next Review: $FUTURE
    \"RUSTSEC-2023-0071\",
]")" 0

# 2. Dated in the past → fails. This is the whole point.
expect "expired review date fails" "$(run_case "[advisories]
ignore = [
    # Next Review: $PAST
    \"RUSTSEC-2023-0071\",
]")" 1

# 3. No date at all → fails; silence must not be cheaper than a date.
expect "missing review date fails" "$(run_case "[advisories]
ignore = [
    # Some reasoning but no date.
    \"RUSTSEC-2023-0071\",
]")" 1

# 4. One comment block covering several ids applies to all of them.
expect "shared comment block dates every id it covers" "$(run_case "[advisories]
ignore = [
    # Next Review: $FUTURE
    \"RUSTSEC-2026-0194\",
    \"RUSTSEC-2026-0195\",
]")" 0

# 5. A later block must not inherit an earlier block's date.
expect "a dated block does not cover a later undated one" "$(run_case "[advisories]
ignore = [
    # Next Review: $FUTURE
    \"RUSTSEC-2026-0194\",
]
# unrelated
[licenses]
allow = []")" 0

# 6. Mixed: one good, one expired → fails.
expect "one expired among several fails" "$(run_case "[advisories]
ignore = [
    # Next Review: $FUTURE
    \"RUSTSEC-2026-0194\",
    # Next Review: $PAST
    \"RUSTSEC-2023-0071\",
]")" 1

# 7. Malformed date → fails rather than being skipped.
expect "malformed date fails" "$(run_case "[advisories]
ignore = [
    # Next Review: soon
    \"RUSTSEC-2023-0071\",
]")" 1

# 8. Empty ignore list → passes.
expect "empty ignore list passes" "$(run_case "[advisories]
ignore = [
]")" 0

# 9. An undated id in a LATER block must not inherit an earlier block's date.
#    Without this, a new suppression could be slipped in under an old date.
expect "undated id does not inherit an earlier block's date" "$(run_case "[advisories]
ignore = [
    # Next Review: $FUTURE
    \"RUSTSEC-2026-0194\",

    # A separate block with reasoning but no date of its own.
    \"RUSTSEC-2023-0071\",
]")" 1

# 9. The real deny.toml in this repo must pass.
if [ -f "$(git rev-parse --show-toplevel)/deny.toml" ]; then
  code=0
  "$UNDER_TEST" >/tmp/advisory-reviews-test.out 2>&1 || code=$?
  expect "the repo's own deny.toml passes" "$code" 0
fi

echo ""
if [ "$failures" -gt 0 ]; then
  echo "❌ check-advisory-reviews.test.sh: $failures case(s) failed."
  exit 1
fi
echo "✅ check-advisory-reviews.test.sh: all cases passed."
exit 0
