#!/usr/bin/env bash
# ABOUTME: Fails a push whose diff touches a one-way-door surface (a migrations dir, secret or key
# ABOUTME: definitions, a whole-set secret replace, an athlete-facing send path) unless a commit says Merge-Risk: ONE-WAY.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists (carnet#663):
#   Most changes to main roll back: revert the commit, redeploy, and the system
#   is where it was. A few do not, and nothing at commit or squash time told
#   them apart. A migration a deploy applied stays applied after the revert, a
#   destroyed secret takes with it the versions only Secret Manager held, and a
#   notification or email that went out cannot be called back. So every squash
#   to main ends with a Merge-Risk trailer (.savoir/platform/AGENTS.md, review-standards skill):
#
#     Merge-Risk: two-way · blast radius: <area>
#     Merge-Risk: ONE-WAY · <why it cannot be rolled back>
#
#   Reviewers skim a two-way door and read a one-way door in full. This gate
#   makes the one-way declaration impossible to forget where it matters: when
#   the diff touches a surface listed below, some commit in the range must
#   carry the ONE-WAY line, or the push fails. It never judges a two-way
#   trailer; whether a change is safe to roll back elsewhere is the reviewer's
#   call.
#
# Scope: BASE..HEAD, with BASE narrowed to its merge-base with HEAD. Pre-push
#   Tier 1o hands it origin/main: a branch is judged from where it forked, a
#   push to main from what the remote holds. The fast gate hands it the same
#   commits: a pull request's base, a push to main's before-sha, and a branch
#   push's fork point from main, never that push's own before-sha, which would
#   miss a declaration an earlier push of the branch carried (ci-backend.yml's
#   Merge-Risk step). One ONE-WAY line anywhere in the range declares it for
#   the range: a branch declares once and the squash that lands it carries its
#   work in one message, and a burst push to main is one review. The
#   trailer is matched at the start of a line, as bilan matches
#   Reviewed-Standards, so the indented copies `git merge --squash` quotes from
#   the branch's commits do not count — the squash message must say it itself.
#   Only a push or a PR names that range, so the gate judges nothing when the
#   base names none: HEAD adds no commit over it (a schedule or dispatch run,
#   a branch level with main), or it is not a commit in the checkout. It never
#   narrows to the tip alone, which gate-base-ref.sh's HEAD~1 fallback would.
#
# Two premises are checked on every run, so the surface list cannot go stale
#   without a sound: every listed path still exists in HEAD, and every
#   sqlx::migrate! invocation in HEAD's Rust sources embeds a directory the
#   list holds. A moved send module or a new migrations directory fails here
#   until the list follows it, instead of leaving the gate guarding a path
#   nothing uses.
#
# Usage: check-merge-risk.sh [base-ref]     (default: $GATE_BASE_REF, else origin/main)
#        check-merge-risk.sh --list-surfaces
set -euo pipefail

