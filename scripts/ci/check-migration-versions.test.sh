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
  # The freeze check needs a base ref to answer "was this already landed?", and
  # fails closed without one. Give every fixture an empty base commit.
  git -C "$dir" commit -q --allow-empty -m base
  git -C "$dir" branch -f base HEAD
  printf '%s\n' "$dir"
}

# Output lands in a fixed file, not a variable: run_gate is called inside a
# command substitution, so any variable it sets dies with that subshell.
OUT="$(mktemp)"
trap 'rm -f "$OUT"' EXIT

run_gate() { # $1 = dir; echoes exit code, leaves output in $OUT
  local dir="$1" code=0
  ( cd "$dir" && GATE_BASE_REF=base "$UNDER_TEST" >"$OUT" 2>&1 ) || code=$?
  printf '%s\n' "$code"
}

land() { # $1 = dir — commit everything, point `base` at it, then move past it
  git -C "$1" add -A
  git -C "$1" commit -q -m landed
  git -C "$1" branch -f base HEAD
  # A follow-up commit so the landing is genuinely in the PAST. Without it HEAD
  # is the landing itself, and the HEAD~1 fallback would see these files as
  # ADDED by the tip rather than already landed — which is the correct reading
  # of that shape, and the wrong fixture for "edited after it shipped".
  git -C "$1" commit -q --allow-empty -m follow-up
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

# --- 7. editing a landed migration: fail ---------------------------------------
# The 2026-09-08 outage. sqlx hashes the file, so a landed migration is frozen;
# every deploy aborted for ~13h while Cloud Run kept serving the last revision.
d="$(make_repo)"
printf 'SELECT 1;\n' > "$d/migrations/20260101000001_a.sql"
printf 'SELECT 1;\n' > "$d/migrations_pg/20260101000001_a.sql"
land "$d"
printf 'SELECT 2;\n' >> "$d/migrations/20260101000001_a.sql"
code="$(run_gate "$d")"
expect "editing a landed migration fails" "$code" 1
grep -q "20260101000001_a.sql" "$OUT" || fail "freeze failure must name the file"
grep -qi "supersede" "$OUT" || fail "freeze failure must say what to do instead"
rm -rf "$d"

# --- 8. a comment-only edit is still an edit -----------------------------------
# sqlx hashes bytes, not statements, so prose changes break the checksum too --
# 3e280bee1 rewrote comments in two migrations for exactly that reason.
d="$(make_repo)"
printf 'SELECT 1;\n' > "$d/migrations/20260101000001_a.sql"
printf 'SELECT 1;\n' > "$d/migrations_pg/20260101000001_a.sql"
land "$d"
printf -- '-- just a comment\n' >> "$d/migrations_pg/20260101000001_a.sql"
expect "comment-only edit of a landed migration fails" "$(run_gate "$d")" 1
rm -rf "$d"

# --- 9. adding a NEW migration alongside landed ones: pass ---------------------
# Superseding is the sanctioned repair, so it must not trip the freeze check.
d="$(make_repo)"
printf 'SELECT 1;\n' > "$d/migrations/20260101000001_a.sql"
printf 'SELECT 1;\n' > "$d/migrations_pg/20260101000001_a.sql"
land "$d"
printf 'SELECT 2;\n' > "$d/migrations/20260102000001_b.sql"
printf 'SELECT 2;\n' > "$d/migrations_pg/20260102000001_b.sql"
expect "superseding with a new migration passes" "$(run_gate "$d")" 0
rm -rf "$d"

# --- 10. a new migration reusing a catalogue id: fail --------------------------
# ON CONFLICT DO NOTHING makes the reuse silent, and a following DELETE then
# loses the row outright -- five agent tools on the deployed database and five
# physiology tools on every fresh one, 2026-09-08.
d="$(make_repo)"
for b in migrations migrations_pg; do
  printf "INSERT INTO tool_catalog (id, tool_name) VALUES ('tc-109', 'first');\n" > "$d/$b/20260101000001_a.sql"
done
land "$d"
for b in migrations migrations_pg; do
  printf "INSERT INTO tool_catalog (id, tool_name) VALUES ('tc-109', 'second');\n" > "$d/$b/20260102000001_b.sql"
done
code="$(run_gate "$d")"
expect "a new migration reusing a catalogue id fails" "$code" 1
grep -q "tc-109" "$OUT" || fail "collision failure must name the id"
rm -rf "$d"

# --- 11. a collision between two LANDED migrations: reported, not failed -------
# Neither can be renumbered once applied, so failing would leave a gate that can
# never go green. It is reported and repaired forward instead (20260909000001).
d="$(make_repo)"
for b in migrations migrations_pg; do
  printf "INSERT INTO tool_catalog (id, tool_name) VALUES ('tc-109', 'first');\n" > "$d/$b/20260101000001_a.sql"
  printf "INSERT INTO tool_catalog (id, tool_name) VALUES ('tc-109', 'second');\n" > "$d/$b/20260102000001_b.sql"
done
land "$d"
code="$(run_gate "$d")"
expect "a standing collision is reported, not failed" "$code" 0
grep -qi "standing collision" "$OUT" || fail "standing collision must still be reported"
rm -rf "$d"

# --- 12. an id mentioned only in a comment is not a claim ----------------------
# These migrations document their own id choices in prose; a scan that reads the
# prose invents a collision that is not there.
d="$(make_repo)"
for b in migrations migrations_pg; do
  printf "INSERT INTO tool_catalog (id, tool_name) VALUES ('tc-109', 'first');\n" > "$d/$b/20260101000001_a.sql"
  printf -- "-- the next free id after 'tc-109'\nINSERT INTO tool_catalog (id, tool_name) VALUES ('tc-110', 'second');\n" > "$d/$b/20260102000001_b.sql"
done
expect "an id named only in a comment is not a claim" "$(run_gate "$d")" 0
rm -rf "$d"

# --- 13. an unresolvable base falls back rather than disarming -----------------
# resolve_gate_base_ref treats a missing or HEAD-equal base as HEAD~1, which is
# what keeps this armed on a push to main, where checkout leaves
# origin/main == HEAD. A bogus ref must therefore still inspect the tip.
d="$(make_repo)"
printf 'SELECT 1;\n' > "$d/migrations/20260101000001_a.sql"
printf 'SELECT 1;\n' > "$d/migrations_pg/20260101000001_a.sql"
land "$d"
printf 'SELECT 2;\n' >> "$d/migrations/20260101000001_a.sql"
code=0
( cd "$d" && GATE_BASE_REF=refs/heads/does-not-exist "$UNDER_TEST" >"$OUT" 2>&1 ) || code=$?
expect "a bogus base still inspects the tip commit" "$code" 1
grep -q "20260101000001_a.sql" "$OUT" || fail "fallback base must still name the edited file"
rm -rf "$d"

echo
if [ "$failures" -gt 0 ]; then
  echo "❌ $failures case(s) failed"
  exit 1
fi
echo "✅ all cases passed"
