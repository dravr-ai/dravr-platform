#!/usr/bin/env bash
# ABOUTME: Guards migration version collisions, edits to landed migrations, and catalogue id reuse
# ABOUTME: Compile-free — catches at authoring time what otherwise only fails when a server boots.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists:
#   sqlx keys a migration on its leading numeric version, not its filename. Two
#   files sharing that number fail at RUNTIME with
#       UNIQUE constraint failed: _sqlx_migrations.version
#   and the server never finishes booting. Nothing else catches it: duplicates
#   compile, pass clippy, and clear every other architectural gate. The first
#   lane to notice is one that starts a server, and what it reports is
#   "Server health check failed after 90000ms" — the cause only in the log.
#
#   2026-08-27: 308dae6e7 (llm_usage cache columns) and 9f1a93516 (intervals.icu
#   calendar ledger) both took 20260827000001, an hour apart. Each was correct
#   alone; the collision existed only once both were on main. As long as the
#   number is hand-picked from today's date, concurrent work on main will keep
#   producing it.
#
# Scope: the WHOLE tree, not the diff. A collision is a property of a pair, and
#   today's pair were both new relative to their own base — a diff-scoped check
#   passes each push and still lets main break. Whole-tree is also cheap: this
#   parses filenames and touches no database.
#
# Uniqueness only, never ordering: sqlx accepts an out-of-order version and
#   rejects only a duplicate (verified 2026-08-27 — a dev DB holding
#   20260827000001 applied 20260826000008 cleanly). A check demanding monotonic
#   versions would reject valid migrations.
#
# The same version appearing once in migrations/ AND once in migrations_pg/ is
#   correct — they are per-backend mirrors of one migration. Each directory is
#   therefore checked on its own.
#
# ── Check 2: a landed migration is frozen ────────────────────────────────────
#   sqlx stores a SHA-384 of each migration's bytes and refuses to boot when the
#   file no longer hashes to it:
#       migration <version> was previously applied but has been modified
#   Cloud Run keeps serving the last good revision, so nothing looks down while
#   every deploy fails.
#
#   This has now happened twice. c2e3177 edited an applied billing migration and
#   killed every revision until 4f298fcbb restored it and superseded it with
#   20260429000001. 119040a11 renumbered catalogue ids inside an applied
#   20260907000003 and killed every revision for ~13h on 2026-09-08.
#
#   "Applied" is database state this check cannot read. "Exists on origin/main"
#   is the proxy, and it is the right one HERE because deploy fires on push:
#   anything on main may already be applied somewhere. Byte-for-byte, so a
#   comment-only edit fails too — sqlx hashes the whole file, and 3e280bee1
#   rewrote comments in two migrations for exactly that reason.
#
#   The remedy is never an exception list. It is to supersede: leave the landed
#   file alone and add a new migration that carries the change forward, which is
#   what 4c1cd67e5 did.
#
# ── Check 3: no catalogue id is claimed twice ────────────────────────────────
#   Seed migrations insert tool_catalog rows under hand-picked 'tc-NNN' ids with
#   ON CONFLICT DO NOTHING. When two migrations pick the same id the second
#   insert is skipped SILENTLY, and a rename that then DELETEs the old row loses
#   it. 20260907000002 claimed tc-109..tc-134 while the already-applied
#   20260907000003 held tc-109..tc-113: five agent tools were dropped on the
#   deployed database and five physiology tools on every fresh one, in mirror
#   image, and only a cron-only SQLite shard noticed, 24h later.
#
#   Comment lines are stripped before scanning: 20260907000002 documents its own
#   id choice in prose, and a scan that reads that prose reports a collision that
#   is not there.
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'

violations=0
scanned_total=0

for dir in migrations migrations_pg; do
  [ -d "$dir" ] || continue

  # Basenames only; sqlx reads <version>_<description>.sql.
  mapfile -t names < <(find "$dir" -maxdepth 1 -name '*.sql' -exec basename {} \; | sort)
  scanned=0
  unparsed=0
  versions_file="$(mktemp)"

  for n in "${names[@]}"; do
    if [[ "$n" =~ ^([0-9]+)_ ]]; then
      printf '%s\n' "${BASH_REMATCH[1]}" >> "$versions_file"
      scanned=$((scanned + 1))
    else
      # A name sqlx cannot parse is its own boot failure, so say so rather than
      # skipping quietly — a filter that drops what it cannot read passes
      # vacuously.
      echo -e "  ${YELLOW}⚠️  $dir/$n — no leading numeric version; sqlx cannot key this${NC}"
      unparsed=$((unparsed + 1))
    fi
  done

  scanned_total=$((scanned_total + scanned))

  while read -r dup; do
    [ -z "$dup" ] && continue
    echo -e "  ${RED}❌ $dir: version $dup claimed by more than one migration:${NC}"
    find "$dir" -maxdepth 1 -name "${dup}_*.sql" -exec basename {} \; | sort | sed 's/^/       /'
    violations=$((violations + 1))
  done < <(sort "$versions_file" | uniq -d)

  rm -f "$versions_file"
  echo "  scanned $dir: $scanned migration(s), $unparsed unparseable"
