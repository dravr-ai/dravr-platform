#!/usr/bin/env bash
# ABOUTME: Fixture test for check-merge-risk.sh — builds throwaway repos and proves the gate fires on
# ABOUTME: an unmarked one-way change, passes with the ONE-WAY trailer and on a non-surface change.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# An unverified gate is worse than no gate: it reports ✅ forever and everyone
# believes the one-way doors are declared. Each case builds a throwaway repo
# whose base commit holds every surface the gate lists (read from the gate's
# own --list-surfaces, so the fixture cannot drift from it) plus two
# sqlx::migrate! calls, applies one change as HEAD, runs the real gate and
# asserts on its exit status and its output.
#
# The whole-set secret replace flag is assembled at run time below rather than
# written out: the gate scans every added non-comment line outside Markdown,
# and this file is one of them on the push that changes it.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/check-merge-risk.sh}"

# The explicit base each case passes outranks this, but a run nested inside
# pre-push-validate.sh inherits the parent's export; clear it so nothing here
# depends on the order of that rule.
unset GATE_BASE_REF

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT
OUT="$ROOT/gate.out"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

SET_SECRETS_FLAG="--$(printf set)-secrets"

if ! SURFACE_PATHS="$("$UNDER_TEST" --list-surfaces | cut -f1)" || [ -z "$SURFACE_PATHS" ]; then
  echo "❌ check-merge-risk.sh --list-surfaces printed nothing; there is no fixture to build"
  exit 1
fi

# Every fixture is addressed by path, and `git -C ""` is a documented no-op: an
# unset path would run `git commit` in the developer's own repository. Nothing
# here touches a directory that is not a throwaway under $ROOT.
require_fixture() {
  case "${1:-}" in
    "$ROOT"/*) [ -d "$1/.git" ] && return 0 ;;
  esac
  echo "❌ not a fixture repo under $ROOT: '${1:-}'" >&2
  exit 1
}

make_repo() {
  local dir path
  dir="$(mktemp -d "$ROOT/case-XXXXXX")"
  git -C "$dir" init -q
  git -C "$dir" config user.email test@example.com
  git -C "$dir" config user.name test
  git -C "$dir" config commit.gpgsign false
  while IFS= read -r path; do
    case "$path" in
      */) mkdir -p "$dir/$path"; printf 'seed\n' >"$dir/${path}seed" ;;
      *) mkdir -p "$dir/$(dirname "$path")"; printf '// seed\n' >"$dir/$path" ;;
    esac
  done <<< "$SURFACE_PATHS"
  # The migrator premise: one crate embedding its own link, one test target
  # reaching the root set, and a doc comment the scan must not read as a call.
  printf '[package]\nname = "pierre-database"\n' >"$dir/crates/pierre-database/Cargo.toml"
  mkdir -p "$dir/crates/pierre-database/src" "$dir/crates/pierre-server/tests"
  # shellcheck disable=SC2016 # the backticks are literal rustdoc, not a substitution
  printf '//! `sqlx::migrate!("./docs_only")` in prose\npub static M: Migrator = sqlx::migrate!("./migrations");\n' \
    >"$dir/crates/pierre-database/src/lib.rs"
  printf '[package]\nname = "pierre_mcp_server"\n' >"$dir/crates/pierre-server/Cargo.toml"
  printf 'static PG: Migrator = sqlx::migrate!("../../migrations_pg");\n' \
    >"$dir/crates/pierre-server/tests/schema_test.rs"
  printf 'base\n' >"$dir/README.md"
  printf '// renders the bodies\n' >"$dir/crates/pierre-email/src/templates.rs"
  mkdir -p "$dir/.github/workflows"
  printf 'jobs:\n  deploy:\n    steps:\n      - run: echo deploy\n' >"$dir/.github/workflows/deploy.yml"
  git -C "$dir" add -A >/dev/null
  git -C "$dir" commit -qm base
  printf '%s\n' "$dir"
}

commit() { # $1 = dir, $2 = message
  require_fixture "$1"
  git -C "$1" add -A >/dev/null
  git -C "$1" commit -qm "$2"
}

