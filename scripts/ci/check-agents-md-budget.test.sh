#!/usr/bin/env bash
# ABOUTME: Fixture test for check-agents-md-budget.sh — proves the gate fires over budget and passes under it
# ABOUTME: Also pins fail-closed on a missing or empty file, and that the real AGENTS.md is within budget
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="$SCRIPT_DIR/check-agents-md-budget.sh"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/agents-budget.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
OUT="$TMP/out"

failures=0
run() { # $1 = label, $2 = expected exit, $3 = expected output fragment, rest = command
    local label="$1" want="$2" frag="$3" code=0
    shift 3
    "$@" >"$OUT" 2>&1 || code=$?
    if [ "$code" = "$want" ] && grep -qF -- "$frag" "$OUT"; then
        echo "  ✅ $label"
    else
        echo "  ❌ $label (exit $code, expected $want; wanted '$frag')"
        sed 's/^/      /' "$OUT"
        failures=$((failures + 1))
    fi
}

echo "check-agents-md-budget.sh fixture tests"

head -c 100 /dev/zero | tr '\0' 'a' > "$TMP/small.md"
head -c 101 /dev/zero | tr '\0' 'a' > "$TMP/big.md"
: > "$TMP/empty.md"

run "a file at its budget passes" 0 "100 of 100 bytes" env AGENTS_MD_BUDGET=100 "$UNDER_TEST" "$TMP/small.md"
run "a file one byte over fails" 1 "over its 100-byte budget by 1" env AGENTS_MD_BUDGET=100 "$UNDER_TEST" "$TMP/big.md"
run "an empty file fails closed" 1 "nothing was measured" "$UNDER_TEST" "$TMP/empty.md"
run "a missing file fails closed" 1 "nothing was measured" "$UNDER_TEST" "$TMP/absent.md"
run "the repo's AGENTS.md is within the default budget" 0 "✅ AGENTS.md is" "$UNDER_TEST"

if [ "$failures" -gt 0 ]; then
    echo "❌ $failures case(s) failed"
    exit 1
fi
echo "✅ all check-agents-md-budget cases passed"