# Each entry is "<path>|<what it is>". A path ending in / covers everything
# under it; any other path is exactly that file (or symlink).
SURFACES=(
  # sqlx records a SHA-384 of every migration it applies and re-checks it on
  # each boot, so an applied migration can be neither edited nor removed
  # (check-migration-immutability.sh). Reverting the commit does not move the
  # schema back either: the PostgreSQL migrator runs with ignore_missing so
  # that a rollback deploy still boots — the older binary, on the newer schema.
  "migrations/|sqlx migration (SQLite)"
  "migrations_pg/|sqlx migration (PostgreSQL)"
  # The symlinks pierre-database's sqlx::migrate! calls embed through
  # (src/database/migrations.rs, src/backends/postgres/mod.rs). Retargeting
  # one swaps the whole set the next deploy applies.
  "crates/pierre-database/migrations|embedded migration set link (SQLite)"
  "crates/pierre-database/migrations_pg|embedded migration set link (PostgreSQL)"
  # Secret Manager definitions. Terraform creates placeholders and the real
  # values are added out of band with `gcloud secrets versions add`
  # (infra/README.md), so a destroyed secret takes with it versions no apply
  # can recreate. The database password and the master encryption key are
  # generated here by random_password: replacing either rotates a credential
  # the running database or the stored ciphertext depends on.
  "infra/modules/secrets/|Secret Manager definition"
  # The KMS key encryption key that wraps the database DEK (ADR-017). Its own
  # lifecycle block says why: destroying it leaves every wrapped DEK, and with
  # it all data at rest, permanently unrecoverable. The 2026-09-30 incident
  # (carnet#696) was a replaced DEK: no stored secret decrypted for three hours.
  "infra/modules/kms/|KMS key encryption key"
  # Athlete-facing sends. A notification or email that went out stays out: a
  # revert stops the next one, never the ones delivered. Listed is what
  # decides who receives what, when, and on which channel: the send
  # primitives every delivery passes through, and the timers that send to
  # many athletes on one tick. Left out on purpose: the renderers (the email
  # templates, notification_text, the localizer), whose wrong sentence the
  # next deploy corrects for the next send, and the emails a user requests
  # for themselves (a reset or verification code), which reach one requester.
  #
  # ResendEmailService, the process's one Resend send path.
  "crates/pierre-email/src/lib.rs|email send path"
  # The dispatch facade (dravr-commere's persist + Expo push pipeline, then the
  # channel sink), and the one fire-and-forget dispatch per product event.
  "crates/pierre-notifications/src/lib.rs|notification dispatch"
  "crates/pierre-notifications/src/triggers.rs|notification triggers"
  # The push-tier ladder and the persona gate enforcing it: what pushes now
  # and what waits for a digest.
  "crates/pierre-notifications/src/policy.rs|push-tier policy"
  "crates/pierre-services/src/persona_notification_policy_gate.rs|push-tier policy gate"
  # Channel delivery: the sink for dispatched notifications, the proactive
  # send to every channel a user linked, and the worker that re-sends a
  # queued failed send.
  "crates/pierre-services/src/notification_channel_sink.rs|notification channel sink"
  "crates/pierre-services/src/messaging_broadcast.rs|proactive channel send"
  "crates/pierre-services/src/messaging_outbound.rs|outbound retry worker"
  # Timers that fan out to many athletes or groups at once.
  "crates/pierre-services/src/notification_digest_scheduler.rs|notification digest scheduler"
  "crates/pierre-services/src/agent_followup_scheduler.rs|agent follow-up scheduler"
  "crates/pierre-routes-groups/src/group_digest_scheduler.rs|group digest scheduler"
  "crates/pierre-server/src/services/group_chat_poster.rs|group chat poster"
)

# A whole-set secret replace on Cloud Run, wherever a file would run it. The
# set-secrets flag of `gcloud run deploy|services update|jobs update` replaces
# every secret the service mounts with the list given ("replaces ALL secrets
# and breaks the service", vault runbook Configuring Messaging Channels), the
# clear-secrets flag with none, and deploy-cloudrun's `secrets_update_strategy:
# overwrite` does the same through the action. Its effect lands on a live
# revision when it runs, outside any commit, so reverting the commit that ran
# it undoes nothing. Matched on added lines in any file but Markdown, skipping
# comment lines, so prose and comments that warn against it do not trip it.
SECRET_REPLACE_RE='(^|[^[:alnum:]_-])--(set|clear)-secrets([^[:alnum:]_-]|$)|secrets_update_strategy:[[:space:]]*["'"'"']?overwrite'

# The declaration. A reason after ONE-WAY is required: the line is what the
# reviewer reads first, and "ONE-WAY" alone tells them nothing about why.
ONE_WAY_RE='^Merge-Risk:[[:space:]]*ONE-WAY[^[:alnum:]]+[[:alnum:]]'

