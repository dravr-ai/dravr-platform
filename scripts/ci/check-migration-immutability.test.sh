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

# --- 12. the gate reads MIGRATION_FREEZE_BASE_REF ahead of GATE_BASE_REF -------
# ci-backend.yml hands every diff gate the push's before-sha as $GATE_BASE_REF,
# and this one alone the fork point as $MIGRATION_FREEZE_BASE_REF.
# architectural-validation.sh calls it with no argument, so the precedence is
# the wiring: reading GATE_BASE_REF first judges a branch's second push from
# its first and freezes a migration main never saw.
d="$(make_repo)"
fork="$(git -C "$d" rev-parse HEAD)"
printf 'SELECT 1;\n' > "$d/migrations_pg/20260106000001_branch.sql"; commit "$d" push-1
before="$(git -C "$d" rev-parse HEAD)"
printf 'SELECT 2;\n' > "$d/migrations_pg/20260106000001_branch.sql"; commit "$d" push-2
code=0
( cd "$d" && GATE_BASE_REF="$before" MIGRATION_FREEZE_BASE_REF="$fork" "$UNDER_TEST" >"$OUT" 2>&1 ) || code=$?
expect "the freeze base wins over GATE_BASE_REF" "$code" 0
grep -qF "2 published migration(s) intact vs $fork" "$OUT" \
  || fail "12. the green must be the gate's verdict against the freeze base, not a skip"
code=0
( cd "$d" && GATE_BASE_REF="$fork" MIGRATION_FREEZE_BASE_REF="$before" "$UNDER_TEST" >"$OUT" 2>&1 ) || code=$?
expect "and still fires when the freeze base published the migration" "$code" 1
rm -rf "$d"

# --- 13-16. what CI resolves, end to end (carnet#1318) --------------------------
# The gate alone cannot tell a branch's previous push from main: handed a sha,
# it protects what that sha published. The discriminating input is the base
# resolve-push-base.sh prints, so these cases build an origin, clone it the way
# actions/checkout does (the pushed sha alone, depth 1, no origin/main) and feed
# the resolver's answer to the gate. Case 13's `before` line pins the shape that
# shipped: a branch-only migration edited in a second push failed the gate.
RESOLVER="$SCRIPT_DIR/resolve-push-base.sh"
ZERO=0000000000000000000000000000000000000000
CI_ROOT="$(mktemp -d)"
ORIGIN="$CI_ROOT/origin.git"
WORK="$CI_ROOT/work"
git init -q --bare "$ORIGIN"
git -C "$ORIGIN" config uploadpack.allowReachableSHA1InWant true
git init -q "$WORK"
git -C "$WORK" config user.email test@example.com
git -C "$WORK" config user.name test
git -C "$WORK" config commit.gpgsign false
git -C "$WORK" config push.negotiate false
git -C "$WORK" checkout -qb main
mkdir -p "$WORK/migrations_pg"
printf 'CREATE TABLE IF NOT EXISTS a (id TEXT);\n' > "$WORK/migrations_pg/20260101000001_a.sql"
commit "$WORK" published
MAIN_BEFORE="$(git -C "$WORK" rev-parse HEAD)"
echo "main moved" > "$WORK/README.md"; commit "$WORK" main-2
# features/branch-only: push 1 adds a migration, push 2 edits it.
git -C "$WORK" checkout -qb features/branch-only "$MAIN_BEFORE"
printf 'SELECT 1;\n' > "$WORK/migrations_pg/20260107000001_b.sql"; commit "$WORK" push-1
ONLY_BEFORE="$(git -C "$WORK" rev-parse HEAD)"
printf 'SELECT 2;\n' > "$WORK/migrations_pg/20260107000001_b.sql"; commit "$WORK" push-2
ONLY="$(git -C "$WORK" rev-parse HEAD)"
# features/edits-main: push 1 is unrelated, push 2 edits the migration main has.
git -C "$WORK" checkout -qb features/edits-main "$MAIN_BEFORE"
echo "unrelated" > "$WORK/notes.md"; commit "$WORK" push-1
EDITS_BEFORE="$(git -C "$WORK" rev-parse HEAD)"
printf 'CREATE TABLE IF NOT EXISTS a (id TEXT, extra TEXT);\n' > "$WORK/migrations_pg/20260101000001_a.sql"
commit "$WORK" push-2
EDITS="$(git -C "$WORK" rev-parse HEAD)"
# main itself: a push editing the migration it published one push earlier.
git -C "$WORK" checkout -q main
MAIN_PREV="$(git -C "$WORK" rev-parse HEAD)"
printf 'CREATE TABLE IF NOT EXISTS a (id TEXT, edited_on_main TEXT);\n' > "$WORK/migrations_pg/20260101000001_a.sql"
commit "$WORK" main-edit
MAIN_EDIT="$(git -C "$WORK" rev-parse HEAD)"
git -C "$WORK" remote add origin "file://$ORIGIN"
git -C "$WORK" push -q origin main features/branch-only features/edits-main

# The base CI hands the gate is the text the runner executes, so the fast
# gate's own step is lifted out of the workflow and run here; a rename or
# re-indent of the step fails the lift. The Architectural Validation step must
# pass its output on under the name the gate reads.
WORKFLOW="$SCRIPT_DIR/../../.github/workflows/ci-backend.yml"
STEP="$CI_ROOT/freeze-base-step.sh"
awk -v name="      - name: Resolve the migration-freeze base" '
  $0 == name { in_step = 1; next }
  in_step && !in_run && /^      - / { exit }
  in_step && /^        run: \|$/ { in_run = 1; next }
  in_run {
    if ($0 ~ /^          / || $0 ~ /^[[:space:]]*$/) { sub(/^          /, ""); print; next }
    exit
  }
