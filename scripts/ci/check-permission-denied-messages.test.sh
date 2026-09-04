#!/usr/bin/env bash
# ABOUTME: Fixture test for check-permission-denied-messages.sh — asserts each verdict,
# ABOUTME: including the blind-spot report, so an unreviewed refusal can never pass green.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# An unverified gate is worse than no gate, and this one's whole value is that it
# FAILS on a message nobody read. Each case builds a throwaway tree holding one
# crate and one inventory, runs the gate inside it, and asserts the exit code.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/check-permission-denied-messages.sh}"
OUT=/tmp/permission-denied-messages-test.out

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

# A tree the gate can run in: it resolves its root two levels above itself.
make_tree() {
  local dir
  dir="$(mktemp -d)"
  mkdir -p "$dir/scripts/ci" "$dir/crates/demo/src"
  cp "$UNDER_TEST" "$dir/scripts/ci/check-permission-denied-messages.sh"
  printf '%s\n' "$dir"
}

run_gate() {
  local dir="$1" code=0
  "$dir/scripts/ci/check-permission-denied-messages.sh" >"$OUT" 2>&1 || code=$?
  echo "$code"
}

expect() { # $1 = label, $2 = actual, $3 = expected
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1 (exit $2, expected $3)"
    sed 's/^/      /' "$OUT"
  fi
}

expect_says() { # $1 = label, $2 = substring
  if grep -qF -- "$2" "$OUT"; then pass "$1"; else
    fail "$1 (output does not mention '$2')"
    sed 's/^/      /' "$OUT"
  fi
}

echo "check-permission-denied-messages.sh fixture tests"

# 1. src and the inventory agree.
dir="$(make_tree)"
cat >"$dir/crates/demo/src/lib.rs" <<'RS'
pub fn refuse() -> AppError {
    AppError::new(ErrorCode::PermissionDenied, "Owner role required")
}
RS
printf '# reviewed\ncrates/demo/src/lib.rs:Owner role required\n' >"$dir/scripts/ci/permission-denied-messages.txt"
expect "a reviewed message passes" "$(run_gate "$dir")" 0
expect_says "success names the site count" "all 1 construction sites are reviewed"

# 2. A new refusal nobody has reviewed.
cat >>"$dir/crates/demo/src/lib.rs" <<'RS'
pub fn refuse_again() -> AppError {
    AppError::new(ErrorCode::PermissionDenied, "Cannot remove the group owner")
}
RS
expect "an unreviewed message fails" "$(run_gate "$dir")" 1
expect_says "the unreviewed site is named" "crates/demo/src/lib.rs:Cannot remove the group owner"

# 3. An inventory line whose construction site is gone.
dir="$(make_tree)"
cat >"$dir/crates/demo/src/lib.rs" <<'RS'
pub fn refuse() -> AppError {
    AppError::new(ErrorCode::PermissionDenied, "Owner role required")
}
RS
printf 'crates/demo/src/lib.rs:Owner role required\ncrates/demo/src/lib.rs:Reworded away\n' \
  >"$dir/scripts/ci/permission-denied-messages.txt"
expect "a stale inventory line fails" "$(run_gate "$dir")" 1
expect_says "the stale line is named" "crates/demo/src/lib.rs:Reworded away"

# 4. A `format!` template IS reviewable: the placeholders are the shape, exactly
#    like the "Permission required: {permission}" entry in the real inventory.
dir="$(make_tree)"
cat >"$dir/crates/demo/src/lib.rs" <<'RS'
pub fn refuse(role: &str) -> AppError {
    AppError::new(ErrorCode::PermissionDenied, format!("{role} required"))
}
RS
printf 'crates/demo/src/lib.rs:{role} required\n' >"$dir/scripts/ci/permission-denied-messages.txt"
expect "a format! template is reviewed by its placeholders" "$(run_gate "$dir")" 0

# 4b. A note between the code and the message does not hide it. This repo asks
#     for that note on a non-obvious refusal, and English prose carries commas —
#     which a comma-terminated argument scan reads as the end of the argument,
#     turning a perfectly reviewable sentence into a false blind spot.
dir="$(make_tree)"
cat >"$dir/crates/demo/src/lib.rs" <<'RS'
pub fn refuse(role: &str) -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        // Names the role, and nothing else: this sentence reaches the caller
        // verbatim, so it must not mention how the check is stored.
        format!("{role} required"),
    )
}
RS
printf 'crates/demo/src/lib.rs:{role} required\n' >"$dir/scripts/ci/permission-denied-messages.txt"
expect "a comment before the message does not hide it" "$(run_gate "$dir")" 0
expect_says "the commented site is still counted" "all 1 construction sites are reviewed"

# 4c. Same for a block comment, which can also swallow the closing paren.
dir="$(make_tree)"
cat >"$dir/crates/demo/src/lib.rs" <<'RS'
pub fn refuse() -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        /* reviewed: names the plan, not the tier column */
        "Group coaching requires a Professional or Enterprise plan",
    )
}
RS
printf 'crates/demo/src/lib.rs:Group coaching requires a Professional or Enterprise plan\n' \
  >"$dir/scripts/ci/permission-denied-messages.txt"
expect "a block comment before the message does not hide it" "$(run_gate "$dir")" 0