if [ "${1:-}" = "--list-surfaces" ]; then
  for entry in "${SURFACES[@]}"; do
    printf '%s\t%s\n' "${entry%%|*}" "${entry#*|}"
  done
  exit 0
fi

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'

fail_closed() {
  echo -e "${RED}❌ merge-risk: $1${NC}"
  shift
  local line
  for line in "$@"; do echo "   $line"; done
  exit 1
}

# Lexical normalisation, no symlink resolution: crates/x/../../migrations ->
# migrations. Portable where `realpath -m -s --relative-to` is not (BSD).
normalize_path() {
  local part
  local -a parts out=()
  IFS=/ read -r -a parts <<< "$1"
  for part in "${parts[@]}"; do
    case "$part" in
      ''|.) ;;
      ..)
        if [ "${#out[@]}" -gt 0 ] && [ "${out[${#out[@]}-1]}" != ".." ]; then
          out=("${out[@]:0:${#out[@]}-1}")
        else
          out+=("..")
        fi
        ;;
      *) out+=("$part") ;;
    esac
  done
  local IFS=/
  printf '%s\n' "${out[*]}"
}

# Sets SURFACE_LABEL to the label of the surface covering a path, or to the
# empty string when none does. A global rather than an echo, so a diff of a
# thousand paths costs no subshell per path.
SURFACE_LABEL=""
match_surface() {
  local path="$1" entry p
  SURFACE_LABEL=""
  for entry in "${SURFACES[@]}"; do
    p="${entry%%|*}"
    case "$p" in
      */) if [ "$path" = "${p%/}" ] || [ "${path#"$p"}" != "$path" ]; then
            SURFACE_LABEL="${entry#*|}"; return 0
          fi ;;
      *) if [ "$path" = "$p" ]; then SURFACE_LABEL="${entry#*|}"; return 0; fi ;;
    esac
  done
}

# --- premise 1: every listed surface still exists ------------------------------
stale=()
for entry in "${SURFACES[@]}"; do
  p="${entry%%|*}"
  git cat-file -e "HEAD:${p%/}" 2>/dev/null || stale+=("$p  (${entry#*|})")
done
if [ "${#stale[@]}" -gt 0 ]; then
  fail_closed "${#stale[@]} listed one-way surface(s) no longer exist in HEAD:" \
    "${stale[@]}" \
    "" \
    "The module moved or was renamed, so the gate would guard a path nothing uses." \
    "Point its entry in SURFACES (scripts/ci/check-merge-risk.sh) at the new path."
fi

# --- premise 2: every sqlx::migrate! embeds a listed directory -----------------
# git grep exits 1 on no match, so only a status above 1 is an error.
migrate_hits=""
if migrate_hits="$(git grep -n -E 'migrate!\(' HEAD -- '*.rs')"; then
  :
else
  rc=$?
  [ "$rc" -eq 1 ] || fail_closed "git grep for sqlx::migrate! failed (exit $rc)."
fi

# The nearest directory above a file holding a Cargo.toml in HEAD: the
# CARGO_MANIFEST_DIR sqlx resolves a migrate! argument against.
manifest_dir_of() {
  local probe
  probe="$(dirname "$1")"
  while [ "$probe" != "." ]; do
    if git cat-file -e "HEAD:$probe/Cargo.toml" 2>/dev/null; then
      printf '%s\n' "$probe"
      return 0
    fi
    probe="$(dirname "$probe")"
  done
  git cat-file -e "HEAD:Cargo.toml" 2>/dev/null || return 1
  printf '.\n'
}