run_gate() { # $1 = dir, $2 = base ref; echoes the exit code, output in $OUT
  local code=0
  require_fixture "$1"
  ( cd "$1" && "$UNDER_TEST" "$2" >"$OUT" 2>&1 ) || code=$?
  printf '%s\n' "$code"
}

expect() { # $1 = label, $2 = actual exit, $3 = wanted exit
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1 (exit $2, wanted $3)"; sed 's/^/       /' "$OUT"
  fi
}

says() { # $1 = label, $2 = fixed string the output must carry
  grep -qF -- "$2" "$OUT" || { fail "$1 — output lacks: $2"; sed 's/^/       /' "$OUT"; }
}

echo "check-merge-risk.sh"

# --- 1. a non-surface change passes with no trailer at all ---------------------
d="$(make_repo)"
printf 'changed\n' >>"$d/README.md"; commit "$d" "docs: readme"
expect "non-surface change passes without a trailer" "$(run_gate "$d" HEAD~1)" 0
says "the pass reports what it verified" "no one-way surface touched"
rm -rf "$d"

# --- 2. an unmarked migration fails --------------------------------------------
d="$(make_repo)"
printf 'CREATE TABLE b (id TEXT);\n' >"$d/migrations/20261012000001_b.sql"
commit "$d" "feat(db): add table b"
expect "unmarked migration fails" "$(run_gate "$d" HEAD~1)" 1
says "the failure names the migration" "migrations/20261012000001_b.sql"
says "the failure prints the trailer to add" "Merge-Risk: ONE-WAY · <why>"
rm -rf "$d"

# --- 3. the same migration with the trailer passes -----------------------------
d="$(make_repo)"
printf 'CREATE TABLE b (id TEXT);\n' >"$d/migrations_pg/20261012000001_b.sql"
commit "$d" "feat(db): add table b

Merge-Risk: ONE-WAY · migration 20261012000001 adds table b"
expect "migration with a ONE-WAY trailer passes" "$(run_gate "$d" HEAD~1)" 0
says "the pass names the declaring commit" "declared by:"
rm -rf "$d"

# --- 4. one declaring commit covers the range -----------------------------------
d="$(make_repo)"
base="$(git -C "$d" rev-parse HEAD)"
printf 'CREATE TABLE c (id TEXT);\n' >"$d/migrations/20261012000002_c.sql"
commit "$d" "feat(db): add table c"
printf 'more\n' >>"$d/README.md"
commit "$d" "docs: note table c

Merge-Risk: ONE-WAY · migration 20261012000002 adds table c"
expect "a ONE-WAY trailer anywhere in the range declares it" "$(run_gate "$d" "$base")" 0
rm -rf "$d"

# --- 5. a two-way trailer does not declare a one-way change ---------------------
d="$(make_repo)"
printf 'CREATE TABLE d (id TEXT);\n' >"$d/migrations/20261012000003_d.sql"
commit "$d" "feat(db): add table d

Merge-Risk: two-way · blast radius: table d"
expect "a two-way trailer on a migration fails" "$(run_gate "$d" HEAD~1)" 1
rm -rf "$d"

# --- 6. ONE-WAY with no reason fails ---------------------------------------------
d="$(make_repo)"
printf 'CREATE TABLE e (id TEXT);\n' >"$d/migrations/20261012000004_e.sql"
commit "$d" "feat(db): add table e

Merge-Risk: ONE-WAY"
expect "a ONE-WAY trailer without a reason fails" "$(run_gate "$d" HEAD~1)" 1
rm -rf "$d"

# --- 7. the indented copy git merge --squash quotes does not count -------------
d="$(make_repo)"
printf 'CREATE TABLE f (id TEXT);\n' >"$d/migrations/20261012000005_f.sql"
commit "$d" "Squashed commit of the following:

    feat(db): add table f

    Merge-Risk: ONE-WAY · migration 20261012000005 adds table f"
expect "an indented, quoted trailer does not declare" "$(run_gate "$d" HEAD~1)" 1
rm -rf "$d"

# --- 8. a migration moved out of its directory is still a one-way change -------
# Only the destination would show under rename detection, and it is on no
# surface; the gate diffs with --no-renames so the removal side counts.
d="$(make_repo)"
printf 'SELECT 1;\n' >"$d/migrations/20261012000006_g.sql"
commit "$d" "feat(db): g

