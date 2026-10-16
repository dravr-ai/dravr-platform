#!/usr/bin/env bash
# ABOUTME: Fixture test for prod-src-scan.sh — scans throwaway src trees and asserts output and exit status
# ABOUTME: Pins that a crashed rg or a crashed test-module filter exits 2 instead of printing zero matches
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# architectural-validation.sh counts unwrap / expect / panic / NULL-UUID
# findings from prod-src-scan.sh's output and passes on zero. Before
# carnet#927 the scan ran `{ rg … || true; } | filter`, so a moved path or a
# filter that died printed nothing and every one of those gates went green.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/prod-src-scan.sh}"
REAL_FILTER="$SCRIPT_DIR/../../.build/validation/test-module-lines.sh"
WORK="$(mktemp -d)"
OUT="$WORK/out"
trap 'rm -rf "$WORK"' EXIT

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

# Runs the scan from $WORK (the filter opens paths relative to the cwd) with
# the filter named by $1; prints the exit status, output lands in $OUT.
run_scan() {
  local filter="$1" code=0
  shift
  ( cd "$WORK" && TEST_MODULE_LINES="$filter" "$UNDER_TEST" "$@" >"$OUT" 2>&1 ) || code=$?
  echo "$code"
}

expect() { # $1 = label, $2 = actual exit, $3 = expected exit
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1 (exit $2, expected $3)"
    sed 's/^/      /' "$OUT"
  fi
}

expect_out() { # $1 = label, $2 = fixed string the output must contain
  if grep -qF -- "$2" "$OUT"; then pass "$1"; else
    fail "$1"
    sed 's/^/      /' "$OUT"
  fi
}

echo "prod-src-scan.sh fixture tests"

if [ ! -x "$REAL_FILTER" ]; then
  echo "❌ $REAL_FILTER missing — run: git submodule update --init --recursive"
  exit 1
fi

mkdir -p "$WORK/src"
cat >"$WORK/src/lib.rs" <<'RS'
pub fn parse(s: &str) -> u32 {
    s.parse().unwrap()
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses() {
        assert_eq!(super::parse("1"), "1".parse::<u32>().unwrap());
    }
}
RS

expect "a production match exits 0" "$(run_scan "$REAL_FILTER" '\.unwrap\(\)' src/)" 0
expect_out "the production line is printed" "src/lib.rs:2:"
if grep -q 'src/lib.rs:9:' "$OUT"; then
  fail "the test-module line is dropped"
  sed 's/^/      /' "$OUT"
else
  pass "the test-module line is dropped"
fi

expect "no match is a clean, empty result" "$(run_scan "$REAL_FILTER" 'panic!\(' src/)" 0
if [ -s "$OUT" ]; then fail "no match prints nothing"; sed 's/^/      /' "$OUT"; else pass "no match prints nothing"; fi

expect "a missing scan path fails closed" "$(run_scan "$REAL_FILTER" '\.unwrap\(\)' moved/src/)" 2
expect_out "the rg failure is named" "rg failed"

expect "an invalid pattern fails closed" "$(run_scan "$REAL_FILTER" 'unclosed(' src/)" 2

crashing_filter="$WORK/crashing-filter.sh"
printf '#!/bin/sh\ncat >/dev/null\nexit 3\n' >"$crashing_filter"
chmod +x "$crashing_filter"
expect "a crashing filter fails closed" "$(run_scan "$crashing_filter" '\.unwrap\(\)' src/)" 2
expect_out "the filter failure is named" "filter $crashing_filter failed (exit 3)"

expect "a missing filter fails closed" "$(run_scan "$WORK/absent-filter.sh" '\.unwrap\(\)' src/)" 2

if [ "$failures" -gt 0 ]; then
  echo "❌ $failures prod-src-scan.sh case(s) failed"
  exit 1
fi
echo "✅ prod-src-scan.sh fixture tests passed"