' "$WORKFLOW" >"$STEP"
if ! grep -qF './scripts/ci/resolve-push-base.sh fork' "$STEP"; then
  echo "❌ could not lift the migration-freeze base step out of $WORKFLOW — its name, indentation or mode changed"
  exit 1
fi
# shellcheck disable=SC2016 # the workflow's own expression, matched literally
grep -qF 'MIGRATION_FREEZE_BASE_REF: ${{ steps.freeze_base.outputs.sha }}' "$WORKFLOW" \
  || fail "the Architectural Validation step must pass the freeze base on as MIGRATION_FREEZE_BASE_REF"

# $1 = branch, $2 = pushed sha, $3 = before-sha, $4 = "step" (the workflow's
# lifted step) or "before" (the push's before-sha, the base that shipped the
# bug), $5 = clone depth. Clones as actions/checkout does, resolves the base,
# runs the gate on it; echoes the gate's exit code, or "resolve exit <n>" when
# there was no base to hand it. The branch `step` cases clone 1 deep: the fork
# point lies outside the checkout, and the resolver's own fetch has to bring
# the history merge-base needs. The rest clone 50 deep, as the fast gate does:
# on main the resolver returns the before-sha without deepening, and a 1-deep
# clone would hide the `before` bug (its before-sha would read as no ancestor
# and fall back to the fork point, which a 50-deep CI checkout never did).
ci_gate() {
  local co="$CI_ROOT/co" base code=0
  rm -rf "$co"
  git init -q "$co"
  git -C "$co" remote add origin "file://$ORIGIN"
  git -C "$co" fetch -q --no-tags --depth="$5" origin "+$2:refs/remotes/origin/$1"
  git -C "$co" checkout -q --detach "$2"
  if [ "$4" = step ]; then
    # The step calls the resolver by its repo path and writes $GITHUB_OUTPUT.
    mkdir -p "$co/scripts/ci"
    cp "$RESOLVER" "$co/scripts/ci/resolve-push-base.sh"
    : >"$CI_ROOT/github-output"
    ( cd "$co" && PR_BASE="" BEFORE="$3" EVENT_NAME=push PUSHED_REF="refs/heads/$1" \
      GITHUB_OUTPUT="$CI_ROOT/github-output" bash -e "$STEP" >/dev/null 2>&1 ) || code=$?
    rm -rf "$co/scripts"
    base="$(sed -n 's/^sha=//p' "$CI_ROOT/github-output")"
  else
    base="$(cd "$co" && PR_BASE="" BEFORE="$3" EVENT_NAME=push PUSHED_REF="refs/heads/$1" \
      "$RESOLVER" before 2>/dev/null)" || code=$?
  fi
  if [ "$code" -ne 0 ] || [ -z "$base" ]; then echo "resolve exit $code"; return; fi
  ( cd "$co" && MIGRATION_FREEZE_BASE_REF="$base" "$UNDER_TEST" >"$OUT" 2>&1 ) || code=$?
  printf '%s\n' "$code"
}

# A red must be the edit being caught, never a base the gate could not read.
expect_edit_caught() { # $1 = label, $2 = version
  grep -q "version $2 was already published" "$OUT" \
    || fail "$1: the red must name the edited version, not an unreadable base"
}

# A green must be the gate's verdict against the fork point: its "no base ref;
# skipping" line exits 0 too, and so would a base the checkout cannot resolve.
expect_intact_vs_fork() { # $1 = label
  grep -qF "1 published migration(s) intact vs $MAIN_BEFORE" "$OUT" \
    || fail "$1: the green must judge from the fork point $MAIN_BEFORE, not skip"
}

expect "13. CI shape: a branch-only migration edited in a second push passes" \
  "$(ci_gate features/branch-only "$ONLY" "$ONLY_BEFORE" step 1)" 0
expect_intact_vs_fork "13"
expect "13. ...and judging it from the before-sha is the bug: it fires" \
  "$(ci_gate features/branch-only "$ONLY" "$ONLY_BEFORE" before 50)" 1
expect_edit_caught "13" 20260107000001
expect "14. CI shape: a branch editing a migration main published still fails" \
  "$(ci_gate features/edits-main "$EDITS" "$EDITS_BEFORE" step 1)" 1
expect_edit_caught "14" 20260101000001
grep -q "previously applied but has been modified" "$OUT" \
  || fail "14. the failure must name the sqlx error"
expect "15. CI shape: a first push (all-zero before-sha) judges from the fork point" \
  "$(ci_gate features/branch-only "$ONLY" "$ZERO" step 1)" 0
expect_intact_vs_fork "15"
expect "16. CI shape: a main push editing what main published one push ago fails" \
  "$(ci_gate main "$MAIN_EDIT" "$MAIN_PREV" step 50)" 1
expect_edit_caught "16" 20260101000001
rm -rf "$CI_ROOT"

echo
if [ "$failures" -gt 0 ]; then
  echo "❌ $failures case(s) failed"
  exit 1
fi
echo "✅ all cases passed"
