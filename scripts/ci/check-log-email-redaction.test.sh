#!/usr/bin/env bash
# ABOUTME: Fixture test for check-log-email-redaction.py — builds throwaway crates and asserts each verdict
# ABOUTME: Pins that a wrapped, inline or positional raw email fails and a masked, DEBUG or boolean one passes
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The gate exists because rustfmt wraps a log call over several lines, so the
# `user.email` argument sits on a line the old `^\s*info!` greps never read.
# Every leak it was written for had that shape; the first cases pin it.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/check-log-email-redaction.py}"
OUT="$(mktemp)"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

# A tree whose only source file is `crates/demo/src/lib.rs` with the given body.
tree_with() {
  local dir
  dir="$(mktemp -d)"
  mkdir -p "$dir/crates/demo/src"
  printf '%s\n' "$1" >"$dir/crates/demo/src/lib.rs"
  printf '%s\n' "$dir"
}

run_gate() {
  local code=0
  python3 "$UNDER_TEST" "$1" >"$OUT" 2>&1 || code=$?
  echo "$code"
}

expect() { # $1 = label, $2 = actual exit, $3 = expected exit
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1 (exit $2, expected $3)"
    sed 's/^/      /' "$OUT"
  fi
}

expect_named() { # $1 = label, $2 = text the report must contain
  if grep -qF "$2" "$OUT"; then pass "$1"; else
    fail "$1 (report lacks: $2)"
    sed 's/^/      /' "$OUT"
  fi
}

# A case that must fail, naming the line and the identifier it caught.
must_fail() { # $1 = label, $2 = source, $3 = expected "lib.rs:N: macro! logs 'x'"
  local dir
  dir="$(tree_with "$2")"
  expect "$1" "$(run_gate "$dir")" 1
  expect_named "$1 names the site" "$3"
  rm -rf "$dir"
}

must_pass() { # $1 = label, $2 = source
  local dir
  dir="$(tree_with "$2")"
  expect "$1" "$(run_gate "$dir")" 0
  rm -rf "$dir"
}

echo "check-log-email-redaction.py fixture tests"

must_fail "a raw field value in a wrapped info! fails" \
'fn f(user: &User) {
    info!(
        user_id = %user.id,
        email = %user.email,
        "User approved"
    );
}' "lib.rs:2: info! logs 'email'"

must_fail "a positional argument on its own line fails" \
'fn f(user_email: &str, e: &str) {
    error!(
        "Failed to create default tenant for user {}: {}",
        user_email, e
    );
}' "lib.rs:2: error! logs 'user_email'"

must_fail "an inline format capture fails" \
'fn f(coach_email: &str) { tracing::warn!("{coach_email} already coaches the group"); }' \
"lib.rs:1: warn! logs 'coach_email'"

must_fail "a raw address beside a masked one still fails" \
'fn f(a: &str, b: &User) { info!("{} and {}", mask_email(a), b.email); }' \
"lib.rs:1: info! logs 'email'"

must_fail "a shorthand field fails" \
'fn f(email: &str) { info!(%email, "Admin setup request"); }' \
"lib.rs:1: info! logs 'email'"

must_pass "a masked value passes" \
'fn f(user: &User) {
    info!(
        target_email = %mask_email(&updated.email),
        "User promoted to admin"
    );
}'

must_pass "an already-masked variable passes" \
'fn f(masked_email: &str) { info!(email_masked = %masked_email, "OTP sent: {masked_email}"); }'

must_pass "the address at DEBUG passes" \
'fn f(to: &str, email: &str) {
    debug!(to, email, "Sending email via Resend");
    trace!("{email}");
    info!(subject = "reset", "Email sent successfully via Resend");
}'

must_pass "a boolean about an address passes" \
'fn f(email: Option<&str>, claims: &Claims) {
    info!(slack = true, email = email.is_some(), "Error notification layer enabled");
    warn!(verified = claims.email_verified.unwrap_or(false), "minted for another identity");
}'

must_pass "prose that mentions email passes" \
'fn f(e: &str) { warn!(error = %e, "Failed to send the {} email", "invitation"); info!("Email service not configured"); }'

must_pass "a commented-out leak passes" \
'fn f(user: &User) {
    // info!("{}", user.email);
    /* warn!(email = %user.email, "x"); */
    info!(user_id = %user.id, "ok");
}'

must_pass "a char literal paren does not unbalance the scan" \
'fn f<'"'"'a>(s: &'"'"'a str) { info!(open = '"'"'('"'"', close = '"'"')'"'"', len = s.len(), "chars"); }'

dir="$(tree_with 'fn f() { debug!("nothing at INFO or above"); }')"
expect "a tree with no INFO+ macro fails closed" "$(run_gate "$dir")" 2
expect_named "the fail-closed verdict says nothing was verified" "nothing was verified"
rm -rf "$dir"

dir="$(tree_with 'info!("a macro cut off at end of file {}", user.id')"
expect "an unclosed macro fails closed" "$(run_gate "$dir")" 2
rm -rf "$dir"

rm -f "$OUT"
if [ "$failures" -gt 0 ]; then
  echo "$failures fixture case(s) failed"
  exit 1
fi
echo "all fixture cases passed"
