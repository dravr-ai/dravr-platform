#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Fails when a client calls t('a.b') with a key the shared en catalogue does not carry
# ABOUTME: Locale-to-locale parity cannot see this — a key removed from all five resolves in none

# `locale-corpus.test.ts` compares the five locales to each other, so a key
# deleted from all five stays "in parity" while every caller of it renders the
# raw key string to the athlete. That is how `app.weeklyReport` shipped as the
# literal heading of the mobile group insights panel in every language: the
# commit that moved it to `groups.weeklyReport` repointed the sibling call in
# the same file and missed this one.
#
# This gate closes the other direction: every literal key a client asks for
# must exist in the English catalogue (the key set is identical across
# locales, which the corpus test already pins).
#
# Scope and deliberate exclusions:
#   - `fallbackKey: 'a.b'` literals count as call sites: describeApiError
#     translates them itself, so a typo there renders the raw key too.
#   - Only single/double-quoted literals. `t(`groups.${id}`)` is dynamic and
#     unresolvable statically; those are counted and reported, never failed on.
#   - Comments are stripped before scanning, so prose naming an old key is not
#     a failure (it is still wrong, but it is not a broken render).
#   - Test and mock files are skipped; they legitimately assert on absent keys.

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"

# With no arguments this is the CI invocation: both client trees, from the repo
# root. With arguments it scans exactly those directories in the caller's own
# working directory, which is what the test harness drives.
if [ "$#" -eq 0 ]; then
    cd "$PROJECT_ROOT"
    SOURCE_DIRS=(frontend/src frontend-mobile/src)
else
    SOURCE_DIRS=("$@")
fi

CATALOGUE="${I18N_CATALOGUE:-packages/i18n/src/locales/en/translation.json}"

echo -e "${BLUE}==== i18n Key Resolution (client -> shared catalogue) ====${NC}"

if [ ! -f "$CATALOGUE" ]; then
    echo -e "${RED}❌ Catalogue not found: $CATALOGUE${NC}"
    exit 1
fi

# The scan asserts its own premise: a run that resolved no catalogue keys, or
# found no call sites at all, has verified nothing and fails rather than
# reporting a clean pass.
if ! OUT="$(python3 - "$CATALOGUE" "${SOURCE_DIRS[@]}" <<'PY' 2>&1
import json, os, re, sys

catalogue_path, source_dirs = sys.argv[1], sys.argv[2:]

with open(catalogue_path, encoding="utf8") as handle:
    catalogue = json.load(handle)


def leaf_keys(node, prefix=""):
    if not isinstance(node, dict):
        return {prefix}
    found = set()
    for key, value in node.items():
        found |= leaf_keys(value, key if not prefix else f"{prefix}.{key}")
    return found


keys = leaf_keys(catalogue)

BLOCK_COMMENT = re.compile(r"/\*.*?\*/", re.S)
LINE_COMMENT = re.compile(r"^\s*(//|\*).*$", re.M)
# `fallbackKey: 'a.b'` is the key describeApiError hands to `t` when a failed
# call carries nothing better; it renders exactly as a `t()` call does.
CALL = re.compile(r"(?:\bt\(\s*|\bfallbackKey:\s*)(['\"])([A-Za-z0-9_]+(?:\.[A-Za-z0-9_]+)+)\1")
# \x60 is the backtick: spelled as an escape because this python sits inside
# a $( ... ) the shell parses first, and bash 3.2 stops at a literal one.
DYNAMIC = re.compile(r"\bt\(\s*\x60")

sites, dynamic_count, scanned = {}, 0, 0
for root in source_dirs:
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in ("node_modules", "__tests__", "__mocks__")]
        for name in filenames:
            if not name.endswith((".ts", ".tsx")) or ".test." in name or ".spec." in name:
                continue
            path = os.path.join(dirpath, name)
            with open(path, encoding="utf8") as handle:
                raw = handle.read()
            scanned += 1
            dynamic_count += len(DYNAMIC.findall(raw))
            code = LINE_COMMENT.sub("", BLOCK_COMMENT.sub("", raw))
            for match in CALL.finditer(code):
                line = code[: match.start()].count("\n") + 1
                sites.setdefault(match.group(2), []).append(f"{path}:{line}")

unresolved = {k: v for k, v in sites.items() if k not in keys}

print(f"catalogue keys: {len(keys)}")
print(f"files scanned: {scanned}")
print(f"literal keys in code: {len(sites)}")
print(f"dynamic t(template) calls (not checkable): {dynamic_count}")
for key in sorted(unresolved):
    for site in unresolved[key]:
        print(f"UNRESOLVED {key} {site}")
if not keys or not sites or not scanned:
    print("SCAN INCOMPLETE")
    raise SystemExit(1)
if unresolved:
    raise SystemExit(1)
print("I18N KEYS RESOLVED")
PY
)"; then
    echo "$OUT" | sed 's/^/    /'
    if echo "$OUT" | grep -q "SCAN INCOMPLETE"; then
        echo -e "${RED}❌ Scan verified nothing — no catalogue keys, no sources, or no call sites.${NC}"
        echo -e "${RED}Fix the scan or its inputs; an empty scan is never a pass.${NC}"
    else
        echo -e "${RED}❌ A client calls t() with a key the catalogue does not carry.${NC}"
        echo -e "${RED}It renders as the raw key string, in every language.${NC}"
        echo -e "${RED}Point the caller at the key that exists, or add the key to all five locales.${NC}"
    fi
    exit 1
fi

# Grep for the success marker, never for the absence of a finding: a crashed
# scan produces no findings either.
if ! echo "$OUT" | grep -q "I18N KEYS RESOLVED"; then
    echo "$OUT" | sed 's/^/    /'
    echo -e "${RED}❌ Scan did not report success — treating as failure.${NC}"
    exit 1
fi

echo "$OUT" | grep -v "I18N KEYS RESOLVED" | sed 's/^/    /'
echo -e "${GREEN}✅ Every literal t() key resolves in the shared catalogue${NC}"
exit 0
