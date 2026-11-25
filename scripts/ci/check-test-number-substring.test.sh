#!/usr/bin/env bash
# ABOUTME: Pins check-test-number-substring.sh — a negated bare-number substring fails, the structured forms,
# ABOUTME: positive checks and a marked token search pass, and a missing path fails closed
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -uo pipefail

CHECK="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-test-number-substring.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILURES=0

pass() { echo "  ok   — $1"; }
fail() { echo "  FAIL — $1"; FAILURES=$((FAILURES + 1)); }

# A throwaway repository holding one Rust file whose body is $1.
probe() {
    rm -rf "$WORK/probe"
    mkdir -p "$WORK/probe/crates/x/tests"
    git -C "$WORK/probe" init -q
    printf '%s\n' "$1" > "$WORK/probe/crates/x/tests/it.rs"
    git -C "$WORK/probe" add -A
}

run_check() {
    ( cd "$WORK/probe" && "$CHECK" crates 2>&1 )
}

expect() {
    local label="$1" want_status="$2" needle="$3"
    OUT="$(run_check)"; STATUS=$?
    if [ "$STATUS" -eq "$want_status" ] && { [ -z "$needle" ] || echo "$OUT" | grep -qF -- "$needle"; }; then
        pass "$label"
    else
        fail "$label (status $STATUS, want $want_status)"
        echo "        ${OUT//$'\n'/$'\n'        }"
    fi
}

probe 'fn t() { assert!(!text.contains("187"), "{text}"); }'
expect "a negated bare integer fails" 1 "it.rs:1"

probe 'fn t() { assert!(!text.contains("33.75")); }'
expect "a negated decimal fails" 1 "it.rs:1"

probe 'fn t() { assert!(!text.contains("-80")); }'
expect "a negated negative number fails" 1 "it.rs:1"

probe 'fn t() {
    assert!(
        !by_id.to_string().contains("120"),
        "bug"
    );
}'
expect "a negation on the line above the call fails" 1 "it.rs:3"

probe 'fn t() { assert!(asked.iter().all(|m| !m.contains("42.0"))); }'
expect "a negated call inside a closure fails" 1 "it.rs:1"

probe 'fn t() { assert!(!asked.iter().any(|m| m.contains("42"))); }'
expect "a negated any(..) fails" 1 "it.rs:1"

probe 'fn t() { assert!(!a.contains("93.5") && !b.contains("97.5")); }'
expect "each side of a negated conjunction fails" 1 "it.rs:1"

probe 'fn t() { assert!(!mentions_number(&text, "187"), "{text}"); }'
expect "the whole-number helper passes" 0 "No test asserts"

probe 'fn t() { assert!(text.contains("187"), "{text}"); }'
expect "a positive check is not scanned" 0 "No test asserts"

probe 'fn t() { assert!(!text.contains("Sécret"), "{text}"); }'
expect "a negated word passes" 0 "No test asserts"

probe 'fn t() { assert!(!list.is_empty() && text.contains("42")); }'
expect "a negation on another operand does not taint a positive one" 0 "No test asserts"

probe 'fn t() { assert!(count != 4 && text.contains("42")); }'
expect "!= is not a negation" 0 "No test asserts"

probe 'fn t() {
    // number-substring-ok: a hash is one hex token
    assert!(!digest.contains("12345678"));
}'
expect "a marked token search passes" 0 "No test asserts"

probe 'fn t() {
    // number-substring-ok: a hash is one hex token



    assert!(!digest.contains("12345678"));
}'
expect "a marker far above does not cover the call" 1 "it.rs:6"

probe '//! a plain `!text.contains("31.7")` reads the clock'
expect "a comment quoting the pattern passes" 0 "No test asserts"

rm -rf "$WORK/probe"
OUT="$( cd "$WORK" && "$CHECK" no-such-dir 2>&1 )"; STATUS=$?
if [ "$STATUS" -eq 1 ] && echo "$OUT" | grep -q MISSING; then
    pass "a missing path fails closed"
else
    fail "a missing path fails closed (status $STATUS)"
fi

probe 'fn t() {}'
git -C "$WORK/probe" rm -q -r --cached . >/dev/null
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -eq 1 ] && echo "$OUT" | grep -q "nothing was verified"; then
    pass "a scan that reads no tracked file fails closed"
else
    fail "a scan that reads no tracked file fails closed (status $STATUS)"
fi

if [ "$FAILURES" -ne 0 ]; then
    echo "$FAILURES case(s) failed"
    exit 1
fi
echo "all cases passed"