Merge-Risk: ONE-WAY · migration 20261012000006"
mkdir -p "$d/archive"
git -C "$d" mv migrations/20261012000006_g.sql archive/20261012000006_g.sql
commit "$d" "chore: move a migration"
expect "a rename out of a migrations dir fails" "$(run_gate "$d" HEAD~1)" 1
says "the failure names the removed path" "migrations/20261012000006_g.sql"
rm -rf "$d"

# --- 9. send path versus renderer ------------------------------------------------
d="$(make_repo)"
printf 'pub fn send() {}\n' >>"$d/crates/pierre-email/src/lib.rs"
commit "$d" "feat(email): change the send path"
expect "an unmarked change to the email send path fails" "$(run_gate "$d" HEAD~1)" 1
says "the failure names the surface kind" "email send path"
rm -rf "$d"

d="$(make_repo)"
printf '// a new template\n' >>"$d/crates/pierre-email/src/templates.rs"
commit "$d" "feat(email): reword a template"
expect "a template beside the send path passes" "$(run_gate "$d" HEAD~1)" 0
rm -rf "$d"

d="$(make_repo)"
printf 'pub fn tick() {}\n' >>"$d/crates/pierre-services/src/notification_digest_scheduler.rs"
commit "$d" "feat(notify): digest cadence"
expect "an unmarked change to a fan-out scheduler fails" "$(run_gate "$d" HEAD~1)" 1
rm -rf "$d"

# --- 10. secret and key definitions ----------------------------------------------
d="$(make_repo)"
printf 'resource "x" "y" {}\n' >"$d/infra/modules/secrets/main.tf"
commit "$d" "infra: add a secret"
expect "an unmarked Secret Manager change fails" "$(run_gate "$d" HEAD~1)" 1
rm -rf "$d"

d="$(make_repo)"
printf 'resource "x" "y" {}\n' >"$d/infra/modules/kms/main.tf"
commit "$d" "infra: kms rotation

Merge-Risk: ONE-WAY · KEK rotation period changes"
expect "a KMS change with the trailer passes" "$(run_gate "$d" HEAD~1)" 0
rm -rf "$d"

# --- 11. a whole-set secret replace, wherever a file runs it --------------------
d="$(make_repo)"
printf '      - run: gcloud run deploy api %s=A=a:latest\n' "$SET_SECRETS_FLAG" >>"$d/.github/workflows/deploy.yml"
commit "$d" "ci: deploy with secrets"
expect "an added whole-set secret replace fails" "$(run_gate "$d" HEAD~1)" 1
says "the failure quotes the line" "whole-set secret replace"
rm -rf "$d"

d="$(make_repo)"
printf '    secrets_update_strategy: %s\n' overwrite >>"$d/.github/workflows/deploy.yml"
commit "$d" "ci: authoritative secrets"
expect "deploy-cloudrun's overwrite strategy fails" "$(run_gate "$d" HEAD~1)" 1
rm -rf "$d"

d="$(make_repo)"
printf '      # never use %s, it replaces every secret\n' "$SET_SECRETS_FLAG" >>"$d/.github/workflows/deploy.yml"
# shellcheck disable=SC2016 # the backticks are literal Markdown, not a substitution
printf 'Never run `gcloud run services update %s`.\n' "$SET_SECRETS_FLAG" >>"$d/README.md"
commit "$d" "docs: warn against the whole-set replace"
expect "a comment or Markdown warning against it passes" "$(run_gate "$d" HEAD~1)" 0
rm -rf "$d"

# With -U0 an added line reading "++ x" prints as "+++ x". Read as a file
# header, it would rename the file to a Markdown one and skip the next line.
d="$(make_repo)"
printf '++ notes.md\n      - run: gcloud run deploy api %s=A=a:latest\n' "$SET_SECRETS_FLAG" \
  >>"$d/.github/workflows/deploy.yml"
commit "$d" "ci: header lookalike"
expect "an added line shaped like a file header does not hide the next" "$(run_gate "$d" HEAD~1)" 1
rm -rf "$d"