MIGRATE_ARG_RE='migrate!\([[:space:]]*("([^"]*)")?[[:space:]]*\)'
migrators=0
unlisted=()
while IFS= read -r hit; do
  [ -n "$hit" ] || continue
  hit="${hit#HEAD:}"
  file="${hit%%:*}"
  rest="${hit#*:}"
  lineno="${rest%%:*}"
  code="${rest#*:}"
  trimmed="${code#"${code%%[![:space:]]*}"}"
  case "$trimmed" in //*) continue ;; esac
  if [[ ! "$code" =~ $MIGRATE_ARG_RE ]]; then
    unlisted+=("$file:$lineno  a migrate! call whose directory this scan cannot read")
    continue
  fi
  # sqlx resolves the argument against CARGO_MANIFEST_DIR, and an empty call
  # means ./migrations.
  arg="${BASH_REMATCH[2]:-migrations}"
  if ! crate_dir="$(manifest_dir_of "$file")"; then
    unlisted+=("$file:$lineno  no Cargo.toml above it, so its directory cannot be resolved")
    continue
  fi
  embedded="$(normalize_path "$crate_dir/$arg")"
  migrators=$((migrators + 1))
  match_surface "$embedded"
  [ -n "$SURFACE_LABEL" ] || unlisted+=("$file:$lineno  embeds $embedded")
done <<< "$migrate_hits"

if [ "${#unlisted[@]}" -gt 0 ]; then
  fail_closed "${#unlisted[@]} sqlx::migrate! call(s) embed a directory no one-way surface covers:" \
    "${unlisted[@]}" \
    "" \
    "Every applied migration is a one-way door. Add the directory to SURFACES in" \
    "scripts/ci/check-merge-risk.sh, with a comment saying which backend applies it."
fi
if [ "$migrators" -eq 0 ]; then
  fail_closed "found no sqlx::migrate! call in HEAD's Rust sources." \
    "The migrations-directory premise verified nothing; refusing to report green."
fi

# --- the range -----------------------------------------------------------------
# The base as given: an explicit argument, else $GATE_BASE_REF, else
# origin/main — gate-base-ref.sh's priority order, without its HEAD~1 fallback.
# That fallback keeps a content gate reading the tip's files when the base is
# missing or is HEAD itself. A commit message is a property of the push range
# instead, and the tip's message alone is not that range: a burst push whose
# ONE-WAY line sits on an earlier commit passes when its push is judged, and a
# later schedule or dispatch run judging only its tip would fail it, red on main
# with no defect. So a base that names no range ends the run with nothing
# judged. Failing closed there instead would red a push over where its base
# went rather than what it adds: a before-sha a force-push left behind, or one
# deeper than the shallow CI checkout. A branch's pushes are judged from their
# fork point, which the fast gate fetches, and the squash that lands the branch
# on main names its own parent, which every checkout holds.
BASE_REF="${1:-${GATE_BASE_REF:-origin/main}}"
if ! git rev-parse --verify --quiet "${BASE_REF}^{commit}" >/dev/null 2>&1; then
  echo -e "${YELLOW}⚠️  merge-risk: base ${BASE_REF} is not a commit in this checkout, so the commits this push adds are unknown; nothing judged.${NC}"
  echo "   A force-push's old tip or a base beyond a shallow fetch does this; the squash"
  echo "   that lands the work on main names its own parent and is judged then."
  exit 0
fi
if ! BASE="$(git merge-base "$BASE_REF" HEAD 2>/dev/null)"; then
  fail_closed "no common ancestor between ${BASE_REF} and HEAD." \
    "Cannot establish which commits this push adds; refusing to report green."
fi
if [ "$BASE" = "$(git rev-parse 'HEAD^{commit}')" ]; then
  echo -e "${GREEN}✅ merge-risk: HEAD adds no commit over ${BASE_REF}; no push range, nothing to judge.${NC}"
  exit 0
fi
range="${BASE}..HEAD"
if ! commit_count="$(git rev-list --count "$range")"; then
  fail_closed "git rev-list over ${range} failed."
fi

if ! changed="$(git -c core.quotePath=false diff --name-only --no-renames "$BASE" HEAD)"; then
  fail_closed "git diff --name-only over ${range} failed."
fi

