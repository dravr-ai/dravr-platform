#!/usr/bin/env bash
# ABOUTME: Guards that a migration already on main is never modified or removed —
# ABOUTME: sqlx refuses to boot when an applied migration's checksum changes.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists:
#   sqlx stores a SHA-384 of every migration it applies in _sqlx_migrations, and
#   on each boot re-hashes the file and compares. Editing a migration that has
#   already run somewhere makes every future boot fail with
#       migration <version> was previously applied but has been modified
#   and deleting one fails the same way with VersionMissing. Neither is a
#   compile error, neither shows up in a test on a FRESH database, and the tree
#   itself looks perfectly consistent — the break exists only relative to a
#   long-lived DB.
#
#   2026-09-08: 119040a11 edited 20260907000002 and 20260907000003 in place,
#   both already applied to the dev DB. Nothing caught it. The nightly
#   drift-check-coaches Cloud Run job died on it at 06:00 UTC the next morning
#   and every deploy would have too; it took two repair commits (3b08b7b79,
#   a6fbcae6a) to put the bytes back.
#
# Scope: the DIFF, not the tree — unlike check-migration-versions.sh, whose
#   collision is a property of a PAIR and so needs a whole-tree scan. "Was this
#   migration already published" is inherently a question about the base ref.
#   A migration added AND edited within one push is fine: nothing deployed
#   between the two commits, so nothing applied it.
#
# Keyed on the sqlx VERSION, not the filename, because that is what sqlx keys on.
#   A rename that keeps the version and the bytes is therefore not a violation;
#   one that changes the version reads as a removal, which is what it is to a DB
#   holding the old number.
#
# The one allowed modification is a RESTORE: content the file previously held in
#   git history. That is the only correct remedy once a checksum break is on main
#   (it returns the bytes the DB hashed), so a gate that blocked it would block
#   its own fix during an incident. It is not an exception list — nothing is
#   named, and the discriminator is provable from history.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

# One resolution rule for every diff-scoped gate: an explicit base, else
# $GATE_BASE_REF, else origin/main — and HEAD~1 whenever that base is missing or
# is HEAD itself (the shape actions/checkout leaves on a push to main).
# shellcheck source=scripts/ci/gate-base-ref.sh
. "$SCRIPT_DIR/gate-base-ref.sh"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'

if ! BASE_REF="$(resolve_gate_base_ref "${1:-}")"; then
  echo -e "${GREEN}✅ migration-immutability: no base ref to diff against; skipping.${NC}"
  exit 0
fi

# The published set is what existed at the FORK POINT, not at the base ref's tip.
# A feature branch that is merely behind main does not carry the migrations main
# gained after the fork, and comparing against main's tip would report every one
# of them as "removed" — failing every branch that had not rebased. The merge
# base is also provably an ancestor of HEAD, which is what makes "published here
# means applied somewhere" true.
if ! BASE="$(git merge-base "$BASE_REF" HEAD 2>/dev/null)"; then
  echo -e "${RED}❌ migration-immutability: no common ancestor between ${BASE_REF} and HEAD.${NC}"
  echo -e "   Cannot establish what was already published; refusing to report green."
  exit 1
fi

violations=0
restores=0
checked=0

# Every blob the given path held AT OR BEFORE the base ref, one hash per line.
#
# Walking from HEAD instead would make this vacuous: HEAD's own commit is part of
# its history, so the edited blob would always be found and every modification
# would read as a restore. The gate passed the real 119040a11 break that way
# before this was pinned by the fixture test.
historical_blobs() {
  local p="$1" c
  while read -r c; do
    [ -n "$c" ] || continue
    git rev-parse -q --verify "${c}:${p}" 2>/dev/null || true
  done < <(git log --format=%H "$BASE" -- "$p" 2>/dev/null || true)
}

