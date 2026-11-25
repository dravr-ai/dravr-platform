#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Fails a Rust test that asserts a number is absent with a bare `!text.contains("<digits>")`
# ABOUTME: Serialized replies carry timestamps and ids whose digits match by chance; use mentions_number

# carnet#1821. A reply that carries a clock (2026-11-13T03:24:31.708009814Z)
# holds digit runs the test never wrote, so `!text.contains("31.7")` fails
# whenever the clock lands there. It reddened 33.75, 120 and 187 in one week.
# The fix is `pierre_test_support::numbers::mentions_number(text, "31.7")`,
# which matches a whole number only, or a field-by-field comparison of the
# parsed JSON.
#
# What is refused: a `.contains("<number>")` whose literal is a bare number
# (`42`, `-80`, `12.5`) in a negated position: after a unary `!`, in
# `assert!(!x.contains(..))`, `.all(|m| !m.contains(..))`, `!xs.iter().any(..)`.
# Positive checks are not scanned: they can only pass for the wrong reason, and
# the sweep converted those that read serialized replies.
#
# The one escape: a line `// number-substring-ok: <reason>` on the line before
# (or the same line), for a needle that is a token to be found inside another
# one (an identity inside a hash or a URL), where substring IS the question.
#
# Usage:
#   check-test-number-substring.sh           the CI invocation: every tracked .rs file
#   check-test-number-substring.sh PATH...   exactly these files or directories (the self-test's form)

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"

if [ "$#" -eq 0 ]; then
    cd "$PROJECT_ROOT"
    set -- .
fi

for root in "$@"; do
    if [ ! -e "$root" ]; then
        echo -e "${RED}MISSING $root${NC}"
        exit 1
    fi
done

echo -e "${BLUE}==== Number substring assertions (carnet#1821) ====${NC}"

# `git ls-files` so build output never counts; a path outside a repository
# fails closed rather than scanning nothing.
# The list goes through a file: a tracked tree's paths overflow one
# environment string on Linux ("Argument list too long").
LIST="$(mktemp)"
trap 'rm -f "$LIST"' EXIT
if ! git ls-files -z -- "$@" | tr '\0' '\n' | { grep -E '\.rs$' || true; } > "$LIST"; then
    echo -e "${RED}git ls-files failed; nothing was verified${NC}"
    exit 1
fi

# Fail closed on a scan that verified nothing: no tracked .rs file under the
# given paths means the gate read no test at all.
if [ ! -s "$LIST" ]; then
    echo -e "${RED}No tracked .rs file under: $*; nothing was verified${NC}"
    exit 1
fi

if ! OUT="$(python3 - "$LIST" <<'PY' 2>&1
import os, re, sys

NEEDLE = re.compile(r'contains\(\s*"(-?[0-9][0-9.,_]*)"\s*\)')
# A unary `!` (not `!=`, not a macro bang) in the statement before the call.
UNARY_NOT = re.compile(r'(?<![\w!])!(?!=)')
BOUNDARY = re.compile(r';|&&|\|\||\{|\}')
MARKER = "number-substring-ok:"

findings = []
for path in open(sys.argv[1], encoding="utf-8").read().split("\n"):
    if not path:
        continue
    try:
        with open(path, encoding="utf-8") as handle:
            text = handle.read()
    except (OSError, UnicodeDecodeError):
        continue
    lines = text.split("\n")
    for hit in NEEDLE.finditer(text):
        before = text[max(0, hit.start() - 240):hit.start()]
        statement = BOUNDARY.split(before)[-1]
        if not UNARY_NOT.search(statement):
            continue
        number_line = text.count("\n", 0, hit.start())
        if lines[number_line].lstrip().startswith("//"):
            continue
        window = lines[max(0, number_line - 3):number_line + 1]
        if any(MARKER in line for line in window):
            continue
        findings.append(f"{path}:{number_line + 1}: {lines[number_line].strip()[:140]}")

for finding in findings:
    print(finding)
sys.exit(1 if findings else 0)
PY
)"; then
    echo -e "${RED}A test asserts a number is absent by substring (carnet#1821):${NC}"
    echo "$OUT"
    echo ""
    echo "A timestamp's nanosecond fraction or an id's digits can match a bare number by"
    echo "chance, so the test fails though the code is right. Use"
    echo "pierre_test_support::numbers::mentions_number(text, \"<number>\") (whole numbers"
    echo "only) or compare the parsed JSON's fields. When substring really is the"
    echo "question (an identity must not sit inside a hash or a URL), say so on the"
    echo "line before:  // number-substring-ok: <reason>"
    exit 1
fi

echo -e "${GREEN}No test asserts a number is absent by bare substring${NC}"