# --- 12. premise: a listed surface that moved fails, trailer or not ------------
d="$(make_repo)"
git -C "$d" mv crates/pierre-services/src/messaging_outbound.rs crates/pierre-services/src/outbound.rs
commit "$d" "refactor: rename the retry worker

Merge-Risk: ONE-WAY · moves the outbound retry worker"
expect "a listed surface missing from HEAD fails" "$(run_gate "$d" HEAD~1)" 1
says "the failure names the stale entry" "crates/pierre-services/src/messaging_outbound.rs"
rm -rf "$d"

# --- 13. premise: a migrate! embedding an unlisted directory fails -------------
d="$(make_repo)"
mkdir -p "$d/crates/pierre-other/src" "$d/crates/pierre-other/migrations_other"
printf '[package]\nname = "pierre-other"\n' >"$d/crates/pierre-other/Cargo.toml"
printf 'pub static M: Migrator = sqlx::migrate!("./migrations_other");\n' >"$d/crates/pierre-other/src/lib.rs"
printf 'SELECT 1;\n' >"$d/crates/pierre-other/migrations_other/20261012000001_x.sql"
commit "$d" "feat: a second migration set"
expect "a migration directory the list does not hold fails" "$(run_gate "$d" HEAD~1)" 1
says "the failure names the unlisted directory" "embeds crates/pierre-other/migrations_other"
rm -rf "$d"

# --- 14. premise: no migrate! call at all verified nothing ---------------------
d="$(make_repo)"
git -C "$d" rm -q crates/pierre-database/src/lib.rs crates/pierre-server/tests/schema_test.rs
commit "$d" "chore: drop the migrators"
expect "a tree with no sqlx::migrate! call fails closed" "$(run_gate "$d" HEAD~1)" 1
says "the failure says the premise verified nothing" "verified nothing"
rm -rf "$d"

# --- 15. no common ancestor with the base fails closed -------------------------
d="$(make_repo)"
trunk="$(git -C "$d" symbolic-ref --short HEAD)"
git -C "$d" checkout -q --orphan unrelated
git -C "$d" rm -rqf .
printf 'other\n' >"$d/other.txt"
commit "$d" "unrelated root"
unrelated="$(git -C "$d" rev-parse HEAD)"
git -C "$d" checkout -q "$trunk"
expect "a base sharing no history with HEAD fails closed" "$(run_gate "$d" "$unrelated")" 1
says "the failure says why it refuses" "no common ancestor"
rm -rf "$d"

# --- 16. a burst push is judged as one range, and never by its tip alone -------
# c1 declares the push; c2 touches a send path and says nothing of its own. A
# schedule or dispatch run hands the gate HEAD as its base; judging the tip's
# message alone there would fail main a day after the push itself passed.
d="$(make_repo)"
before="$(git -C "$d" rev-parse HEAD)"
printf 'CREATE TABLE h (id TEXT);\n' >"$d/migrations/20261012000007_h.sql"
commit "$d" "feat(db): add table h

Merge-Risk: ONE-WAY · migration 20261012000007 adds table h"
printf 'pub fn tick() {}\n' >>"$d/crates/pierre-services/src/agent_followup_scheduler.rs"
commit "$d" "feat(notify): follow up on table h"
expect "the burst passes against the push's own base" "$(run_gate "$d" "$before")" 0
expect "the same tip with HEAD as its base judges nothing" "$(run_gate "$d" HEAD)" 0
says "the no-range run says why" "no push range, nothing to judge"
tip="$(git -C "$d" rev-parse HEAD)"
git -C "$d" reset -q --hard HEAD~1
expect "a base ahead of HEAD judges nothing" "$(run_gate "$d" "$tip")" 0
says "the behind-base run says why" "no push range, nothing to judge"
rm -rf "$d"

# --- 17. a base that is not a commit here names no range -----------------------
# A before-sha a force-push left behind, or one deeper than a shallow checkout.
d="$(make_repo)"
printf 'CREATE TABLE i (id TEXT);\n' >"$d/migrations/20261012000008_i.sql"
commit "$d" "feat(db): add table i"
expect "a base missing from the checkout judges nothing" \
  "$(run_gate "$d" 0123456789abcdef0123456789abcdef01234567)" 0
says "the missing-base run names the base" "is not a commit in this checkout"
rm -rf "$d"

