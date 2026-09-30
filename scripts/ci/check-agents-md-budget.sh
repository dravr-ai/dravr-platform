#!/usr/bin/env bash
# ABOUTME: Fails when AGENTS.md outgrows its byte budget, so implementer context cannot regrow silently
# ABOUTME: Raising the budget is a reviewed one-line diff here; removing text is the retro step's job
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# AGENTS.md (which .claude/CLAUDE.md links to) loads into every session. It
# doubled in September 2026 to 76.8 KB, one incident paragraph at a time, and
# nothing ever removed one. carnet#660 cut it to what an implementing agent
# needs to act and moved the coding standards to docs/coding-standards.md,
# which only the review-standards pass reads. This keeps the cut: a rule that
# judges written code belongs in the standards file, a procedure in a skill.
#
# Usage: check-agents-md-budget.sh [path] — AGENTS_MD_BUDGET overrides the budget.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FILE="${1:-$ROOT/AGENTS.md}"
BUDGET="${AGENTS_MD_BUDGET:-40000}"

# Fail closed: a missing or empty file measured nothing.
if [ ! -s "$FILE" ]; then
    echo "❌ $FILE is missing or empty — nothing was measured"
    exit 1
fi

size=$(wc -c < "$FILE" | tr -d ' ')
if [ "$size" -gt "$BUDGET" ]; then
    echo "❌ $(basename "$FILE") is $size bytes, over its $BUDGET-byte budget by $((size - BUDGET))"
    echo "   A rule that judges written code goes in docs/coding-standards.md; a task procedure"
    echo "   goes in a skill. Raise AGENTS_MD_BUDGET's default only as a deliberate, reviewed change."
    exit 1
fi
echo "✅ $(basename "$FILE") is $size of $BUDGET bytes"
