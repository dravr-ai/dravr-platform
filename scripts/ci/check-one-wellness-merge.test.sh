#!/usr/bin/env bash
# ABOUTME: Fixture test for check-one-wellness-merge.sh (guard G3) — makes it fire, pass and fail closed
# ABOUTME: A merge or repository read outside stored_health.rs fails; one inside a trailing test module does not
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$SCRIPT_DIR/check-one-wellness-merge.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILURES=0

pass() { echo "  ok   — $1"; }
fail() { echo "  FAIL — $1"; FAILURES=$((FAILURES + 1)); }

# A two-crate tree: the chokepoint merging and reading, and a reader.
scaffold() {
    rm -rf "$WORK/root"
    mkdir -p "$WORK/root/crates/pierre-services/src" "$WORK/root/crates/pierre-tool-runtime/src"
    cat > "$WORK/root/crates/pierre-services/src/stored_health.rs" <<'RS'
use dravr_equilibre::merge_sleep_sessions;
pub async fn merged_sleep(repos: &Repos) -> Vec<Merged> {
    merge_sleep_sessions(repos.sleep.get_sleep_sessions(1, 2, 3, 4).await)
}
RS
    cat > "$WORK/root/crates/pierre-tool-runtime/src/reader.rs" <<'RS'
pub async fn read(repos: &Repos) -> Vec<Merged> {
    stored_health::merged_sleep(repos).await
}
RS
}

run() { # prints the exit status; output in $WORK/out
    local code=0
    bash "$CHECK" "$WORK/root" >"$WORK/out" 2>&1 || code=$?
    echo "$code"
}

echo "check-one-wellness-merge.test.sh"

scaffold
code="$(run)"
if [ "$code" -eq 0 ] && grep -q "only in crates/pierre-services/src/stored_health.rs" "$WORK/out"; then
    pass "readers that go through the chokepoint pass"
else
    fail "a clean tree should pass (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
cat >> "$WORK/root/crates/pierre-tool-runtime/src/reader.rs" <<'RS'
pub fn merge_myself(rows: Vec<Row>) -> Vec<Merged> {
    dravr_equilibre::merge_recovery_metrics(rows)
}
RS
code="$(run)"
if [ "$code" -eq 1 ] && grep -q "reader.rs:5:" "$WORK/out"; then
    pass "a merge outside the chokepoint fails and is named"
else
    fail "a merge outside the chokepoint should fail (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
cat >> "$WORK/root/crates/pierre-tool-runtime/src/reader.rs" <<'RS'
pub async fn raw(repos: &Repos) -> Vec<Row> {
    repos.health_snapshots.get_health_snapshots(1, 2, 3, 4).await
}
RS
code="$(run)"
if [ "$code" -eq 1 ] && grep -q "reader.rs:5:" "$WORK/out"; then
    pass "a repository read outside the chokepoint fails"
else
    fail "a raw repository read should fail (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

# Each spelling that brings back a second read path: name, snippet, line.
expect_fires() {
    local what="$1" snippet="$2"
    scaffold
    printf '%s\n' "$snippet" >> "$WORK/root/crates/pierre-tool-runtime/src/reader.rs"
    local code
    code="$(run)"
    if [ "$code" -eq 1 ] && grep -q "reader.rs:4:" "$WORK/out"; then
        pass "$what outside the chokepoint fails"
    else
        fail "$what outside the chokepoint should fail (exit $code)"; sed 's/^/       /' "$WORK/out"
    fi
}
expect_fires "a UFCS repository read" \
    'fn raw(r: &R) { SleepRepository::get_sleep_sessions(&*r.sleep, 1, 2, 3, 4); }'
expect_fires "an origin read" \
    'fn raw(r: &R) { r.recovery.recovery_origins(1, 2, 3, 4); }'
expect_fires "raw SQL on a wellness table" \
    'const RAW: &str = "SELECT * FROM recovery_metrics WHERE user_id = $1";'
expect_fires "night dating by the UTC date of the end" \
    'fn night(s: &S) -> NaiveDate { s.end_datetime.date_naive() }'
expect_fires "a daily-activity merge" \
    'fn raw(rows: Vec<R>) { dravr_equilibre::merge_daily_activity(rows); }'
expect_fires "an HRV spelling of its own" \
    'fn hrv(m: &M) -> Option<f64> { m.hrv_rmssd.or(m.hrv_ms) }'

scaffold
mkdir -p "$WORK/root/crates/pierre-database/src"
printf 'const GET: &str = "SELECT id FROM sleep_sessions WHERE user_id = $1";\n' \
    > "$WORK/root/crates/pierre-database/src/health.rs"
code="$(run)"
if [ "$code" -eq 0 ]; then
    pass "the repository layer's own SQL passes"
else
    fail "repository SQL should pass (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
cat >> "$WORK/root/crates/pierre-tool-runtime/src/reader.rs" <<'RS'

#[cfg(test)]
mod tests {
    use dravr_equilibre::merge_sleep_sessions;
    #[test]
    fn merges() { assert!(merge_sleep_sessions(Vec::new()).is_empty()); }
}
RS
code="$(run)"
if [ "$code" -eq 0 ]; then
    pass "a merge inside a trailing test module is not production code"
else
    fail "a test-module merge should pass (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
rm "$WORK/root/crates/pierre-services/src/stored_health.rs"
code="$(run)"
if [ "$code" -eq 2 ]; then
    pass "a missing chokepoint fails closed"
else
    fail "a missing chokepoint should exit 2 (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
printf 'pub fn nothing() {}\n' > "$WORK/root/crates/pierre-services/src/stored_health.rs"
code="$(run)"
if [ "$code" -eq 2 ]; then
    pass "a chokepoint that no longer merges fails closed"
else
    fail "an empty chokepoint should exit 2 (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

if [ "$FAILURES" -gt 0 ]; then
    echo "check-one-wellness-merge.test.sh: $FAILURES failure(s)"
    exit 1
fi
echo "check-one-wellness-merge.test.sh: all passed"
