#!/usr/bin/env bash
# ABOUTME: Pins check-backend-name-exposure.sh — catalogue and template catch cases, the clean case,
# ABOUTME: the identifier carve-outs (keys, API route, script id literal) and fail-closed inputs
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -uo pipefail

CHECK="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-backend-name-exposure.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILURES=0

pass() { echo "  ok   — $1"; }
fail() { echo "  FAIL — $1"; FAILURES=$((FAILURES + 1)); }

# The backend's name is assembled at run time so this file never carries it as
# a literal a future scan of scripts/ could trip on.
B="sci""otte"
CAP="Sci""otte"

scaffold() {
    rm -rf "$WORK/probe"
    mkdir -p "$WORK/probe/locales/en" "$WORK/probe/templates"
    printf '{ "providers": { "strava": "Strava" }, "app": { "title": "Dravr" } }\n' \
        > "$WORK/probe/locales/en/translation.json"
    printf '<!doctype html><html><body><h1>Sign in</h1></body></html>\n' \
        > "$WORK/probe/templates/page.html"
}

run_check() {
    ( cd "$WORK/probe" && "$CHECK" locales templates 2>&1 )
}

expect() {
    local label="$1" want_status="$2" needle="$3"
    OUT="$(run_check)"; STATUS=$?
    if [ "$STATUS" -eq "$want_status" ] && { [ -z "$needle" ] || echo "$OUT" | grep -qF -- "$needle"; }; then
        pass "$label"
    else
        fail "$label (status $STATUS, wanted $want_status)"
        echo "$OUT" | sed 's/^/        /'
    fi
}

echo "check-backend-name-exposure.sh"

# 1. Clean: nothing names the backend.
scaffold
expect "passes when no value or template names the backend" 0 "No catalogue value or served template names the backend"

# 2. Catch: a catalogue value naming the backend, the shape that shipped.
scaffold
printf '{ "providers": { "strava%s": "Strava — %s" } }\n' "$CAP" "$CAP" \
    > "$WORK/probe/locales/en/translation.json"
expect "fails on a catalogue value naming the backend, and names the key" 1 "providers.strava$CAP"

# 3. No false positive: a KEY carrying the name is an identifier, not copy.
scaffold
printf '{ "connect": { "%sBack": "Back" } }\n' "$B" > "$WORK/probe/locales/en/translation.json"
expect "a key carrying the name with a clean value passes" 0 "No catalogue value or served template names the backend"

# 4. Catch: the name in a template's visible text.
scaffold
printf '<html><body><p>Signing in through %s</p></body></html>\n' "$CAP" > "$WORK/probe/templates/page.html"
expect "fails on the name in a template's visible text" 1 "templates/page.html:1"

# 5. Catch: an HTML comment reaches view-source.
scaffold
printf '<html><!-- %s hosted login --><body></body></html>\n' "$CAP" > "$WORK/probe/templates/page.html"
expect "fails on the name in a template's HTML comment" 1 "TEMPLATE"

# 6. Catch: a script variable or comment is not an identifier the page needs.
scaffold
printf '<html><body><script>\n// open the %s fallback\nconst %sTarget = 1;\n</script></body></html>\n' "$B" "$B" \
    > "$WORK/probe/templates/page.html"
expect "fails on the name in a script comment or variable" 1 "templates/page.html:2"

# 7. No false positive: the API route and a quoted provider-id literal in script.
scaffold
{
    printf '<html><body><script>\n'
    printf "const d = await callApi('/api/providers/%s/login', {});\n" "$B"
    printf "if (provider === 'strava' || provider === '%s') {}\n" "$B"
    printf "const ids = [\"%s_garmin\"];\n" "$B"
    printf '</script></body></html>\n'
} > "$WORK/probe/templates/page.html"
expect "the API route and a quoted id literal in script pass" 0 "No catalogue value or served template names the backend"

# 8. Catch: a quoted id literal OUTSIDE a script can render (an attribute, a title).
scaffold
printf '<html><body><span title="%s">x</span></body></html>\n' "$B" > "$WORK/probe/templates/page.html"
expect "a quoted id outside a script is not exempt" 1 "TEMPLATE"

# 9. Fail closed: an empty catalogue verified nothing.
scaffold
printf '{}\n' > "$WORK/probe/locales/en/translation.json"
expect "an empty catalogue fails rather than passing" 1 "EMPTY CATALOGUE"

# 10. Fail closed: a path that does not exist.
OUT="$( cd "$WORK" && "$CHECK" no-such-dir 2>&1 )"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "MISSING no-such-dir"; then
    pass "a missing path fails rather than passing"
else
    fail "a missing path did not fail (status $STATUS)"
fi

# 11. Fail closed: a directory holding nothing to scan.
mkdir -p "$WORK/empty"
OUT="$( cd "$WORK" && "$CHECK" empty 2>&1 )"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "SCAN INCOMPLETE"; then
    pass "a directory with no catalogue or template fails"
else
    fail "an empty directory did not fail (status $STATUS)"
fi

if [ "$FAILURES" -ne 0 ]; then
    echo "check-backend-name-exposure.test.sh: $FAILURES failure(s)"
    exit 1
fi
echo "check-backend-name-exposure.test.sh: all cases pass"
