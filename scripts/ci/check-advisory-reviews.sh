#!/usr/bin/env bash
# ABOUTME: Makes deny.toml advisory suppressions expire — every ignored RUSTSEC id
# ABOUTME: needs a "Next Review" date, and a date in the past fails the build.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists:
#   A suppressed advisory is a decision made once against a snapshot of the
#   dependency graph. The graph moves; the decision does not. Our deny.toml
#   carried a thorough write-up per advisory — severity, risk assessment,
#   mitigation — and a "Next Review" date that nothing enforced, so it sat seven
#   months expired while two of the three suppressions quietly went stale (an
#   affected version drifted 0.8.5 -> 0.8.6, a cited source path moved).
#
#   An unenforced date is documentation of an intention, not a control. This
#   turns it into one: every id inside the `ignore = [...]` block must carry a
#   `# Next Review: YYYY-MM-DD` in its comment block, and the build fails once
#   that date passes.
#
#   Renewing is deliberately not free — re-check the graph, update the stated
#   facts, then move the date. Bumping the date alone is the failure mode this
#   is meant to prevent, and review will see it in the diff.
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

DENY_FILE="${1:-deny.toml}"
TODAY="$(date -u +%Y-%m-%d)"

if [ ! -f "$DENY_FILE" ]; then
  echo "✅ advisory-reviews: no $DENY_FILE; nothing to check."
  exit 0
fi

violations=0
checked=0

# Walk the ignore = [ ... ] block. A "Next Review:" line covers only the ids in
# its OWN contiguous comment block: a blank line ends the block and clears the
# date. Ids that share a block (the paired quick-xml advisories) share its date;
# an id further down does NOT inherit an earlier block's date, or a new
# suppression could be slipped under an old date and ride it for free.
pending_date=""
in_ignore=0

while IFS= read -r line; do
  case "$line" in
    *"ignore"*"= ["*) in_ignore=1; continue ;;
  esac
  [ "$in_ignore" -eq 1 ] || continue
  case "$line" in
    "]"*) break ;;
  esac

  # Blank line = end of this comment block; its date stops applying. Tested with
  # a shell string test, not grep: an empty line gives grep zero lines of input,
  # so `grep -qE '^[[:space:]]*$'` never matches it.
  if [ -z "$(printf '%s' "$line" | tr -d '[:space:]')" ]; then
    pending_date=""
    continue
  fi

  # Capture a review date as we pass it.
  if printf '%s' "$line" | grep -qE '^[[:space:]]*#[[:space:]]*Next Review:'; then
    pending_date="$(printf '%s' "$line" \
      | sed -nE 's/.*Next Review:[[:space:]]*([0-9]{4}-[0-9]{2}-[0-9]{2}).*/\1/p')"
    if [ -z "$pending_date" ]; then
      echo "  ❌ malformed 'Next Review:' (want YYYY-MM-DD): $(printf '%s' "$line" | sed 's/^[[:space:]]*//')"
      violations=$((violations + 1))
    fi
    continue
  fi

  # An advisory id line consumes the pending date.
  id="$(printf '%s' "$line" | sed -nE 's/^[[:space:]]*"(RUSTSEC-[0-9]{4}-[0-9]{4})".*/\1/p')"
  [ -n "$id" ] || continue

  checked=$((checked + 1))
  if [ -z "$pending_date" ]; then
    echo "  ❌ $id has no '# Next Review: YYYY-MM-DD' in its comment block."
    violations=$((violations + 1))
    continue
  fi

  # String comparison is correct for zero-padded ISO-8601 dates.
  if [ "$pending_date" \< "$TODAY" ]; then
    echo "  ❌ $id review expired on $pending_date (today $TODAY)."
    violations=$((violations + 1))
  fi
done <"$DENY_FILE"

if [ "$violations" -gt 0 ]; then
  echo ""
  echo "❌ advisory-reviews: $violations suppression(s) need attention in $DENY_FILE."
  echo "   Re-check the dependency graph (cargo tree -i <crate>), update the stated"
  echo "   facts, then set a new 'Next Review:' date — or drop the ignore entirely"
  echo "   if upstream has shipped the fix."
  exit 1
fi

echo "✅ advisory-reviews: $checked suppression(s) reviewed and unexpired."
exit 0