for dir in migrations migrations_pg; do
  # The set to protect is what the BASE ref published, not what HEAD still has —
  # a deletion is invisible from HEAD's side.
  mapfile -t base_files < <(
    git ls-tree -r --name-only "$BASE" -- "$dir" 2>/dev/null | grep '\.sql$' | sort || true
  )
  [ "${#base_files[@]}" -gt 0 ] || continue

  for bf in "${base_files[@]}"; do
    base_name="$(basename "$bf")"
    if [[ ! "$base_name" =~ ^([0-9]+)_ ]]; then
      # A name sqlx cannot parse is its own boot failure; say so rather than
      # silently skipping it, so this scan never reports green on what it could
      # not read.
      echo -e "  ${RED}❌${NC} ${bf}  unparseable migration name (no <version>_ prefix)"
      violations=$((violations + 1))
      continue
    fi
    version="${BASH_REMATCH[1]}"
    checked=$((checked + 1))

    # Find the same VERSION in HEAD, under the same backend dir, whatever it is
    # now called.
    mapfile -t head_matches < <(
      git ls-tree -r --name-only HEAD -- "$dir" 2>/dev/null \
        | grep -E "/${version}_[^/]*\.sql$" | sort || true
    )

    if [ "${#head_matches[@]}" -eq 0 ]; then
      echo -e "  ${RED}❌${NC} ${bf}"
      echo -e "       version ${version} was published in ${BASE_REF} and is gone from HEAD"
      echo -e "       → a database that applied it fails to boot with VersionMissing"
      violations=$((violations + 1))
      continue
    fi

    head_path="${head_matches[0]}"
    base_blob="$(git rev-parse "${BASE}:${bf}")"
    head_blob="$(git rev-parse "HEAD:${head_path}")"

    [ "$base_blob" = "$head_blob" ] && continue

    # Changed. A return to content this file previously held is a restore — the
    # correct repair for a checksum break — and is allowed.
    if historical_blobs "$bf" | grep -qx "$head_blob" \
       || { [ "$head_path" != "$bf" ] && historical_blobs "$head_path" | grep -qx "$head_blob"; }; then
      echo -e "  ${YELLOW}↩${NC}  ${head_path}  restored to previously-committed content (allowed)"
      restores=$((restores + 1))
      continue
    fi

    echo -e "  ${RED}❌${NC} ${head_path}"
    if [ "$head_path" != "$bf" ]; then
      echo -e "       renamed from $(basename "$bf") AND modified"
    fi
    echo -e "       version ${version} was already published in ${BASE_REF}; its content changed"
    echo -e "       → every DB that applied it fails to boot: \"previously applied but has been modified\""
    violations=$((violations + 1))
  done
done

if [ "$violations" -gt 0 ]; then
  cat <<EOF

$(echo -e "${RED}❌ migration-immutability: ${violations} published migration(s) changed or removed.${NC}")

   A migration that has run against a long-lived DB is immutable — its SHA-384 is
   recorded in _sqlx_migrations and re-checked on every boot.

   To change what a published migration did, add a NEW migration that alters the
   result. Do not edit the old file.

   If you are REPAIRING a break already on main, restore the file to the exact
   bytes it had when it was applied — that is recognised here and allowed:
     git checkout <commit-before-the-edit> -- <migration file>
EOF
  exit 1
fi

# Report a success MARKER with what was actually verified: absence of a finding
# is indistinguishable from absence of a run, absence of this line is not.
if [ "$checked" -eq 0 ]; then
  # Not a vacuous pass: $BASE is a proven ancestor of HEAD, so "it published no
  # migrations" is a fact about history, not a scan that failed to run. The
  # unverifiable case — no common ancestor — already exited 1 above.
  echo -e "${GREEN}✅ migration-immutability: no migrations published at the fork point to protect.${NC}"
else
  msg="${checked} published migration(s) intact vs ${BASE_REF} ($(git rev-parse --short "$BASE"))"
  [ "$restores" -gt 0 ] && msg="${msg}, ${restores} restored"
  echo -e "${GREEN}✅ migration-immutability: ${msg}.${NC}"
fi