# --- 18. with no argument, $GATE_BASE_REF is the base -------------------------
d="$(make_repo)"
before="$(git -C "$d" rev-parse HEAD)"
printf 'CREATE TABLE j (id TEXT);\n' >"$d/migrations/20261012000009_j.sql"
commit "$d" "feat(db): add table j"
code=0
( cd "$d" && GATE_BASE_REF="$before" "$UNDER_TEST" >"$OUT" 2>&1 ) || code=$?
expect "an unmarked migration fails against an inherited base" "$code" 1
says "the inherited base is the one judged" "vs ${before}"
code=0
( cd "$d" && "$UNDER_TEST" >"$OUT" 2>&1 ) || code=$?
expect "no argument and no origin/main judges nothing" "$code" 0
says "the default base is origin/main" "base origin/main is not a commit"
rm -rf "$d"

# --- 19. CI judges the range Tier 1o judges -------------------------------------
# Pre-push hands the gate origin/main, so a branch is judged from where it
# forked: it declares ONE-WAY once and its squash restates it. The fast gate
# once handed it the push's before-sha, which on a branch covers that push
# alone, so a second push re-touching a migration the first push declared
# passed before the push and failed in CI. The fast gate's own step is lifted
# out of the workflow and run here, so the range CI judges is the text the
# runner executes; a rename or re-indent of the step fails the lift.
WORKFLOW="$SCRIPT_DIR/../../.github/workflows/ci-backend.yml"
STEP="$ROOT/merge-risk-step.sh"
awk -v name="      - name: Merge-Risk one-way doors" '
  $0 == name { in_step = 1; next }
  in_step && !in_run && /^      - / { exit }
  in_step && /^        run: \|$/ { in_run = 1; next }
  in_run {
    if ($0 ~ /^          / || $0 ~ /^[[:space:]]*$/) { sub(/^          /, ""); print; next }
    exit
  }
' "$WORKFLOW" >"$STEP"
# shellcheck disable=SC2016 # the step's own text, matched literally
if ! grep -qF './scripts/ci/check-merge-risk.sh "$base"' "$STEP"; then
  echo "❌ could not lift the Merge-Risk step out of $WORKFLOW — its name or indentation changed"
  exit 1
fi

# A bare origin the fixture pushes to, serving a sha in a want as GitHub does.
add_origin() { # $1 = fixture dir; renames its branch to main and pushes it
  local origin="$1.origin.git"
  require_fixture "$1"
  git init -q --bare "$origin"
  git -C "$origin" config uploadpack.allowReachableSHA1InWant true
  # A developer's push.negotiate fails its handshake against a local bare repo
  # and prints a fatal line on every push, though the push itself lands.
  git -C "$1" config push.negotiate false
  git -C "$1" branch -M main
  git -C "$1" remote add origin "file://$origin"
  git -C "$1" push -q origin main
}

# $1 = dir, $2 = event, $3 = pushed ref, $4 = the event's base: the pull
# request's base sha, or the push's before-sha. Echoes the exit code, output in
# $OUT. The gate and the base resolver sit where the step calls them only while
# the step runs, so no fixture commit picks them up.
run_step() {
  local code=0 pr_base="" before=""
  require_fixture "$1"
  if [ "$2" = pull_request ]; then pr_base="$4"; else before="$4"; fi
  mkdir -p "$1/scripts/ci"
  cp "$UNDER_TEST" "$1/scripts/ci/check-merge-risk.sh"
  cp "$SCRIPT_DIR/resolve-push-base.sh" "$1/scripts/ci/resolve-push-base.sh"
  ( cd "$1" && PR_BASE="$pr_base" BEFORE="$before" EVENT_NAME="$2" PUSHED_REF="$3" bash -e "$STEP" >"$OUT" 2>&1 ) || code=$?
  rm -f "$1/scripts/ci/check-merge-risk.sh" "$1/scripts/ci/resolve-push-base.sh"
  printf '%s\n' "$code"
}

# A second push re-touches a migration the first push declared.
d="$(make_repo)"
add_origin "$d"
git -C "$d" checkout -qb features/x
printf 'CREATE TABLE k (id TEXT);\n' >"$d/migrations/20261012000010_k.sql"
commit "$d" "feat(db): add table k

