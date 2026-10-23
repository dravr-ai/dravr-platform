#!/usr/bin/env bash
# ABOUTME: Fixture test for bench-ab.sh — base and head bench on one machine, suite by suite, base first
# ABOUTME: A suite only the head has runs on the head alone; a bad base or suite is refused before anything runs
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# A `cargo` stub on PATH records the directory it ran in and its arguments.
# What is under test is which tree benches which suite, in what order, with
# which baseline flags, never Criterion itself.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/bench-ab.sh}"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
WORK="$(cd "$WORK" && pwd -P)"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

mkdir -p "$WORK/bin" "$WORK/repo/crates/pierre-server/benches"
cat >"$WORK/bin/cargo" <<'STUB'
#!/bin/sh
echo "$(basename "$PWD") $*" >>"$CARGO_LOG"
cat .cargo/config.toml 2>/dev/null | sed "s/^/$(basename "$PWD") config: /" >>"$CARGO_CONFIG_LOG"
STUB
chmod +x "$WORK/bin/cargo"

REPO="$WORK/repo"
git -C "$REPO" init -q -b main
git -C "$REPO" config user.email fixture@example.com
git -C "$REPO" config user.name fixture
git -C "$REPO" config commit.gpgsign false
: >"$REPO/crates/pierre-server/benches/cache_bench.rs"
: >"$REPO/crates/pierre-server/benches/tool_dispatch_bench.rs"
git -C "$REPO" add -A && git -C "$REPO" commit -qm base
BASE="$(git -C "$REPO" rev-parse HEAD)"
: >"$REPO/crates/pierre-server/benches/guardian_bench.rs"
git -C "$REPO" add -A && git -C "$REPO" commit -qm head
# Untracked, as CI writes it: the build flags both sides must share.
mkdir -p "$REPO/.cargo"
echo 'rustflags = ["-C", "llvm-args=-align-all-functions=6"]' >"$REPO/.cargo/config.toml"

LOG="$WORK/cargo.log"
CONFIG_LOG="$WORK/cargo-config.log"
OUT="$WORK/out"

# $1 = label, $2 = expected exit (0 or nonzero), $3.. = args.
expect() {
    local label="$1" want="$2" code=0
    shift 2
    : >"$LOG"
    : >"$CONFIG_LOG"
    (cd "$REPO" && env PATH="$WORK/bin:$PATH" CARGO_LOG="$LOG" CARGO_CONFIG_LOG="$CONFIG_LOG" BENCH_BASE_TREE="$WORK/bench-base" \
        CARGO_TARGET_DIR="$WORK/target" CRITERION_HOME= RUNNER_TEMP= "$UNDER_TEST" "$@") >"$OUT" 2>&1 || code=$?
    if { [ "$want" = 0 ] && [ "$code" -eq 0 ]; } || { [ "$want" != 0 ] && [ "$code" -ne 0 ]; }; then
        pass "$label"
    else
        fail "$label (exit $code)"
        sed 's/^/      /' "$OUT"
    fi
}

echo "bench-ab.sh fixture tests"

expect "a run over every suite passes" 0 "$BASE"
want="bench-base bench -p pierre_mcp_server --bench cache_bench -- --save-baseline base
repo bench -p pierre_mcp_server --bench cache_bench -- --baseline-lenient base
repo bench -p pierre_mcp_server --bench guardian_bench
bench-base bench -p pierre_mcp_server --bench tool_dispatch_bench -- --save-baseline base
repo bench -p pierre_mcp_server --bench tool_dispatch_bench -- --baseline-lenient base"
if [ "$(cat "$LOG")" = "$want" ]; then
    pass "each suite benches on the base, then on the head against it, one suite at a time"
else
    fail "each suite benches on the base, then on the head against it, one suite at a time"
    sed 's/^/      /' "$LOG"
fi
if grep -qF 'bench-base config: rustflags = ["-C", "llvm-args=-align-all-functions=6"]' "$CONFIG_LOG" \
    && grep -qF 'repo config: rustflags = ["-C", "llvm-args=-align-all-functions=6"]' "$CONFIG_LOG"; then
    pass "the base builds under the checkout's cargo config, as the head does"
else
    fail "the base builds under the checkout's cargo config, as the head does"
    sed 's/^/      /' "$CONFIG_LOG"
fi
if grep -qF "Suites with no base to compare against: guardian_bench" "$OUT"; then
    pass "a suite the base lacks is named as head-only"
else
    fail "a suite the base lacks is named as head-only"
    sed 's/^/      /' "$OUT"
fi
if [ ! -e "$WORK/bench-base" ] && ! git -C "$REPO" worktree list | grep -q bench-base; then
    pass "the base worktree is removed afterwards"
else
    fail "the base worktree is removed afterwards"
fi

expect "one named suite runs alone" 0 "$BASE" cache_bench
if [ "$(wc -l <"$LOG" | tr -d ' ')" = 2 ] && ! grep -q tool_dispatch "$LOG"; then
    pass "only the named suite ran"
else
    fail "only the named suite ran"
    sed 's/^/      /' "$LOG"
fi

expect "a suite the head lacks is refused" 1 "$BASE" no_such_bench
expect "a base commit that is not in the clone is refused" 1 0123456789abcdef0123456789abcdef01234567
if [ ! -s "$LOG" ]; then pass "a refused run benches nothing"; else
    fail "a refused run benches nothing"
    sed 's/^/      /' "$LOG"
fi
expect "no arguments is refused" 1

echo ""
if [ "$failures" -gt 0 ]; then
    echo "❌ $failures case(s) failed"
    exit 1
fi
echo "✅ all bench-ab.sh cases passed"
