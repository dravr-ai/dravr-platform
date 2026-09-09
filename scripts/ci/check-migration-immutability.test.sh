#!/usr/bin/env bash
# ABOUTME: Fixture test for check-migration-immutability.sh — builds throwaway repos
# ABOUTME: and pins every branch, including the false negative that shipped first.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# An unverified gate is worse than no gate. The case that matters most here is
# case 6: the first draft walked the file's history from HEAD, so HEAD's own
# edited blob was always "in history" and EVERY modification read as an allowed
# restore. It reported ✅ on the real 119040a11 break it was written to catch.
# A gate whose carve-out swallows its own subject is the failure mode this file
# exists to prevent, so that case is pinned first and permanently.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="$SCRIPT_DIR/check-migration-immutability.sh"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

OUT="$(mktemp)"
trap 'rm -f "$OUT"' EXIT

# A repo with one published migration in both backends, committed as BASE.
make_repo() {
  local dir
  dir="$(mktemp -d)"
  git -C "$dir" init -q
  git -C "$dir" config user.email test@example.com
  git -C "$dir" config user.name test
  git -C "$dir" config commit.gpgsign false
  mkdir -p "$dir/migrations" "$dir/migrations_pg"
  for d in migrations migrations_pg; do
    printf 'CREATE TABLE IF NOT EXISTS a (id TEXT);\n' > "$dir/$d/20260101000001_a.sql"
  done
  git -C "$dir" add -A >/dev/null
  git -C "$dir" commit -qm base
  printf '%s\n' "$dir"
}

commit() { git -C "$1" add -A >/dev/null && git -C "$1" commit -qm "${2:-change}"; }

run_gate() { # $1 = dir, $2 = base ref; echoes exit code, output in $OUT
  local dir="$1" base="$2" code=0
  ( cd "$dir" && "$UNDER_TEST" "$base" >"$OUT" 2>&1 ) || code=$?
  printf '%s\n' "$code"
}

expect() { # $1 = label, $2 = actual, $3 = wanted
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1 (exit $2, wanted $3)"; sed 's/^/       /' "$OUT"
  fi
}

echo "check-migration-immutability.sh"

# --- 1. nothing touched: pass --------------------------------------------------
d="$(make_repo)"
echo "unrelated" > "$d/README.md"; commit "$d"
expect "unrelated change passes" "$(run_gate "$d" HEAD~1)" 0
rm -rf "$d"

# --- 2. a NEW migration, nothing edited: pass (the correct pattern) -------------
# What 4c1cd67e5 did instead of editing an applied file.
d="$(make_repo)"
for b in migrations migrations_pg; do
  printf 'INSERT INTO a (id) VALUES ('"'"'x'"'"');\n' > "$d/$b/20260102000001_b.sql"
done
commit "$d"
expect "new migration passes" "$(run_gate "$d" HEAD~1)" 0
rm -rf "$d"

# --- 3. the real 2026-09-08 bug: editing a published migration: FAIL ------------
d="$(make_repo)"
printf 'CREATE TABLE IF NOT EXISTS a (id TEXT, extra TEXT);\n' > "$d/migrations_pg/20260101000001_a.sql"
commit "$d"
code="$(run_gate "$d" HEAD~1)"
expect "editing a published migration fails" "$code" 1
grep -q "previously applied but has been modified" "$OUT" \
  || fail "failure must name the sqlx error the developer will see"
rm -rf "$d"

# --- 4. deleting a published migration: FAIL (VersionMissing at boot) -----------
d="$(make_repo)"
rm "$d/migrations_pg/20260101000001_a.sql"; commit "$d"
code="$(run_gate "$d" HEAD~1)"
expect "deleting a published migration fails" "$code" 1
grep -q "VersionMissing" "$OUT" || fail "deletion must name VersionMissing"
rm -rf "$d"

