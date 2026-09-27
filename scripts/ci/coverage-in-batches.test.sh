#!/usr/bin/env bash
# ABOUTME: Pins coverage-in-batches.sh — every batch's counts reach one well-formed lcov file, failures fail the run
# ABOUTME: Builds a throwaway two-crate workspace rather than the real one, so it runs in seconds with no deps
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -uo pipefail

RUN="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/coverage-in-batches.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILURES=0

pass() { echo "  ok   — $1"; }
fail() { echo "  FAIL — $1"; FAILURES=$((FAILURES + 1)); }

# Crate `a` has a unit test, a bin an integration test spawns, a target gated
# on a default-on feature, one gated on an off feature, and a `test = false`
# target that panics if it ever runs. Crate `b` shares a test name with `a`,
# so one `--test shared` must build and run both. Every function name is a
# substring of no other, so a hit on one can never stand in for another.
scaffold() {
    rm -rf "$WORK/ws"
    mkdir -p "$WORK/ws/a/src/bin" "$WORK/ws/a/tests" "$WORK/ws/b/src" "$WORK/ws/b/tests"
    cat >"$WORK/ws/Cargo.toml" <<'EOF'
[workspace]
resolver = "2"
members = ["a", "b"]
EOF
    cat >"$WORK/ws/a/Cargo.toml" <<'EOF'
[package]
name = "a"
version = "0.1.0"
edition = "2021"

[features]
default = ["on"]
on = []
off = []

[[test]]
name = "gated_on"
required-features = ["on"]

[[test]]
name = "gated_off"
required-features = ["off"]

[[test]]
name = "parked"
test = false
EOF
    cat >"$WORK/ws/b/Cargo.toml" <<'EOF'
[package]
name = "b"
version = "0.1.0"
edition = "2021"
EOF
    cat >"$WORK/ws/a/src/lib.rs" <<'EOF'
pub fn alpha_unit(x: i32) -> i32 { x + 1 }
pub fn bravo_first(x: i32) -> i32 { x * 2 }
pub fn charlie_gated(x: i32) -> i32 { x * 3 }
pub fn delta_bin(x: i32) -> i32 { x - 100 }
pub fn echo_shared(x: i32) -> i32 { x + 5 }
#[cfg(test)]
mod tests {
    #[test]
    fn unit() { assert_eq!(super::alpha_unit(1), 2); }
}
EOF
    echo 'fn main() { println!("{}", a::delta_bin(200)); }' >"$WORK/ws/a/src/bin/serve.rs"
    echo '#[test] fn first() { assert_eq!(a::bravo_first(2), 4); }' >"$WORK/ws/a/tests/first.rs"
    echo '#[test] fn on() { assert_eq!(a::charlie_gated(2), 6); }' >"$WORK/ws/a/tests/gated_on.rs"
    echo '#[test] fn off() { assert_eq!(1, 1); }' >"$WORK/ws/a/tests/gated_off.rs"
    echo '#[test] fn parked() { panic!("a test = false target must never run"); }' >"$WORK/ws/a/tests/parked.rs"
    echo '#[test] fn shared() { assert_eq!(a::echo_shared(1), 6); }' >"$WORK/ws/a/tests/shared.rs"
    cat >"$WORK/ws/a/tests/spawns_bin.rs" <<'EOF'
#[test]
fn spawns() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_serve")).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "100");
}
EOF
    echo 'pub fn foxtrot_b(x: i32) -> i32 { x + 7 }' >"$WORK/ws/b/src/lib.rs"
    echo '#[test] fn shared() { assert_eq!(b::foxtrot_b(1), 8); }' >"$WORK/ws/b/tests/shared.rs"
}

ALL_FNS="alpha_unit bravo_first charlie_gated delta_bin echo_shared foxtrot_b"

# Names of the functions in $ALL_FNS that the lcov file at $1 never records as hit.
missing_hits() {
    local fn missing=""
    for fn in $ALL_FNS; do
        grep -qE "^FNDA:[1-9][0-9]*,.*${fn}\$" "$1" || missing="$missing $fn"
    done
    echo "$missing"
}

# Test executables left in the deps dir under the build root $1.
leftover_tests() {
    find "$1/llvm-cov-target/debug/deps" -maxdepth 1 -type f -perm -u+x ! -name '*.*' -links 1 2>/dev/null
}

echo "coverage-in-batches.test.sh"

# 1. Batches of two: five runnable targets under four names split over two
#    batches after the unit tests, and every function any batch reached is
#    counted, in both crates. The parked target would panic, so exit 0 also
#    proves it never ran.
scaffold
OUT="$(cd "$WORK/ws" && COVERAGE_BATCH_SIZE=2 "$RUN" "$WORK/lcov.info" -- --test-threads=1 2>&1)"
STATUS=$?
MISSING="$(missing_hits "$WORK/lcov.info")"
if [ "$STATUS" -eq 0 ] && [ -z "$MISSING" ] &&
    printf '%s' "$OUT" | grep -q "5 test targets under 4 names, in 2 batch(es) of up to 2 names"; then
    pass "every batch's counts reach the one lcov file, for both crates"
else
    fail "expected exit 0, 5 targets under 4 names in 2 batches, and a hit for every function (exit $STATUS, missing:${MISSING:- none})"
    printf '%s\n' "$OUT" | tail -40 | sed 's/^/       /'
fi

