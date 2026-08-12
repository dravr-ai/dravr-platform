#!/usr/bin/env bash
# ABOUTME: Ratchets Rust source file size — a file may not exceed the ceiling, and a
# ABOUTME: file already over it may not grow by a single line. No refactor required.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists:
#   Large files do not arrive large; they arrive 40 lines at a time. A plain
#   ceiling is unlandable in a codebase that already has files over it, so it
#   never gets added and the growth never stops. A ratchet is landable today:
#
#     limit(file) = base_lines > MAX ? base_lines : MAX
#
#   A file at or under the ceiling must stay under it. A file already over the
#   ceiling is grandfathered at exactly its current size and may shrink or hold,
#   but never grow. Files shrink over time and nothing has to be refactored to
#   turn the gate on.
#
# Scope: production Rust source only (crates/*/src/**.rs). Test files are a
#   different shape — a long table of cases is not the same debt as a long
#   module — and are deliberately not gated here.
#
# Escape hatch: some files are data, not complexity (a five-locale string table
#   grows by five lines per key, by design). Put a line
#
#     // file-size-ok: <reason>
#
#   anywhere in the file to exempt it. Same idiom as `-- idempotency-ok:` in
#   check-migration-idempotency.sh: the reason lives with the file it excuses,
#   not in a central list of offenders that nobody ever prunes.
set -euo pipefail

MAX_LINES="${FILE_SIZE_MAX_LINES:-1200}"
BASE_REF="${1:-origin/main}"
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

# Resolve the base ref robustly: CI checkouts may lack origin/main. Fall back to
# the parent commit so we always have a meaningful diff (or pass if neither).
if ! git rev-parse --verify --quiet "${BASE_REF}^{commit}" >/dev/null 2>&1; then
  if git rev-parse --verify --quiet "HEAD~1^{commit}" >/dev/null 2>&1; then
    BASE_REF="HEAD~1"
  else
    echo "✅ file-sizes: no base ref to diff against; skipping."
    exit 0
  fi
fi

# Only files this change added or modified. An untouched file can never fail
# this gate — you are responsible for the size of what you edit, not for
# inheriting the repository.
CHANGED="$(git diff --name-only --diff-filter=AM "${BASE_REF}...HEAD" 2>/dev/null || true)"

if [ -z "$CHANGED" ]; then
  echo "✅ file-sizes: no changed files to check."
  exit 0
fi

violations=0
checked=0

while IFS= read -r f; do
  [ -n "$f" ] || continue
  # crates/<crate>/src/**.rs only.
  case "$f" in
    crates/*/src/*.rs) ;;
    *) continue ;;
  esac
  [ -f "$f" ] || continue

  # Per-file escape hatch, read from the working tree.
  if grep -qE '^[[:space:]]*//[[:space:]]*file-size-ok:' "$f"; then
    continue
  fi

  checked=$((checked + 1))
  candidate="$(wc -l <"$f" | tr -d ' ')"

  # Lines at the base ref. A file that did not exist there is new, and a new
  # file gets no grandfathering — it must come in under the ceiling.
  if base_blob="$(git show "${BASE_REF}:${f}" 2>/dev/null)"; then
    base="$(printf '%s\n' "$base_blob" | wc -l | tr -d ' ')"
  else
    base=0
  fi

  if [ "$base" -gt "$MAX_LINES" ]; then
    limit="$base"
    grandfathered=1
  else
    limit="$MAX_LINES"
    grandfathered=0
  fi

  if [ "$candidate" -gt "$limit" ]; then
    violations=$((violations + 1))
    if [ "$grandfathered" -eq 1 ]; then
      echo "  ❌ $f: $candidate lines, was $base — already over the ${MAX_LINES}-line"
      echo "     ceiling, so it is frozen at its current size and may not grow."
    else
      echo "  ❌ $f: $candidate lines, ceiling is ${MAX_LINES}."
    fi
  fi
done <<<"$CHANGED"

if [ "$violations" -gt 0 ]; then
  echo ""
  echo "❌ file-sizes: $violations file(s) over budget."
  echo "   Split the file, or — if its size is data rather than complexity —"
  echo "   add a '// file-size-ok: <reason>' line explaining why."
  exit 1
fi

echo "✅ file-sizes: $checked changed source file(s) within budget (ceiling ${MAX_LINES})."
exit 0
