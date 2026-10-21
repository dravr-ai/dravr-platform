#!/usr/bin/env bash
# ABOUTME: Fixture test for check-unused-deps.sh — a throwaway two-crate workspace with a planted unused,
# ABOUTME: misplaced or ignored dependency, plus stub cargo-shears that are missing, stale, crash or report nothing
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The gate must fail on a finding and pass a clean tree, but it must also fail
# whenever it could not have looked: no tool, another tool version, a crash,
# an empty report, a report that disagrees with the exit status, a workspace
# with no member. Each of those passes silently in a gate that greps for a
# finding. The fixture workspace has only path dependencies, so the real
# cargo-shear runs offline in about a second.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/check-unused-deps.sh}"
WORK="$(mktemp -d)"
OUT="$WORK/out"
trap 'rm -rf "$WORK"' EXIT

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

PINNED="$("$UNDER_TEST" --pinned-version)"

if ! command -v cargo-shear >/dev/null 2>&1; then
  echo "❌ check-unused-deps.test.sh needs cargo-shear: cargo install --locked cargo-shear@${PINNED}"
  exit 1
fi

# A clean two-crate workspace: `app` calls into `lib_b`, its only dependency.
fixture() {
  rm -rf "$WORK/ws"
  mkdir -p "$WORK/ws/crates/app/src" "$WORK/ws/crates/lib_b/src" "$WORK/ws/crates/lib_c/src"
  cat >"$WORK/ws/Cargo.toml" <<'EOF'
[workspace]
resolver = "2"
members = ["crates/*"]
EOF
  cat >"$WORK/ws/crates/lib_b/Cargo.toml" <<'EOF'
[package]
name = "lib_b"
version = "0.1.0"
edition = "2021"
EOF
  echo 'pub fn answer() -> u32 { 42 }' >"$WORK/ws/crates/lib_b/src/lib.rs"
  cat >"$WORK/ws/crates/lib_c/Cargo.toml" <<'EOF'
[package]
name = "lib_c"
version = "0.1.0"
edition = "2021"
EOF
  echo 'pub fn other() -> u32 { 7 }' >"$WORK/ws/crates/lib_c/src/lib.rs"
  cat >"$WORK/ws/crates/app/Cargo.toml" <<'EOF'
[package]
name = "app"
version = "0.1.0"
edition = "2021"

[dependencies]
lib_b = { path = "../lib_b" }
EOF
  echo 'pub fn run() -> u32 { lib_b::answer() }' >"$WORK/ws/crates/app/src/lib.rs"
}

# Regenerates the lockfile the gate's --locked run expects.
lock() {
  (cd "$WORK/ws" && cargo generate-lockfile --offline -q)
}

add_dep() { # $1 = a dependency line; [dependencies] is the last table of app's manifest
  printf '%s\n' "$1" >>"$WORK/ws/crates/app/Cargo.toml"
}

run_check() { # $1 = workspace root; extra env via the caller; prints exit status, output in $OUT
  local code=0
  "$UNDER_TEST" "$1" >"$OUT" 2>&1 || code=$?
  echo "$code"
}

expect() { # $1 = label, $2 = actual exit, $3 = expected exit
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1 (exit $2, expected $3)"
    sed 's/^/      /' "$OUT"
  fi
}

expect_out() { # $1 = label, $2 = fixed string the output must contain
  if grep -qF -- "$2" "$OUT"; then pass "$1"; else
    fail "$1"
    sed 's/^/      /' "$OUT"
  fi
}

# A stub cargo-shear: $1 = version it reports, $2 = exit status of a scan,
# $3 = what the scan prints on stdout.
stub() {
  cat >"$WORK/stub-shear" <<EOF
#!/usr/bin/env bash
if [ "\${2:-}" = "--version" ]; then echo "Version: $1"; exit 0; fi
printf '%s' '$3'
echo "stub stderr" >&2
exit $2
EOF
  chmod +x "$WORK/stub-shear"
}

echo "check-unused-deps.sh fixture tests"

fixture; lock
expect "a clean workspace passes" "$(run_check "$WORK/ws")" 0
expect_out "the pass names the members it scanned" "scanned 3 workspace crates"

fixture; add_dep 'lib_c = { path = "../lib_c" }'; lock
expect "a planted unused dependency fails" "$(run_check "$WORK/ws")" 1
expect_out "the unused dependency is named" 'unused dependency `lib_c`'
expect_out "its manifest is named" "crates/app/Cargo.toml"