# 2. The targets `--tests` would skip are skipped here too, and said out loud.
if printf '%s' "$OUT" | grep -q "skipped: a/gated_off has features off" &&
    printf '%s' "$OUT" | grep -q "skipped: a/parked has test = false"; then
    pass "unsatisfied required-features and test = false are skipped and named"
else
    fail "gated_off and parked should both be reported as skipped"
fi

# 3. One record per line: a batch's first SF: must not share a line with the
#    previous batch's end_of_record.
SF_ANYWHERE="$(grep -o 'SF:' "$WORK/lcov.info" | wc -l | tr -d ' ')"
SF_LINES="$(grep -c '^SF:' "$WORK/lcov.info")"
if [ "$SF_ANYWHERE" -gt 0 ] && [ "$SF_ANYWHERE" -eq "$SF_LINES" ] && ! grep -q '^end_of_record.' "$WORK/lcov.info"; then
    pass "the merged lcov file keeps one record per line ($SF_LINES SF records)"
else
    fail "merged lcov has glued records: $SF_ANYWHERE SF: occurrences on $SF_LINES SF lines"
fi

# 4. Nothing a batch linked for testing is left behind.
LEFT="$(leftover_tests "$WORK/ws/target")"
if [ -z "$LEFT" ]; then
    pass "no test executable outlives its batch"
else
    fail "test executables left on disk:"
    printf '%s\n' "$LEFT" | sed 's/^/       /'
fi

# 5. What batches share survives them: the workspace library is compiled once,
#    not cleaned away between batches and rebuilt by the next.
if ls "$WORK/ws/target/llvm-cov-target/debug/deps"/liba-*.rlib >/dev/null 2>&1; then
    pass "the workspace library outlives the batches that reuse it"
else
    fail "liba-*.rlib is gone — something between batches cleaned the workspace artifacts"
fi

# 6. A failing test fails the run, even in the last batch, and no lcov file is
#    written for a run that did not pass.
scaffold
echo '#[test] fn fails() { assert_eq!(1, 2, "deliberate"); }' >"$WORK/ws/a/tests/zz_fails.rs"
OUT="$(cd "$WORK/ws" && COVERAGE_BATCH_SIZE=1 "$RUN" "$WORK/lcov-fail.info" 2>&1)"
STATUS=$?
if [ "$STATUS" -ne 0 ] && [ ! -e "$WORK/lcov-fail.info" ] && printf '%s' "$OUT" | grep -q "deliberate"; then
    pass "a failing test in the last batch fails the run and writes no lcov file"
else
    fail "a failing test should fail the run with no output (exit $STATUS, lcov written: $([ -e "$WORK/lcov-fail.info" ] && echo yes || echo no))"
    printf '%s\n' "$OUT" | tail -15 | sed 's/^/       /'
fi

# 7. A configured build-dir moves the executables away from the target dir;
#    the batcher follows it instead of deleting nothing.
scaffold
OUT="$(cd "$WORK/ws" && CARGO_BUILD_BUILD_DIR="$WORK/bd" COVERAGE_BATCH_SIZE=2 "$RUN" "$WORK/lcov-bd.info" 2>&1)"
STATUS=$?
MISSING="$(missing_hits "$WORK/lcov-bd.info" 2>/dev/null)"
LEFT="$(leftover_tests "$WORK/bd")"
if [ "$STATUS" -eq 0 ] && [ -z "$MISSING" ] && [ -z "$LEFT" ] && [ -d "$WORK/bd/llvm-cov-target/debug/deps" ]; then
    pass "a configured build-dir is followed: full coverage, no executables left there"
else
    fail "build-dir run: exit $STATUS, missing:${MISSING:- none}, left:${LEFT:- none}"
    printf '%s\n' "$OUT" | tail -15 | sed 's/^/       /'
fi

# 8. `--tests` would also test an example or bench marked `test = true`; the
#    batcher does not select those, so it refuses rather than dropping them.
scaffold
mkdir -p "$WORK/ws/a/examples"
echo 'fn main() {}' >"$WORK/ws/a/examples/demo.rs"
printf '\n[[example]]\nname = "demo"\ntest = true\n' >>"$WORK/ws/a/Cargo.toml"
OUT="$(cd "$WORK/ws" && "$RUN" "$WORK/lcov-example.info" 2>&1)"
STATUS=$?
if [ "$STATUS" -ne 0 ] && printf '%s' "$OUT" | grep -q "a/demo (example)"; then
    pass "an example with test = true is a refusal naming it"
else
    fail "an example with test = true should be refused (exit $STATUS)"
    printf '%s\n' "$OUT" | tail -10 | sed 's/^/       /'
fi

# 9. Fail closed: a workspace with no test targets is a refusal, not a pass.
scaffold
rm -rf "$WORK/ws/a/tests" "$WORK/ws/b/tests"
sed -i.bak '/^\[\[test\]\]/,$d' "$WORK/ws/a/Cargo.toml"
OUT="$(cd "$WORK/ws" && "$RUN" "$WORK/lcov-empty.info" 2>&1)"
STATUS=$?
if [ "$STATUS" -ne 0 ] && printf '%s' "$OUT" | grep -q "refusing to report green"; then
    pass "no test targets is a refusal, not a pass"
else
    fail "an empty target list should fail closed (exit $STATUS)"
    printf '%s\n' "$OUT" | tail -10 | sed 's/^/       /'
fi

echo ""
if [ "$FAILURES" -ne 0 ]; then
    echo "coverage-in-batches.test.sh: $FAILURES case(s) failed"
    exit 1
fi
echo "coverage-in-batches.test.sh: all cases passed"