touched=()
while IFS= read -r path; do
  [ -n "$path" ] || continue
  match_surface "$path"
  [ -z "$SURFACE_LABEL" ] || touched+=("$path  ($SURFACE_LABEL)")
done <<< "$changed"

if ! patch="$(git -c core.quotePath=false diff --no-renames --no-color --no-ext-diff -U0 "$BASE" HEAD)"; then
  fail_closed "git diff over ${range} failed."
fi
# Added lines as "<file><TAB><line>", outside Markdown and comment lines. The
# header state matters: with -U0 an added line reading "++ x" prints as
# "+++ x", which only the position after an @@ tells apart from a file header.
if ! added="$(printf '%s\n' "$patch" | awk '
  /^diff --git / { in_hunk = 0; file = ""; next }
  !in_hunk && /^\+\+\+ / { file = substr($0, 5); sub(/^b\//, "", file); next }
  /^@@/ { in_hunk = 1; next }
  in_hunk && /^\+/ {
    if (file ~ /\.md$/) next
    line = substr($0, 2)
    probe = line
    sub(/^[ \t]+/, "", probe)
    if (probe ~ /^#/ || probe ~ /^\/\//) next
    print file "\t" line
  }
')"; then
  fail_closed "reading the added lines of ${range} failed."
fi
if replace_hits="$(printf '%s\n' "$added" | grep -E "$SECRET_REPLACE_RE")"; then
  while IFS=$'\t' read -r file line; do
    trimmed="${line#"${line%%[![:space:]]*}"}"
    touched+=("$file  (whole-set secret replace: ${trimmed:0:100})")
  done <<< "$replace_hits"
else
  rc=$?
  [ "$rc" -eq 1 ] || fail_closed "grep for whole-set secret replaces failed (exit $rc)."
fi

surface_total="${#SURFACES[@]}"
changed_count="$(printf '%s\n' "$changed" | grep -c . || true)"
premise="${surface_total} surfaces present, ${migrators} sqlx::migrate! call(s) covered"

if [ "${#touched[@]}" -eq 0 ]; then
  echo -e "${GREEN}✅ merge-risk: ${changed_count} changed file(s) in ${commit_count} commit(s) vs ${BASE_REF} ($(git rev-parse --short "$BASE")), no one-way surface touched (${premise}).${NC}"
  exit 0
fi

declared=()
while IFS= read -r sha; do
  [ -n "$sha" ] || continue
  if ! body="$(git log -1 --format=%B "$sha")"; then
    fail_closed "reading the message of ${sha} failed."
  fi
  while IFS= read -r msg_line; do
    if [[ "$msg_line" =~ $ONE_WAY_RE ]]; then
      declared+=("$(git rev-parse --short "$sha")  $msg_line")
    fi
  done <<< "$body"
done < <(git rev-list "$range")

echo "merge-risk: ${#touched[@]} one-way surface change(s) in ${commit_count} commit(s) vs ${BASE_REF} ($(git rev-parse --short "$BASE")):"
for t in "${touched[@]}"; do echo -e "  ${YELLOW}↯${NC}  $t"; done

if [ "${#declared[@]}" -gt 0 ]; then
  echo "  declared by:"
  for d in "${declared[@]}"; do echo "    $d"; done
  echo -e "${GREEN}✅ merge-risk: one-way change declared; reviewers read it in full (${premise}).${NC}"
  exit 0
fi

cat <<EOF

$(echo -e "${RED}❌ merge-risk: the range touches a one-way door and no commit in it says so.${NC}")

   Reverting a commit does not undo an applied migration, a destroyed secret or
   a delivered notification. Add a trailer at the start of a line in the commit
   message (the squash message on the feature path, the commit itself on a
   direct fix) naming why it cannot be rolled back:

     Merge-Risk: ONE-WAY · <why>

   e.g.  Merge-Risk: ONE-WAY · migration 20261012000001 adds a NOT NULL column

   For an unpushed commit: git commit --amend, add the line, re-run the gate.
EOF
exit 1
