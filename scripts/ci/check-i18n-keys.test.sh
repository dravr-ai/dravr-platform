#!/usr/bin/env bash
# ABOUTME: Pins check-i18n-keys.sh — both catch cases, the clean case, three no-false-positive cases, fail-closed
# ABOUTME: Builds a throwaway catalogue and source tree, so it runs in under a second with no deps
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -uo pipefail

CHECK="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-i18n-keys.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILURES=0

pass() { echo "  ok   — $1"; }
fail() { echo "  FAIL — $1"; FAILURES=$((FAILURES + 1)); }

scaffold() {
    rm -rf "$WORK/probe"
    mkdir -p "$WORK/probe/src"
    cat > "$WORK/probe/catalogue.json" <<'EOF'
{ "groups": { "weeklyReport": "Weekly report" }, "app": { "title": "Dravr" } }
EOF
}

# `t()` sites are written through a variable so this file's own literals are
# not scanned as call sites when the gate runs over the repo.
run_check() {
    ( cd "$WORK/probe" && I18N_CATALOGUE="catalogue.json" "$CHECK" src 2>&1 )
}

echo "check-i18n-keys.sh"

# 1. Catch: a key the catalogue does not carry.
scaffold
printf 'export const H = () => <Text>{t(%s)}</Text>;\n' "'app.weeklyReport'" > "$WORK/probe/src/Broken.tsx"
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "UNRESOLVED app.weeklyReport"; then
    pass "fails on a key the catalogue does not carry, and names it"
else
    fail "did not catch the unresolved key (status $STATUS)"
fi

# 2. Clean: every key resolves.
scaffold
printf 'export const H = () => <Text>{t(%s)}</Text>;\n' "'groups.weeklyReport'" > "$WORK/probe/src/Good.tsx"
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -eq 0 ]; then
    pass "passes when every key resolves"
else
    fail "false positive on a resolving key (status $STATUS): $OUT"
fi

# 3. No false positive: a comment naming a retired key is prose, not a render.
scaffold
{
    printf '// the hint used to read t(%s) before the move\n' "'app.weeklyReport'"
    printf 'export const H = () => <Text>{t(%s)}</Text>;\n' "'groups.weeklyReport'"
} > "$WORK/probe/src/Commented.tsx"
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -eq 0 ]; then
    pass "ignores a retired key named only in a comment"
else
    fail "flagged a commented-out key (status $STATUS)"
fi

# 4. No false positive: a dynamic key cannot be resolved statically.
scaffold
{
    printf 'export const H = ({ id }: { id: string }) => <Text>{t(`groups.${id}`)}</Text>;\n'
    printf 'export const J = () => <Text>{t(%s)}</Text>;\n' "'app.title'"
} > "$WORK/probe/src/Dynamic.tsx"
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -eq 0 ] && echo "$OUT" | grep -q "dynamic t"; then
    pass "counts a dynamic key without failing on it"
else
    fail "mishandled a dynamic key (status $STATUS)"
fi

# 5. Catch: a classifier fallback key is translated like a t() call.
scaffold
printf 'export const m = (err: unknown) => describeApiError(err, { t, fallbackKey: %s });\n' "'app.weeklyReport'" > "$WORK/probe/src/Fallback.tsx"
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "UNRESOLVED app.weeklyReport"; then
    pass "fails on a describeApiError fallback key the catalogue does not carry"
else
    fail "did not catch the unresolved fallback key (status $STATUS)"
fi

# 6. Fail closed: a scan that found no call sites has verified nothing.
scaffold
printf 'export const H = () => <Text>nothing translated here</Text>;\n' > "$WORK/probe/src/Empty.tsx"
OUT="$(run_check)"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "SCAN INCOMPLETE"; then
    pass "fails closed when the scan resolved no call sites"
else
    fail "reported a pass on a scan that verified nothing (status $STATUS)"
fi

echo
if [ "$FAILURES" -gt 0 ]; then
    echo "❌ $FAILURES case(s) failed"
    exit 1
fi
echo "✅ all cases passed"
exit 0
