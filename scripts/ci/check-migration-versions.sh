#!/usr/bin/env bash
# ABOUTME: Guards that no two migrations in a backend claim the same sqlx version prefix
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
