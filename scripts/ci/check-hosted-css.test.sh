#!/usr/bin/env bash
# ABOUTME: Pins check-hosted-css.sh — the clean case, a stale block per generated sheet, a palette creeping back, a token under its floor
# ABOUTME: Copies the gate, the generators, the tokens and the sheets into a throwaway tree, so it never touches the checkout
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILURES=0

pass() { echo "  ok   — $1"; }
fail() { echo "  FAIL — $1"; FAILURES=$((FAILURES + 1)); }

if ! command -v bun >/dev/null 2>&1; then
    echo "check-hosted-css.test.sh: bun is not on PATH; the gate under test regenerates with bun."
    exit 1
fi

# Everything the gate reads, at the paths it reads them from.
PATHS=(
    scripts/ci/check-hosted-css.sh
    packages/shared-constants/scripts
    packages/shared-constants/src
    crates/pierre-core/src/hosted_page.css
    frontend/public/brand/mark-ink-96.png
    frontend/src/index.css
    frontend/src/boreal-tokens.generated.css
    frontend-mobile/global.css
    frontend-mobile/boreal-tokens.generated.css
)

scaffold() {
    rm -rf "$WORK/repo"
    mkdir -p "$WORK/repo"
    local path
    for path in "${PATHS[@]}"; do
        mkdir -p "$WORK/repo/$(dirname "$path")"
        cp -R "$ROOT/$path" "$WORK/repo/$path"
    done
    # The hosted templates the gate discovers, wherever they live under crates/.
    ( cd "$ROOT" && grep -rl --include='*.html' '<html' crates/*/templates crates/*/src ) | while IFS= read -r template; do
        mkdir -p "$WORK/repo/$(dirname "$template")"
        cp "$ROOT/$template" "$WORK/repo/$template"
    done
}

run_check() {
    "$WORK/repo/scripts/ci/check-hosted-css.sh" 2>&1
}

# `sed -i` differs between GNU and BSD; rewrite through a temp file instead.
replace_in() {
    local file="$1" from="$2" to="$3"
    FROM="$from" TO="$to" perl -pe 's/\Q$ENV{FROM}\E/$ENV{TO}/' "$file" > "$file.tmp" && mv "$file.tmp" "$file"
}

echo "check-hosted-css.sh"

# 1. Clean: the tree as committed passes every check.
scaffold
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -eq 0 ] && echo "$OUT" | grep -q "both committed blocks are the generator's output"; then
    pass "passes on the committed sheets"
else
    fail "false positive on the committed tree (status $STATUS): $OUT"
fi

# 2. Catch: a hand edit to the web's generated block.
scaffold
replace_in "$WORK/repo/frontend/src/boreal-tokens.generated.css" "--color-outline: 176 182 175;" "--color-outline: 138 147 137;"
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "frontend/src/boreal-tokens.generated.css is stale"; then
    pass "fails on a drifted web block, and names it"
else
    fail "did not catch the drifted web block (status $STATUS)"
fi

# 3. Catch: a hand edit to the phone's generated block.
scaffold
replace_in "$WORK/repo/frontend-mobile/boreal-tokens.generated.css" "--color-primary: 37 95 77;" "--color-primary: 0 36 26;"
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "frontend-mobile/boreal-tokens.generated.css is stale"; then
    pass "fails on a drifted mobile block, and names it"
else
    fail "did not catch the drifted mobile block (status $STATUS)"
fi

# 4. Catch: a token moved in the source and the blocks were not regenerated.
scaffold
replace_in "$WORK/repo/packages/shared-constants/src/design-system.ts" "  surfaceContainerLowest: '#0b0e0b'," "  surfaceContainerLowest: '#191c19',"
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "frontend/src/boreal-tokens.generated.css is stale" \
    && echo "$OUT" | grep -q "frontend-mobile/boreal-tokens.generated.css is stale"; then
    pass "fails when the tokens move under both committed blocks"
else
    fail "did not catch blocks left behind by a token change (status $STATUS)"
fi

# 5. Catch: a token below its WCAG floor — the generator refuses, and the gate with it.
scaffold
replace_in "$WORK/repo/packages/shared-constants/src/design-system.ts" "  outline: '#b0b6af'," "  outline: '#8a9389',"
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "dark outline on surface-container-highest"; then
    pass "fails when a token would ship under its floor, and names the pairing"
else
    fail "did not refuse a token under its floor (status $STATUS)"
fi

# 6. Catch: the hand copy creeping back beside the import.
scaffold
printf ':root {\n  --color-primary: 0 36 26;\n}\n' >> "$WORK/repo/frontend/src/index.css"
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "frontend/src/index.css: declares custom properties of its own"; then
    pass "fails on a variable declared beside the generated block"
else
    fail "did not catch a palette declared in index.css (status $STATUS)"
fi

# 7. Catch: a client stylesheet that stops importing its block.
scaffold
replace_in "$WORK/repo/frontend-mobile/global.css" "@import './boreal-tokens.generated.css';" ""
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "frontend-mobile/global.css: does not import its generated block"; then
    pass "fails on a stylesheet that no longer imports its block"
else
    fail "did not catch a missing import (status $STATUS)"
fi

# 8. Catch: the hosted sheet, the gate's original case, still goes stale.
scaffold
replace_in "$WORK/repo/crates/pierre-core/src/hosted_page.css" "  --color-card: 255 255 255;" "  --color-card: 250 250 250;"
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "hosted_page.css is stale"; then
    pass "fails on a drifted hosted sheet"
else
    fail "did not catch the drifted hosted sheet (status $STATUS)"
fi

if [ "$FAILURES" -gt 0 ]; then
    echo "check-hosted-css.test.sh: $FAILURES case(s) failed"
    exit 1
fi
echo "check-hosted-css.test.sh: all cases passed"