done

# A check that scanned nothing must not report success — that is the vacuous
# pass this class of guard is prone to.
if [ "$scanned_total" -eq 0 ]; then
  echo -e "${RED}❌ migration-versions: scanned 0 migrations — the check found nothing to verify.${NC}"
  exit 1
fi

if [ "$violations" -gt 0 ]; then
  echo -e "${RED}❌ migration-versions: $violations duplicate version(s).${NC}"
  echo "   Renumber ONE of each pair — the one NOT yet applied anywhere."
  echo "   An already-applied migration has its version recorded in that database"
  echo "   against its file's checksum; renumbering it orphans the applied row and"
  echo "   re-runs DDL that already exists. Deploy is gated behind CI, so a commit"
  echo "   whose CI broke never applied its migration: that is the free one to move."
  echo "   Rename the file in BOTH migrations/ and migrations_pg/."
  exit 1
fi

echo -e "${GREEN}✅ No duplicate sqlx migration versions${NC}"

# ─── Check 2: a migration that already landed on the base ref is frozen ───────
# Diff-scoped, unlike check 1: modification is a property of the change, not of
# the tree. MIGRATION_FREEZE_BASE lets the test suite point this at a fixture.
# One resolution rule for every diff-scoped gate, and it is the rule that keeps
# a push to main armed: actions/checkout leaves origin/main == HEAD there, which
# would make this compare HEAD against itself and read nothing.
# shellcheck source=scripts/ci/gate-base-ref.sh
. "$(dirname "${BASH_SOURCE[0]}")/gate-base-ref.sh"

if ! BASE="$(resolve_gate_base_ref "${1:-}")"; then
  # Only a root commit reaches here: there is genuinely no earlier tree, so
  # nothing can have "already landed". Check 1 above still ran over the whole
  # tree, which is the part that does not need a baseline.
  echo -e "${YELLOW}⚠️  migration-freeze / catalogue-ids: no base commit to diff against; skipped.${NC}"
  exit 0
fi

# Diff the MERGE BASE against the working tree, not against HEAD: that catches
# an edit still sitting uncommitted as well as one already committed, and does
# not read a peer's newer commits on the base ref as though this change reverted
# them.
MERGE_BASE="$(git merge-base "$BASE" HEAD)"

mapfile -t modified < <(
  git diff --name-only --diff-filter=M "$MERGE_BASE" -- migrations migrations_pg 2>/dev/null \
    | grep -E '\.sql$' || true
)

if [ "${#modified[@]}" -gt 0 ]; then
  echo -e "${RED}❌ migration-freeze: ${#modified[@]} migration(s) already on $BASE were modified:${NC}"
  printf '       %s\n' "${modified[@]}"
  echo
  echo "   sqlx stores a SHA-384 of each migration's bytes. Changing one after it"
  echo "   has been applied makes every later boot abort with"
  echo "     migration <version> was previously applied but has been modified"
  echo "   and Cloud Run keeps serving the last good revision, so nothing looks"
  echo "   down while every deploy fails. This is byte-for-byte: a comment-only"
  echo "   edit breaks it too."
  echo
  echo "   Supersede instead of editing. Leave the landed file untouched and add"
  echo "   a NEW migration carrying the change forward, idempotent on the value"
  echo "   it seeds rather than on an id, so it is safe on a database that"
  echo "   already has the row."
  exit 1
fi

echo -e "${GREEN}✅ No landed migration was modified${NC}"

# ─── Check 3: no tool_catalog id is claimed by two migrations ─────────────────
id_violations=0
standing=0
ids_seen_total=0
declare -A dir_ids=()

# Every migration this change adds or edits, against the same base check 2 uses.
# Tracked changes plus untracked additions: a migration written but not yet
# `git add`ed is exactly the one being authored, and it is the one whose id
# choice this check exists to catch.
mapfile -t changed_migrations < <(
  {
    git diff --name-only "$MERGE_BASE" -- migrations migrations_pg 2>/dev/null || true
    git ls-files --others --exclude-standard -- migrations migrations_pg 2>/dev/null || true
  } | grep -E '\.sql$' | sort -u || true
)

