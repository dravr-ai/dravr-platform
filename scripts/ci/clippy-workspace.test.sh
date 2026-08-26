#!/usr/bin/env bash
# ABOUTME: Fixture test for clippy-workspace.py — builds a throwaway cargo workspace
# ABOUTME: and asserts the unevaluated report fires, so the reporter cannot stay silent
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The whole point of the reporter is to speak up on a red run. A reporter that
# never fires looks identical to a workspace with nothing hidden, which is the
# exact confusion it exists to remove — so the failing case is tested by making
# it happen: three crates, one broken, one downstream of the break, one beside
# it. The downstream crate must be named and the independent one must not.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="$SCRIPT_DIR/clippy-workspace.py"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

# leaf <- dependent, and island beside them with no relationship to either.
# $1 = "broken" to give leaf a denied warning, anything else for a clean tree.
make_workspace() {
  local dir state
  dir="$(mktemp -d)"
  state="$1"

  cat >"$dir/Cargo.toml" <<'EOF'
[workspace]
resolver = "2"
members = ["leaf", "dependent", "island"]
EOF

  mkdir -p "$dir/leaf/src" "$dir/dependent/src" "$dir/island/src"

  cat >"$dir/leaf/Cargo.toml" <<'EOF'
[package]
name = "leaf"
version = "0.1.0"
edition = "2021"
EOF

  if [ "$state" = "broken" ]; then
    # An unused import is a warning; `-D warnings` makes it an error, the lib
    # produces no rmeta, and `dependent` becomes uncompilable through no fault
    # of its own. That is the exact shape this reporter has to describe.
    cat >"$dir/leaf/src/lib.rs" <<'EOF'
use std::collections::HashMap;

pub fn value() -> u8 {
    7
}
EOF
  else
    cat >"$dir/leaf/src/lib.rs" <<'EOF'
pub fn value() -> u8 {
    7
}
EOF
  fi

  cat >"$dir/dependent/Cargo.toml" <<'EOF'
[package]
name = "dependent"
version = "0.1.0"
edition = "2021"

[dependencies]
leaf = { path = "../leaf" }
EOF
  cat >"$dir/dependent/src/lib.rs" <<'EOF'
pub fn doubled() -> u8 {
    leaf::value() * 2
}
EOF

  cat >"$dir/island/Cargo.toml" <<'EOF'
[package]
name = "island"
version = "0.1.0"
edition = "2021"
EOF
  cat >"$dir/island/src/lib.rs" <<'EOF'
pub fn alone() -> u8 {
    1
}
EOF

  printf '%s\n' "$dir"
}

echo "clippy-workspace.py"

# ---- Case 1: a broken leaf hides its dependent -----------------------------
dir="$(make_workspace broken)"
set +e
out="$(cd "$dir" && python3 "$UNDER_TEST" 2>&1)"
status=$?
set -e

if [ "$status" -ne 0 ]; then
  pass "broken leaf: exits non-zero"
else
  fail "broken leaf: expected non-zero exit, got $status"
fi

if grep -q "unused import" <<<"$out"; then
  pass "broken leaf: the reachable error is still rendered"
else
  fail "broken leaf: expected the unused-import diagnostic in the output"
fi

if grep -qE '^\s+- dependent\s+\(blocked by leaf\)' <<<"$out"; then
  pass "broken leaf: names 'dependent' as blocked by 'leaf'"
else
  fail "broken leaf: expected 'dependent' reported as blocked by 'leaf'"
  echo "$out"
fi

if grep -qE '^\s+- island' <<<"$out"; then
  fail "broken leaf: 'island' was linted and must not be listed as unevaluated"
else
  pass "broken leaf: 'island' linted, so it is absent from the report"
fi

if grep -q "cargo clippy -p dependent" <<<"$out"; then
  pass "broken leaf: prints the command that lints the hidden crate"
else
  fail "broken leaf: expected a per-crate clippy command for 'dependent'"
fi
rm -rf "$dir"

# ---- Case 2: nothing broken, nothing to report -----------------------------
dir="$(make_workspace clean)"
set +e
out="$(cd "$dir" && python3 "$UNDER_TEST" 2>&1)"
status=$?
set -e

if [ "$status" -eq 0 ]; then
  pass "clean tree: exits zero"
else
  fail "clean tree: expected zero exit, got $status"
  echo "$out"
fi

if grep -q "evaluated all 3 workspace crates" <<<"$out"; then
  pass "clean tree: confirms full coverage"
else
  fail "clean tree: expected the all-crates-evaluated confirmation"
  echo "$out"
fi

if grep -q "were NOT evaluated" <<<"$out"; then
  fail "clean tree: reported unevaluated crates on a green run"
else
  pass "clean tree: reports nothing unevaluated"
fi

# ---- Case 3: a scoped run must not claim it covered the workspace ----------
# A crate cargo was never asked to build has no artifacts and no failed
# dependency, which is indistinguishable from "blocked" unless the two are
# separated. Claiming full coverage here would be the same false-clean reading
# the reporter exists to prevent.
set +e
out="$(cd "$dir" && python3 "$UNDER_TEST" -p island 2>&1)"
status=$?
set -e

if [ "$status" -eq 0 ]; then
  pass "scoped run: exits zero"
else
  fail "scoped run: expected zero exit, got $status"
fi

if grep -q "evaluated all 3 workspace crates" <<<"$out"; then
  fail "scoped run: claimed full coverage after building only one crate"
else
  pass "scoped run: does not claim full coverage"
fi

if grep -q "outside this invocation's scope" <<<"$out"; then
  pass "scoped run: says how many crates were out of scope"
else
  fail "scoped run: expected an out-of-scope count"
  echo "$out"
fi
rm -rf "$dir"

echo ""
if [ "$failures" -eq 0 ]; then
  echo "✅ clippy-workspace.py: all assertions passed"
else
  echo "❌ clippy-workspace.py: $failures assertion(s) failed"
  exit 1
fi