# 5. A message the scan genuinely cannot read: the caller hands it in.
dir="$(make_tree)"
cat >"$dir/crates/demo/src/lib.rs" <<'RS'
pub fn refuse(reason: &str) -> AppError {
    AppError::new(ErrorCode::PermissionDenied, reason)
}
RS
printf 'crates/demo/src/lib.rs:anything\n' >"$dir/scripts/ci/permission-denied-messages.txt"
expect "a caller-supplied message fails loudly" "$(run_gate "$dir")" 1
expect_says "the blind spot is reported as such" "scan incomplete"

# 6. Same for a message built by a method call rather than a literal.
dir="$(make_tree)"
cat >"$dir/crates/demo/src/lib.rs" <<'RS'
pub fn refuse(reason: String) -> AppError {
    AppError::new(ErrorCode::PermissionDenied, reason.to_uppercase())
}
RS
printf 'crates/demo/src/lib.rs:anything\n' >"$dir/scripts/ci/permission-denied-messages.txt"
expect "a computed message fails loudly" "$(run_gate "$dir")" 1
expect_says "the computed blind spot names the line" "does not resolve to a literal"

# 7. Comparisons, match arms and doc comments are read sites, not constructions.
#    The file also holds one real construction, so a pass here means the read
#    sites contributed nothing rather than that the scan found nothing at all.
dir="$(make_tree)"
cat >"$dir/crates/demo/src/lib.rs" <<'RS'
/// Returns [`ErrorCode::PermissionDenied`] when the caller is refused.
pub fn describe(e: &AppError) -> &'static str {
    if e.code == ErrorCode::PermissionDenied {
        return "refused";
    }
    match e.code {
        ErrorCode::PermissionDenied => "refused",
        _ => "other",
    }
}

pub fn refuse() -> AppError {
    AppError::new(ErrorCode::PermissionDenied, "Owner role required")
}
RS
printf 'crates/demo/src/lib.rs:Owner role required\n' >"$dir/scripts/ci/permission-denied-messages.txt"
expect "read sites need no inventory entry" "$(run_gate "$dir")" 0
expect_says "only the construction is counted" "all 1 construction sites are reviewed"

# 8. A use of the code the classifier does not recognise must not be skipped.
dir="$(make_tree)"
cat >"$dir/crates/demo/src/lib.rs" <<'RS'
pub fn code() -> ErrorCode {
    let held = ErrorCode::PermissionDenied;
    held
}
RS
printf 'crates/demo/src/lib.rs:unused\n' >"$dir/scripts/ci/permission-denied-messages.txt"
expect "an unrecognised use fails" "$(run_gate "$dir")" 1
expect_says "the unrecognised use is named" "unrecognised use of ErrorCode::PermissionDenied"

# 9. user_state_error mints PermissionDenied, and its prefix is part of the message.
dir="$(make_tree)"
cat >"$dir/crates/demo/src/lib.rs" <<'RS'
use pierre_core::error_helpers::user_state_error;

pub fn refuse() -> AppError {
    user_state_error("User already exists")
}
RS
printf 'crates/demo/src/lib.rs:User state error: User already exists\n' \
  >"$dir/scripts/ci/permission-denied-messages.txt"
expect "a user_state_error site is reviewed under its rendered prefix" "$(run_gate "$dir")" 0

# 10. A message held in a &str constant is pinned by VALUE, not by name.
dir="$(make_tree)"
cat >"$dir/crates/demo/src/lib.rs" <<'RS'
pub const OWNER_ONLY: &str = "Owner role required";

pub fn refuse() -> AppError {
    AppError::new(ErrorCode::PermissionDenied, OWNER_ONLY)
}
RS
printf 'crates/demo/src/lib.rs:Owner role required\n' >"$dir/scripts/ci/permission-denied-messages.txt"
expect "a constant resolves to its value" "$(run_gate "$dir")" 0
sed -i.bak 's/Owner role required/Owner privileges required/' "$dir/crates/demo/src/lib.rs"
rm -f "$dir/crates/demo/src/lib.rs.bak"
expect "changing the constant's value fails" "$(run_gate "$dir")" 1

# 11. An inventory out of byte order.
dir="$(make_tree)"
cat >"$dir/crates/demo/src/lib.rs" <<'RS'
pub fn refuse() -> AppError {
    AppError::new(ErrorCode::PermissionDenied, "B refusal")
}
pub fn refuse_other() -> AppError {
    AppError::new(ErrorCode::PermissionDenied, "A refusal")
}
RS
printf 'crates/demo/src/lib.rs:B refusal\ncrates/demo/src/lib.rs:A refusal\n' \
  >"$dir/scripts/ci/permission-denied-messages.txt"
expect "an unsorted inventory fails" "$(run_gate "$dir")" 1
expect_says "the ordering failure says so" "not in byte order"

# 12. No inventory at all, and an empty one, are both failures — never a pass.
dir="$(make_tree)"
cat >"$dir/crates/demo/src/lib.rs" <<'RS'
pub fn refuse() -> AppError {
    AppError::new(ErrorCode::PermissionDenied, "Owner role required")
}
RS
expect "a missing inventory fails" "$(run_gate "$dir")" 1
printf '# nothing reviewed yet\n' >"$dir/scripts/ci/permission-denied-messages.txt"
expect "an empty inventory fails" "$(run_gate "$dir")" 1

echo ""
if [ "$failures" -eq 0 ]; then
  echo "✅ all fixture cases passed"
  exit 0
fi
echo "❌ $failures fixture case(s) failed"
exit 1
