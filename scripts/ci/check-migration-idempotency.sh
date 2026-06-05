#!/usr/bin/env bash
# ABOUTME: Guards that NEW/changed migrations use idempotent DDL so they survive
# ABOUTME: live-DB drift (the shared_insights deploy crash class) — no DB access needed.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists:
#   A long-lived database (dev/prod) drifts from the fresh migration tree (an
#   out-of-tree column, a squash that dropped a column the live DB still has).
#   A non-idempotent `ALTER TABLE x ADD COLUMN y` then fails with "already exists"
#   on the live DB and crashes container startup — even though it passed on a
#   fresh CI database. Forcing idempotent DDL on every NEW migration makes them
#   safe against ANY drift: create-where-missing, no-op where present.
#
# Scope: only migrations ADDED/CHANGED vs the base ref (historical ones are
#   grandfathered — they are already applied everywhere). Escape hatch for the
#   rare legitimate non-idempotent statement (e.g. a table-rebuild CREATE/DROP):
#   put `-- idempotency-ok: <reason>` on the same line.
set -euo pipefail

BASE_REF="${1:-origin/main}"
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

# Resolve the base ref robustly: CI checkouts may lack origin/main. Fall back to
# the parent commit so we always have a meaningful diff (or pass if neither).
if ! git rev-parse --verify --quiet "${BASE_REF}^{commit}" >/dev/null 2>&1; then
  if git rev-parse --verify --quiet "HEAD~1^{commit}" >/dev/null 2>&1; then
    BASE_REF="HEAD~1"
  else
    echo "✅ migration-idempotency: no base ref to diff against; skipping."
    exit 0
  fi
fi

# Changed/added migration files vs base (both backends).
mapfile -t FILES < <(
  git diff --name-only --diff-filter=AM "${BASE_REF}...HEAD" -- \
    'migrations/*.sql' 'migrations_pg/*.sql' 2>/dev/null | sort -u
)

if [ "${#FILES[@]}" -eq 0 ]; then
  echo "✅ migration-idempotency: no new/changed migrations to check."
  exit 0
fi

violations=0
report() { echo "  ❌ $1:$2  $3"; violations=$((violations + 1)); }

for f in "${FILES[@]}"; do
  [ -f "$f" ] || continue
  lineno=0
  while IFS= read -r raw; do
    lineno=$((lineno + 1))
    # strip trailing/leading space; skip blank + comment-only lines
    line="$(printf '%s' "$raw" | sed 's/^[[:space:]]*//')"
    case "$line" in ''|--*) continue ;; esac
    # per-statement escape hatch
    printf '%s' "$line" | grep -qiE -- '--[[:space:]]*idempotency-ok' && continue
    U="$(printf '%s' "$line" | tr '[:lower:]' '[:upper:]')"

    case "$U" in
      *"CREATE TABLE"*)
        printf '%s' "$U" | grep -q "CREATE TABLE IF NOT EXISTS" || \
          report "$f" "$lineno" "CREATE TABLE without IF NOT EXISTS" ;;
    esac
    case "$U" in
      *"CREATE INDEX"*|*"CREATE UNIQUE INDEX"*)
        printf '%s' "$U" | grep -q "INDEX IF NOT EXISTS" || \
          report "$f" "$lineno" "CREATE INDEX without IF NOT EXISTS" ;;
    esac
    case "$U" in
      *"ADD COLUMN"*)
        printf '%s' "$U" | grep -q "ADD COLUMN IF NOT EXISTS" || \
          report "$f" "$lineno" "ADD COLUMN without IF NOT EXISTS" ;;
    esac
    case "$U" in
      *"DROP TABLE"*)
        printf '%s' "$U" | grep -q "DROP TABLE IF EXISTS" || \
          report "$f" "$lineno" "DROP TABLE without IF EXISTS" ;;
    esac
    case "$U" in
      *"DROP COLUMN"*)
        printf '%s' "$U" | grep -q "DROP COLUMN IF EXISTS" || \
          report "$f" "$lineno" "DROP COLUMN without IF EXISTS" ;;
    esac
    case "$U" in
      *"DROP INDEX"*)
        printf '%s' "$U" | grep -q "DROP INDEX IF EXISTS" || \
          report "$f" "$lineno" "DROP INDEX without IF EXISTS" ;;
    esac
  done < "$f"
done

if [ "$violations" -gt 0 ]; then
  cat <<EOF

❌ migration-idempotency: ${violations} non-idempotent statement(s) in new migrations.

   Make DDL idempotent so it survives live-DB drift:
     ADD COLUMN  → ADD COLUMN IF NOT EXISTS
     CREATE TABLE/INDEX → ... IF NOT EXISTS
     DROP TABLE/COLUMN/INDEX → ... IF EXISTS
   For a deliberate exception (e.g. a table rebuild), append on the line:
     -- idempotency-ok: <reason>
EOF
  exit 1
fi

echo "✅ migration-idempotency: ${#FILES[@]} new migration(s) all idempotent."
exit 0
