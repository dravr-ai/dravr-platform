#!/usr/bin/env bash
# ABOUTME: Fixture test for check-inline-paths.sh — builds throwaway repos and asserts each verdict
# ABOUTME: Pins that a 2-segment `crate::name!` passes under every awk, and a 3-segment inline path fails
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# clippy::absolute_paths denies a path of three or more segments, so the gate
# must flag `std::env::set_var(..)` and must not flag `crate::declare_security!`.
# The second case is the one mawk broke: it ignored the `{2,}` interval and
# reported every 2-segment `crate::` macro call, blocking pushes on Linux.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/check-inline-paths.sh}"
OUT="$(mktemp)"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

# A repo whose HEAD adds `crates/demo/src/lib.rs` with the given body.
repo_with() {
  local dir
  dir="$(mktemp -d)"
  git -C "$dir" init -q
  git -C "$dir" config user.email test@example.com
  git -C "$dir" config user.name test
  printf 'base\n' >"$dir/README"
  git -C "$dir" add -A
  git -C "$dir" commit -qm base
  mkdir -p "$dir/crates/demo/src"
  printf '%s\n' "$1" >"$dir/crates/demo/src/lib.rs"
  git -C "$dir" add -A
  git -C "$dir" commit -qm change
  printf '%s\n' "$dir"
}

run_gate() {
  local code=0
  ( cd "$1" && "$UNDER_TEST" HEAD~1 >"$OUT" 2>&1 ) || code=$?
  echo "$code"
}

expect() { # $1 = label, $2 = actual exit, $3 = expected exit
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1 (exit $2, expected $3)"
    sed 's/^/      /' "$OUT"
  fi
}

echo "check-inline-paths.sh fixture tests"

dir="$(repo_with 'crate::declare_security!(GetTool => UNTRUSTED_OUTPUT);')"
expect "a 2-segment crate:: macro call passes" "$(run_gate "$dir")" 0
rm -rf "$dir"

dir="$(repo_with 'fn f() { std::env::set_var("A", "B"); }')"
expect "a 3-segment std:: inline path fails" "$(run_gate "$dir")" 1
if grep -q 'lib.rs:1: std::env::set_var' "$OUT"; then
  pass "the report names the path it found"
else
  fail "the report names the path it found"
  sed 's/^/      /' "$OUT"
fi
rm -rf "$dir"

dir="$(repo_with 'use std::env::set_var;')"
expect "a use declaration is not an inline path" "$(run_gate "$dir")" 0
rm -rf "$dir"

rm -f "$OUT"
if [ "$failures" -gt 0 ]; then
  echo "❌ $failures check-inline-paths.sh case(s) failed"
  exit 1
fi
echo "✅ check-inline-paths.sh fixture tests passed"
