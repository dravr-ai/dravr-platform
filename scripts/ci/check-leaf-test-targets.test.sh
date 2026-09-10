#!/usr/bin/env bash
# ABOUTME: Pins check-leaf-test-targets.sh — one catch case, one clean case, one fail-closed case
# ABOUTME: Builds a throwaway crate rather than the workspace, so it runs in seconds with no deps
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -uo pipefail

CHECK="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-leaf-test-targets.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILURES=0

pass() { echo "  ok   — $1"; }
fail() { echo "  FAIL — $1"; FAILURES=$((FAILURES + 1)); }

scaffold() {
    local gate="$1"
    rm -rf "$WORK/probe"
    mkdir -p "$WORK/probe/src" "$WORK/probe/tests"
    cat > "$WORK/probe/Cargo.toml" <<EOF
[package]
name = "leaftestprobe"
version = "0.1.0"
edition = "2021"

[features]
gated = []

[[test]]
name = "needs_gate"
required-features = ["$gate"]
EOF
    cat > "$WORK/probe/src/lib.rs" <<'EOF'
#[cfg(feature = "gated")]
#[must_use]
pub fn gated_value() -> u32 { 7 }
EOF
    cat > "$WORK/probe/tests/needs_gate.rs" <<'EOF'
#[test]
fn reads_the_gated_item() { assert_eq!(leaftestprobe::gated_value(), 7); }
EOF
    cat > "$WORK/probe/tests/always.rs" <<'EOF'
#[test]
fn always_builds() { assert_eq!(1 + 1, 2); }
EOF
}

echo "check-leaf-test-targets.test.sh"

# 1. A satisfiable gate: both targets build, the check is silent and green.
scaffold "gated"
OUT="$(cd "$WORK/probe" && "$CHECK" -p leaftestprobe 2>&1)"
STATUS=$?
if [ "$STATUS" -eq 0 ] && printf '%s' "$OUT" | grep -q "2 declared test targets, all 2 built"; then
    pass "satisfiable required-features passes and counts both targets"
else
    fail "satisfiable required-features should pass (exit $STATUS)"
    printf '%s\n' "$OUT" | sed 's/^/       /'
fi

# 2. A gate no profile can satisfy — the silent-skip regression this exists for.
#    cargo does not error on it; the target simply never builds.
scaffold "gated-renamed-away"
OUT="$(cd "$WORK/probe" && "$CHECK" -p leaftestprobe 2>&1)"
STATUS=$?
if [ "$STATUS" -ne 0 ] && printf '%s' "$OUT" | grep -q "needs_gate"; then
    pass "unsatisfiable required-features fails and names the skipped target"
else
    fail "unsatisfiable required-features should fail naming needs_gate (exit $STATUS)"
    printf '%s\n' "$OUT" | sed 's/^/       /'
fi

# 3. Fail closed: asked to verify nothing, it refuses rather than reporting green.
OUT="$("$CHECK" 2>&1)"
STATUS=$?
if [ "$STATUS" -ne 0 ] && printf '%s' "$OUT" | grep -q "refusing to report green"; then
    pass "no packages passed is a refusal, not a pass"
else
    fail "empty package list should fail closed (exit $STATUS)"
fi

echo ""
if [ "$FAILURES" -ne 0 ]; then
    echo "check-leaf-test-targets.test.sh: $FAILURES case(s) failed"
    exit 1
fi
echo "check-leaf-test-targets.test.sh: all cases passed"