Merge-Risk: ONE-WAY · migration 20261012000010 adds table k"
git -C "$d" push -q origin features/x
first_push="$(git -C "$d" rev-parse HEAD)"
printf 'CREATE TABLE k (id TEXT, note TEXT);\n' >"$d/migrations/20261012000010_k.sql"
commit "$d" "fix(db): a column table k needs before it ever applies"
git -C "$d" push -q origin features/x
expect "Tier 1o's view: the branch declared it once" "$(run_gate "$d" origin/main)" 0
expect "the second push alone does not carry the declaration" "$(run_gate "$d" "$first_push")" 1
expect "CI judges a branch push from its fork point" \
  "$(run_step "$d" push refs/heads/features/x "$first_push")" 0
says "the step names the fork point" "where this branch forked from main"
says "the fork point's range holds the declaring commit" "declared by:"
expect "a pull request is judged against its base" \
  "$(run_step "$d" pull_request refs/pull/7/merge "$(git -C "$d" rev-parse main)")" 0
rm -rf "$d" "$d.origin.git"

# The mirror: an undeclared migration on the branch's first push stays a
# failure on a later push that touches nothing itself.
d="$(make_repo)"
add_origin "$d"
git -C "$d" checkout -qb features/y
printf 'CREATE TABLE l (id TEXT);\n' >"$d/migrations/20261012000011_l.sql"
commit "$d" "feat(db): add table l"
git -C "$d" push -q origin features/y
first_push="$(git -C "$d" rev-parse HEAD)"
printf 'table l\n' >>"$d/README.md"
commit "$d" "docs: table l"
git -C "$d" push -q origin features/y
expect "Tier 1o's view: the branch never declared it" "$(run_gate "$d" origin/main)" 1
expect "CI fails the same branch on its second push" \
  "$(run_step "$d" push refs/heads/features/y "$first_push")" 1
says "the failure names the first push's migration" "migrations/20261012000011_l.sql"
rm -rf "$d" "$d.origin.git"

# On main the fork point is HEAD itself, so the before-sha stays the base.
d="$(make_repo)"
add_origin "$d"
before="$(git -C "$d" rev-parse HEAD)"
printf 'CREATE TABLE m (id TEXT);\n' >"$d/migrations/20261012000012_m.sql"
commit "$d" "feat(db): add table m"
git -C "$d" push -q origin main
expect "a push to main is judged against its before-sha" \
  "$(run_step "$d" push refs/heads/main "$before")" 1
says "the main push names the migration" "migrations/20261012000012_m.sql"
rm -rf "$d" "$d.origin.git"

# A branch sharing no history with main has no fork point to judge from.
d="$(make_repo)"
add_origin "$d"
git -C "$d" checkout -q --orphan features/z
printf 'CREATE TABLE n (id TEXT);\n' >"$d/migrations/20261012000013_n.sql"
commit "$d" "feat(db): an unrelated root"
git -C "$d" push -q origin features/z
expect "a branch push with no fork point in reach judges nothing" \
  "$(run_step "$d" push refs/heads/features/z "$(git -C "$d" rev-parse HEAD)")" 0
says "the skip says why" "no fork point from main"
rm -rf "$d" "$d.origin.git"

# An origin the runner cannot reach is a crash: the step fails rather than
# reading it as "no fork point" and skipping.
d="$(make_repo)"
add_origin "$d"
git -C "$d" checkout -qb features/w
printf 'CREATE TABLE o (id TEXT);\n' >"$d/migrations/20261012000014_o.sql"
commit "$d" "feat(db): add table o"
git -C "$d" push -q origin features/w
rm -rf "$d.origin.git"
expect "a branch push whose fork-point lookup crashes fails" \
  "$(run_step "$d" push refs/heads/features/w "$(git -C "$d" rev-parse HEAD~1)")" 3
says "the failure names the resolver" "resolve-push-base.sh failed"
rm -rf "$d"

echo ""
if [ "$failures" -gt 0 ]; then
  echo "❌ check-merge-risk.test.sh: $failures failure(s)"
  exit 1
fi
echo "✅ check-merge-risk.test.sh: every case passed"
