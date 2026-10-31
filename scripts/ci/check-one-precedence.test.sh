#!/usr/bin/env bash
# ABOUTME: Fixture test for check-one-precedence.sh (guard G4) — makes it fire, pass and fail closed
# ABOUTME: A retired priority table or a canonical_order outside rank_activity_copy fails; a test module does not
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$SCRIPT_DIR/check-one-precedence.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILURES=0

pass() { echo "  ok   — $1"; }
fail() { echo "  FAIL — $1"; FAILURES=$((FAILURES + 1)); }

# The activity merge ranking its copies, and a reader.
scaffold() {
    rm -rf "$WORK/root"
    mkdir -p "$WORK/root/crates/pierre-providers/src/deduplication" "$WORK/root/crates/pierre-services/src"
    cat > "$WORK/root/crates/pierre-providers/src/deduplication/detection.rs" <<'RS'
fn rank_activity_copy(a: &Activity, b: &Activity) -> Ordering {
    rank(Kind::Activity, &activity_source(a), &activity_source(b)).then_with(|| canonical_order(a, b))
}
fn canonical_order(a: &Activity, b: &Activity) -> Ordering { a.id().cmp(b.id()) }
fn lift_edited_strava_copy(members: &mut Vec<usize>, p: usize) {
    let row = members.remove(p);
    members.insert(0, row);
}
RS
    printf 'pub fn read() {}\n' > "$WORK/root/crates/pierre-services/src/reader.rs"
}

run() { # prints the exit status; output in $WORK/out
    local code=0
    bash "$CHECK" "$WORK/root" >"$WORK/out" 2>&1 || code=$?
    echo "$code"
}

echo "check-one-precedence.test.sh"

scaffold
code="$(run)"
if [ "$code" -eq 0 ]; then
    pass "a merge ranking its copies through the one rank passes"
else
    fail "a clean tree should pass (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

expect_fires() { # name, snippet appended to the reader
    scaffold
    printf '%s\n' "$2" >> "$WORK/root/crates/pierre-services/src/reader.rs"
    local code
    code="$(run)"
    if [ "$code" -eq 1 ] && grep -q "reader.rs:2:" "$WORK/out"; then
        pass "$1 fails and is named"
    else
        fail "$1 should fail (exit $code)"; sed 's/^/       /' "$WORK/out"
    fi
}
expect_fires "a provider priority table" 'fn pick(a: &str) -> u8 { ProviderPriority::priority(a) }'
expect_fires "a recovery priority" 'fn pick(a: &str) -> u8 { ProviderPriority::recovery_priority(a) }'
expect_fires "a device priority" 'fn pick(d: &DeviceType) -> u8 { DevicePriority::priority(d) }'
expect_fires "resolve_duplicate" 'fn pick<'"'"'a>(a: &'"'"'a S, b: &'"'"'a S) -> &'"'"'a S { resolve_duplicate(a, b) }'
expect_fires "canonical_order outside the merge" 'fn order(a: &A, b: &A) -> Ordering { canonical_order(a, b) }'

scaffold
printf 'fn sort(m: &mut [A]) { m.sort_by(|a, b| canonical_order(a, b)); }\n' \
    >> "$WORK/root/crates/pierre-providers/src/deduplication/detection.rs"
code="$(run)"
if [ "$code" -eq 1 ] && grep -q "named 3 time" "$WORK/out"; then
    pass "a second sort by canonical_order alone in the merge fails"
else
    fail "a bypass of the rank in the merge should fail (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
printf 'fn lift_native_twins(members: &mut Vec<usize>, p: usize, t: usize) { let row = members.remove(p); members.insert(t, row); }\n' \
    >> "$WORK/root/crates/pierre-providers/src/deduplication/detection.rs"
code="$(run)"
if [ "$code" -eq 1 ] && grep -q "moved after the rank 2 time" "$WORK/out"; then
    pass "a second reorder of a group after the rank fails"
else
    fail "a second reorder after the rank should fail (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
cat >> "$WORK/root/crates/pierre-services/src/reader.rs" <<'RS'

#[cfg(test)]
mod tests {
    #[test]
    fn old_table() { let _ = ProviderPriority::priority("garmin"); }
}
RS
code="$(run)"
if [ "$code" -eq 0 ]; then
    pass "a name inside a trailing test module is not production code"
else
    fail "a test-module name should pass (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
rm "$WORK/root/crates/pierre-providers/src/deduplication/detection.rs"
code="$(run)"
if [ "$code" -eq 2 ]; then
    pass "a missing merge fails closed"
else
    fail "a missing merge should exit 2 (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
printf 'fn canonical_order(a: &A, b: &A) -> Ordering { a.cmp(b) }\nfn s(m: &mut [A]) { m.sort_by(|a, b| canonical_order(a, b)); }\n' \
    > "$WORK/root/crates/pierre-providers/src/deduplication/detection.rs"
code="$(run)"
if [ "$code" -eq 2 ]; then
    pass "a merge that no longer ranks with the one rank fails closed"
else
    fail "a merge without the rank should exit 2 (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

if [ "$FAILURES" -gt 0 ]; then
    echo "check-one-precedence.test.sh: $FAILURES failure(s)"
    exit 1
fi
echo "check-one-precedence.test.sh: all passed"
