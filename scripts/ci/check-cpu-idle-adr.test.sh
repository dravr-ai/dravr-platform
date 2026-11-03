#!/usr/bin/env bash
# ABOUTME: Fixture test for check-cpu-idle-adr.sh — builds throwaway infra/ trees and asserts each verdict
# ABOUTME: Pins that cpu_idle=false passes with an ADR or a reasoned cpu-idle-ok line, and fails with neither
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/check-cpu-idle-adr.sh}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
OUT="$WORK/out"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

tree_with() { # $1 = contents of infra/environments/dev/service.tf
  local dir
  dir="$(mktemp -d "$WORK/tree.XXXXXX")"
  mkdir -p "$dir/infra/environments/dev"
  printf '%s\n' "$1" >"$dir/infra/environments/dev/service.tf"
  printf '%s\n' "$dir"
}

run_gate() {
  local code=0
  bash "$UNDER_TEST" "$1" >"$OUT" 2>&1 || code=$?
  echo "$code"
}

expect() { # $1 = label, $2 = actual exit, $3 = expected exit
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1 (exit $2, expected $3)"
    sed 's/^/      /' "$OUT"
  fi
}

echo "check-cpu-idle-adr.sh"

expect "an ADR reference above cpu_idle=false passes" \
  "$(run_gate "$(tree_with '  # The ACP runner owns this floor (ADR-019).
  cpu_idle = false')")" 0

expect "a cpu-idle-ok line with a reason passes" \
  "$(run_gate "$(tree_with '  # cpu-idle-ok: Active Assist measured allocated CPU ~$14/mo cheaper here.
  cpu_idle = false')")" 0

expect "a cpu-idle-ok line with no reason fails" \
  "$(run_gate "$(tree_with '  # cpu-idle-ok:
  cpu_idle = false')")" 1

expect "cpu_idle=false with neither fails" \
  "$(run_gate "$(tree_with '  # Always-on CPU.
  cpu_idle = false')")" 1

expect "cpu_throttling=false is held to the same rule" \
  "$(run_gate "$(tree_with '  cpu_throttling = false')")" 1

expect "a comment that only mentions cpu_idle = false is not an assignment" \
  "$(run_gate "$(tree_with '  # we once ran cpu_idle = false here
  cpu_idle = true')")" 0

expect "a root with no infra/ fails closed" \
  "$(run_gate "$(mktemp -d "$WORK/empty.XXXXXX")")" 1

if [ "$failures" -gt 0 ]; then
  echo "❌ check-cpu-idle-adr.test.sh: $failures case(s) failed"
  exit 1
fi
echo "✅ check-cpu-idle-adr.test.sh: every case passed"
