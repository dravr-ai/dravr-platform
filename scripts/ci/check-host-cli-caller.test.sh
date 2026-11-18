#!/usr/bin/env bash
# ABOUTME: Fixture test for check-host-cli-caller.sh — makes it fire, pass and fail closed
# ABOUTME: host_cli named in a server crate fails; in pierre-cli, a test module or tests/ it passes
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$SCRIPT_DIR/check-host-cli-caller.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILURES=0

pass() { echo "  ok   — $1"; }
fail() { echo "  FAIL — $1"; FAILURES=$((FAILURES + 1)); }

# The definition, the CLI building it, and a server route that does not.
scaffold() {
    rm -rf "$WORK/root"
    mkdir -p "$WORK/root/crates/pierre-services/src/admin_ops" \
        "$WORK/root/crates/pierre-cli/src/commands" \
        "$WORK/root/crates/pierre-routes-admin/src"
    cat > "$WORK/root/crates/pierre-services/src/admin_ops/caller.rs" <<'RS'
impl AccountCaller {
    pub fn host_cli(recorded_as: Option<Uuid>) -> Self {
        Self { platform: true, recorded_as }
    }
}
RS
    cat > "$WORK/root/crates/pierre-cli/src/commands/user.rs" <<'RS'
pub async fn approve(repos: &Repos) {
    let host = AccountCaller::host_cli(None);
    run(repos, &host).await;
}
RS
    cat > "$WORK/root/crates/pierre-routes-admin/src/users.rs" <<'RS'
pub async fn handle(token: &ValidatedAdminToken) {
    let caller = AccountCaller::of_token(token);
    run(&caller).await;
}
RS
}

run() { # prints the exit status; output in $WORK/out
    local code=0
    bash "$CHECK" "$WORK/root" >"$WORK/out" 2>&1 || code=$?
    echo "$code"
}

echo "check-host-cli-caller.test.sh"

scaffold
code="$(run)"
if [ "$code" -eq 0 ] && grep -q "built only under crates/pierre-cli/src/ (1 site(s))" "$WORK/out"; then
    pass "the CLI building it passes"
else
    fail "a clean tree should pass (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

# Each spelling that builds or brings in the host caller outside the CLI.
expect_fires() {
    local what="$1" snippet="$2"
    scaffold
    printf '%s\n' "$snippet" >> "$WORK/root/crates/pierre-routes-admin/src/users.rs"
    local code
    code="$(run)"
    if [ "$code" -eq 1 ] && grep -q "pierre-routes-admin/src/users.rs:5:" "$WORK/out"; then
        pass "$what in a server crate fails and is named"
    else
        fail "$what in a server crate should fail (exit $code)"; sed 's/^/       /' "$WORK/out"
    fi
}
expect_fires "AccountCaller::host_cli" \
    'pub fn every_power() -> AccountCaller { AccountCaller::host_cli(None) }'
expect_fires "a fully qualified path" \
    'pub fn every_power() -> AccountCaller { pierre_services::admin_ops::AccountCaller::host_cli(None) }'
expect_fires "a function pointer" \
    'pub fn every_power() -> fn(Option<Uuid>) -> AccountCaller { AccountCaller::host_cli }'
expect_fires "Self::host_cli in another impl" \
    'impl AccountCaller { pub fn sneaky() -> Self { Self::host_cli(None) } }'

scaffold
cat >> "$WORK/root/crates/pierre-routes-admin/src/users.rs" <<'RS'

#[cfg(test)]
mod tests {
    #[test]
    fn host() { let _ = AccountCaller::host_cli(None); }
}
RS
code="$(run)"
if [ "$code" -eq 0 ]; then
    pass "a trailing test module is not production code"
else
    fail "a test-module use should pass (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
mkdir -p "$WORK/root/crates/pierre-routes-admin/tests"
printf 'fn host() { let _ = AccountCaller::host_cli(None); }\n' \
    > "$WORK/root/crates/pierre-routes-admin/tests/host.rs"
code="$(run)"
if [ "$code" -eq 0 ]; then
    pass "an integration test is not scanned"
else
    fail "a tests/ use should pass (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
rm "$WORK/root/crates/pierre-services/src/admin_ops/caller.rs"
code="$(run)"
if [ "$code" -eq 2 ]; then
    pass "a missing definition fails closed"
else
    fail "a missing definition should exit 2 (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
code=0
PROD_SRC_SCAN="$WORK/no-such-scan.sh" bash "$CHECK" "$WORK/root" >"$WORK/out" 2>&1 || code=$?
if [ "$code" -ne 0 ] && [ "$code" -ne 1 ]; then
    pass "a scan that crashes fails closed"
else
    fail "a crashed scan should not pass or read as a finding (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

scaffold
printf 'pub async fn approve() {}\n' > "$WORK/root/crates/pierre-cli/src/commands/user.rs"
code="$(run)"
if [ "$code" -eq 2 ] && grep -q "verified nothing" "$WORK/out"; then
    pass "a scan that finds no site in pierre-cli fails closed"
else
    fail "no site in pierre-cli should exit 2 (exit $code)"; sed 's/^/       /' "$WORK/out"
fi

if [ "$FAILURES" -gt 0 ]; then
    echo "check-host-cli-caller.test.sh: $FAILURES failure(s)"
    exit 1
fi
echo "check-host-cli-caller.test.sh: all passed"