fixture; add_dep 'lib_c = { path = "../lib_c" }'
printf '\n# lib_c is a feature-unification edge in this fixture.\n[package.metadata.cargo-shear]\nignored = ["lib_c"]\n' \
  >>"$WORK/ws/crates/app/Cargo.toml"; lock
expect "a per-crate ignore with its reason passes" "$(run_check "$WORK/ws")" 0

fixture
printf '\n[package.metadata.cargo-shear]\nignored = ["lib_b"]\n' >>"$WORK/ws/crates/app/Cargo.toml"; lock
expect "an ignore that suppresses nothing fails" "$(run_check "$WORK/ws")" 1
expect_out "the stale ignore is named" "redundant"

fixture; add_dep 'lib_c = { path = "../lib_c" }'
mkdir -p "$WORK/ws/crates/app/tests"
echo '#[test] fn t() { assert_eq!(lib_c::other(), 7); }' >"$WORK/ws/crates/app/tests/it.rs"; lock
expect "a normal dependency only a test uses fails" "$(run_check "$WORK/ws")" 1
expect_out "it is reported as misplaced" 'misplaced dependency `lib_c`'

fixture; lock
echo 'pub fn orphan() {}' >"$WORK/ws/crates/app/src/orphan.rs"
expect "an unlinked source file fails" "$(run_check "$WORK/ws")" 1
expect_out "the unlinked file is named" "orphan.rs"

fixture; lock
expect "a missing cargo-shear fails" "$(CARGO_SHEAR="$WORK/no-such-shear" run_check "$WORK/ws")" 1
expect_out "it says how to install the pinned version" "cargo-shear@${PINNED}"

stub "0.0.1" 0 '{"summary":{"errors":0,"warnings":0,"fixed":0},"findings":[]}'
expect "another cargo-shear version fails" "$(CARGO_SHEAR="$WORK/stub-shear" run_check "$WORK/ws")" 1
expect_out "it names the required version" "cargo-shear ${PINNED} is required"

stub "$PINNED" 2 ''
expect "a crashed cargo-shear fails" "$(CARGO_SHEAR="$WORK/stub-shear" run_check "$WORK/ws")" 1
expect_out "the crash is reported as verifying nothing" "crashed (exit 2)"

stub "$PINNED" 0 ''
expect "an empty report with exit 0 fails" "$(CARGO_SHEAR="$WORK/stub-shear" run_check "$WORK/ws")" 1
expect_out "it says no report was read" "no readable report"

stub "$PINNED" 0 '{"summary":{"errors":1,"warnings":0,"fixed":0},"findings":[{"message":"unused dependency `x`"}]}'
expect "a report of findings under exit 0 fails" "$(CARGO_SHEAR="$WORK/stub-shear" run_check "$WORK/ws")" 1
expect_out "the disagreement is named" "disagrees with the exit status"

stub "$PINNED" 1 '{"summary":{"errors":0,"warnings":0,"fixed":0},"findings":[]}'
expect "exit 1 with an empty report fails" "$(CARGO_SHEAR="$WORK/stub-shear" run_check "$WORK/ws")" 1

stub "$PINNED" 0 '{"summary":{"errors":0,"warnings":0,"fixed":0},"findings":[]}'
expect "a stub reporting clean passes, so the cases above failed on their defect" \
  "$(CARGO_SHEAR="$WORK/stub-shear" run_check "$WORK/ws")" 0

fixture; rm -rf "$WORK/ws/crates"
expect "a workspace with no member fails" "$(run_check "$WORK/ws")" 1

rm -rf "$WORK/ws"; mkdir -p "$WORK/ws"
expect "a directory with no manifest fails" "$(run_check "$WORK/ws")" 1
expect_out "it says the check verified nothing" "verified nothing"

# The fast gate installs the version the script pins.
WORKFLOW="$REPO_ROOT/.github/workflows/ci-backend.yml"
if grep -qF "tool: cargo-shear@${PINNED}" "$WORKFLOW"; then
  pass "ci-backend.yml installs cargo-shear@${PINNED}, the version the gate pins"
else
  fail "ci-backend.yml does not install cargo-shear@${PINNED} (found: $(grep -o 'cargo-shear@[0-9.]*' "$WORKFLOW" | head -1))"
fi

if [ "$failures" -gt 0 ]; then
  echo "❌ $failures check-unused-deps.sh case(s) failed"
  exit 1
fi
echo "✅ check-unused-deps.sh fixture tests passed"
