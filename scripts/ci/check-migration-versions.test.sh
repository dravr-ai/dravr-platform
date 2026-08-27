#!/usr/bin/env bash
# ABOUTME: Fixture test for check-migration-versions.sh — builds throwaway repos and
# ABOUTME: asserts every branch, including that an empty scan fails rather than passes.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# An unverified gate is worse than no gate: it reports ✅ forever and everyone
# believes duplicates are impossible. The case that matters most here is the
# empty one — a collect-and-filter check that scans nothing passes vacuously,
# which is the exact way this class of guard rots.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="$SCRIPT_DIR/check-migration-versions.sh"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

make_repo() {
  local dir
  dir="$(mktemp -d)"
  git -C "$dir" init -q
  git -C "$dir" config user.email test@example.com
  git -C "$dir" config user.name test
  mkdir -p "$dir/migrations" "$dir/migrations_pg"
  printf '%s\n' "$dir"
}

# Output lands in a fixed file, not a variable: run_gate is called inside a
# command substitution, so any variable it sets dies with that subshell.
OUT="$(mktemp)"
trap 'rm -f "$OUT"' EXIT

run_gate() { # $1 = dir; echoes exit code, leaves output in $OUT
  local dir="$1" code=0
  ( cd "$dir" && "$UNDER_TEST" >"$OUT" 2>&1 ) || code=$?
  printf '%s\n' "$code"
}

expect() { # $1 = label, $2 = actual, $3 = wanted
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1 (exit $2, wanted $3)"; sed 's/^/       /' "$OUT"
  fi
}

echo "check-migration-versions.sh"

# --- 1. distinct versions in both backends: pass -------------------------------
d="$(make_repo)"
touch "$d/migrations/20260101000001_a.sql" "$d/migrations/20260101000002_b.sql"
touch "$d/migrations_pg/20260101000001_a.sql" "$d/migrations_pg/20260101000002_b.sql"
expect "distinct versions pass" "$(run_gate "$d")" 0
rm -rf "$d"

# --- 2. the real 2026-08-27 collision: fail ------------------------------------
# Two different migrations claiming 20260827000001, an hour apart on main.
d="$(make_repo)"
touch "$d/migrations/20260827000001_llm_usage_cache_write_and_reasoning.sql"
touch "$d/migrations/20260827000001_prescribed_workouts_calendar_ledger.sql"
code="$(run_gate "$d")"
expect "duplicate version fails" "$code" 1
grep -q "20260827000001" "$OUT" || fail "failure output must name the version"
grep -q "prescribed_workouts_calendar_ledger" "$OUT" || fail "failure output must name both files"
rm -rf "$d"

# --- 3. same version once per backend: pass ------------------------------------
# migrations/ and migrations_pg/ are per-backend mirrors of ONE migration, so
# the same number appearing once in each is correct, not a collision.
d="$(make_repo)"
touch "$d/migrations/20260101000001_a.sql" "$d/migrations_pg/20260101000001_a.sql"
expect "mirrored version across backends passes" "$(run_gate "$d")" 0
rm -rf "$d"

# --- 4. out-of-order versions: pass --------------------------------------------
# sqlx rejects duplicates, not out-of-order inserts (verified against a live dev
# DB 2026-08-27). A gate demanding monotonic versions would block valid work.
d="$(make_repo)"
touch "$d/migrations/20260827000001_later.sql" "$d/migrations/20260826000008_earlier.sql"
expect "out-of-order versions pass" "$(run_gate "$d")" 0
rm -rf "$d"

# --- 5. nothing to scan: fail, never a vacuous pass ----------------------------
d="$(make_repo)"
expect "empty tree fails rather than passing vacuously" "$(run_gate "$d")" 1
rm -rf "$d"

# --- 6. unparseable filename: reported, but not a duplicate --------------------
d="$(make_repo)"
touch "$d/migrations/no_version_prefix.sql" "$d/migrations/20260101000001_a.sql"
code="$(run_gate "$d")"
expect "unparseable name does not fail the gate" "$code" 0
grep -q "no_version_prefix.sql" "$OUT" || fail "unparseable name must be reported, not skipped silently"
rm -rf "$d"

echo
if [ "$failures" -gt 0 ]; then
  echo "❌ $failures case(s) failed"
  exit 1
fi
echo "✅ all cases passed"