for dir in migrations migrations_pg; do
  [ -d "$dir" ] || continue

  pairs_file="$(mktemp)"
  for f in "$dir"/*.sql; do
    [ -e "$f" ] || continue
    # Drop whole-line comments before scanning: these migrations discuss their
    # own id choices in prose, and reading that prose invents collisions.
    # `|| true`: most migrations seed no catalogue row, and a grep that matches
    # nothing exits 1, which pipefail would turn into a failed run.
    base="$(basename "$f")"
    sed -E 's/^[[:space:]]*--.*$//' "$f" \
      | grep -oE "'tc-[0-9]+'" \
      | tr -d "'" \
      | sort -u \
      | awk -v f="$base" '{ print $0, f }' >> "$pairs_file" || true
  done

  count=$(wc -l < "$pairs_file" | tr -d ' ')
  ids_seen_total=$((ids_seen_total + count))
  dir_ids["$dir"]="$(cut -d' ' -f1 < "$pairs_file" | sort -u | tr '\n' ' ')"

  while read -r dup; do
    [ -z "$dup" ] && continue
    mapfile -t claimants < <(grep -E "^$dup " "$pairs_file" | awk '{print $2}')

    # Gate the diff, report the standing stock. The tc-109..113 pair below is
    # the worked example: 20260907000002 and 20260907000003 are BOTH applied and
    # therefore both frozen, so the collision cannot be renumbered away — it was
    # repaired forward by 20260909000001 instead. Failing on it would leave a
    # gate that can never go green, so a collision only fails the run when this
    # change is party to it.
    touched=0
    for c in "${claimants[@]}"; do
      if printf '%s\n' "${changed_migrations[@]}" | grep -qx "$dir/$c"; then
        touched=1
      fi
    done

    if [ "$touched" -eq 1 ]; then
      echo -e "  ${RED}❌ $dir: catalogue id $dup claimed by more than one migration:${NC}"
      printf '       %s\n' "${claimants[@]}"
      id_violations=$((id_violations + 1))
    else
      echo -e "  ${YELLOW}⚠️  $dir: standing collision on $dup (${claimants[*]})${NC}"
      standing=$((standing + 1))
    fi
  done < <(cut -d' ' -f1 < "$pairs_file" | sort | uniq -d)

  rm -f "$pairs_file"
  echo "  scanned $dir: $count catalogue id claim(s)"
done

# The two backends mirror one migration set, so a divergent id claim means one
# backend seeds a row the other does not.
if [ "${dir_ids[migrations]:-}" != "${dir_ids[migrations_pg]:-}" ]; then
  echo -e "  ${RED}❌ migrations/ and migrations_pg/ claim different catalogue ids${NC}"
  echo "       migrations/    : ${dir_ids[migrations]:-<none>}"
  echo "       migrations_pg/ : ${dir_ids[migrations_pg]:-<none>}"
  id_violations=$((id_violations + 1))
fi

# Fail closed only where zero is impossible. A tree with no tool_catalog seeds
# legitimately claims no ids; a tree that seeds the table and yields none means
# the extraction broke, which is the vacuous pass this class of guard rots into.
# `|| true` again: no match is exit 1, and pipefail would end the run here.
seeding_files=$( { grep -rlE 'INSERT INTO tool_catalog' migrations migrations_pg 2>/dev/null || true; } | wc -l | tr -d ' ')
if [ "$seeding_files" -gt 0 ] && [ "$ids_seen_total" -eq 0 ]; then
  echo -e "${RED}❌ catalogue-ids: $seeding_files migration(s) seed tool_catalog but the scan extracted 0 ids.${NC}"
  echo "   The id pattern stopped matching — fix the scan, do not trust this pass."
  exit 1
fi

if [ "$id_violations" -gt 0 ]; then
  echo -e "${RED}❌ catalogue-ids: $id_violations collision(s).${NC}"
  echo "   tool_catalog seeds insert under a hand-picked id with ON CONFLICT DO"
  echo "   NOTHING, so a reused id is skipped SILENTLY. When the migration then"
  echo "   DELETEs the row it was replacing, that tool is simply gone: no error,"
  echo "   no failed migration, and guardian reads a missing row as 'no override"
  echo "   applies', so a tool an operator disabled comes back on."
  echo "   Give the migration that has NOT landed yet a free id — the highest"
  echo "   claimed above, plus one."
  exit 1
fi

if [ "$standing" -gt 0 ]; then
  echo -e "${YELLOW}   $standing standing collision(s) above — pre-existing, and not blessed by this pass.${NC}"
  echo "   Each is a pair of already-applied migrations, so neither can be renumbered."
  echo "   The repair is forward: a new migration re-seeding whatever the collision"
  echo "   dropped, idempotent on tool_name. 20260909000001 is the worked example."
fi

echo -e "${GREEN}✅ This change adds no catalogue id collision, and both backends claim the same set${NC}"