# --- 5. restoring the applied bytes: pass (the only correct remedy) -------------
# 3b08b7b79 / a6fbcae6a. A gate that blocked this would block its own fix.
d="$(make_repo)"
printf 'CREATE TABLE IF NOT EXISTS a (id TEXT, extra TEXT);\n' > "$d/migrations_pg/20260101000001_a.sql"
commit "$d" break
printf 'CREATE TABLE IF NOT EXISTS a (id TEXT);\n' > "$d/migrations_pg/20260101000001_a.sql"
commit "$d" restore
expect "restore to previously-committed bytes passes" "$(run_gate "$d" HEAD~1)" 0
grep -q "restored to previously-committed content" "$OUT" \
  || fail "a restore must be reported as such, not pass silently"
rm -rf "$d"

# --- 6. REGRESSION: a modification must not read as a restore ------------------
# The first draft walked history from HEAD, so the edited blob was always found
# and this returned 0. Pinned permanently.
d="$(make_repo)"
printf 'CREATE TABLE IF NOT EXISTS a (id TEXT, never_seen_before TEXT);\n' > "$d/migrations_pg/20260101000001_a.sql"
commit "$d"
code="$(run_gate "$d" HEAD~1)"
expect "a fresh edit is NOT swallowed by the restore carve-out" "$code" 1
grep -q "restored to previously-committed content" "$OUT" \
  && fail "a never-before-seen blob must not be reported as a restore"
rm -rf "$d"

# --- 7. added AND edited within the same push: pass ----------------------------
# Nothing deployed between the two commits, so no DB applied the first version.
d="$(make_repo)"
base="$(git -C "$d" rev-parse HEAD)"
printf 'SELECT 1;\n' > "$d/migrations_pg/20260103000001_c.sql"; commit "$d" add
printf 'SELECT 2;\n' > "$d/migrations_pg/20260103000001_c.sql"; commit "$d" amend
expect "migration added and edited in one push passes" "$(run_gate "$d" "$base")" 0
rm -rf "$d"

# --- 8. rename keeping version and bytes: pass ---------------------------------
# sqlx keys on the version, not the filename.
d="$(make_repo)"
git -C "$d" mv migrations_pg/20260101000001_a.sql migrations_pg/20260101000001_a_renamed.sql
commit "$d"
expect "rename keeping version and bytes passes" "$(run_gate "$d" HEAD~1)" 0
rm -rf "$d"

# --- 9. renumbering a published migration: FAIL --------------------------------
# To a DB holding the old number this is a deletion.
d="$(make_repo)"
git -C "$d" mv migrations_pg/20260101000001_a.sql migrations_pg/20260104000001_a.sql
commit "$d"
expect "renumbering a published migration fails" "$(run_gate "$d" HEAD~1)" 1
rm -rf "$d"

# --- 10. base ref that published nothing while HEAD has migrations: FAIL -------
# Fail closed rather than reporting green on a scan that inspected nothing.
d="$(mktemp -d)"
git -C "$d" init -q
git -C "$d" config user.email test@example.com
git -C "$d" config user.name test
git -C "$d" config commit.gpgsign false
mkdir -p "$d/migrations_pg"
echo x > "$d/README.md"; commit "$d" empty-base
printf 'SELECT 1;\n' > "$d/migrations_pg/20260101000001_a.sql"; commit "$d" first-migration
expect "first-ever migration passes (fork point genuinely had none)" "$(run_gate "$d" HEAD~1)" 0
rm -rf "$d"

# --- 11. a branch merely BEHIND main: pass -------------------------------------
# The published set is the FORK POINT, not main's tip. Comparing against the tip
# would read every migration main gained after the fork as "removed" and fail
# every branch that had not rebased.
d="$(make_repo)"
git -C "$d" branch -q feature
printf 'SELECT 1;\n' > "$d/migrations_pg/20260105000001_added_on_main.sql"; commit "$d" main-moved
git -C "$d" checkout -q feature
echo "unrelated work" > "$d/README.md"; commit "$d" branch-work
expect "branch behind main passes (baseline is the fork point)" "$(run_gate "$d" master)" 0
grep -q "is gone from HEAD" "$OUT" \
  && fail "a migration added on main after the fork must not read as removed"
rm -rf "$d"

echo
if [ "$failures" -gt 0 ]; then
  echo "❌ $failures case(s) failed"
  exit 1
fi
echo "✅ all cases passed"
