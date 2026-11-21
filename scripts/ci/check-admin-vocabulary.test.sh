#!/usr/bin/env bash
# ABOUTME: Pins check-admin-vocabulary.sh — an admin called operator fails, the clean case passes,
# ABOUTME: an allowlisted span passes only on its own path and line, and fail-closed inputs
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -uo pipefail

CHECK="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-admin-vocabulary.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILURES=0

pass() { echo "  ok   — $1"; }
fail() { echo "  FAIL — $1"; FAILURES=$((FAILURES + 1)); }

# The word is assembled at run time so this file never carries it as a literal
# a future scan of scripts/ could trip on.
W="oper""ator"

# A throwaway repository: the gate reads tracked files through git grep.
scaffold() {
    rm -rf "$WORK/probe"
    mkdir -p "$WORK/probe/crates/pierre-database/src" "$WORK/probe/crates/other/src"
    git -C "$WORK/probe" init -q
    printf '/// An admin approves the login.\npub fn approve() {}\n' \
        > "$WORK/probe/crates/other/src/lib.rs"
}

run_check() {
    git -C "$WORK/probe" add -A
    ( cd "$WORK/probe" && "$CHECK" crates 2>&1 )
}

expect() {
    local label="$1" want_status="$2" needle="$3"
    OUT="$(run_check)"; STATUS=$?
    if [ "$STATUS" -eq "$want_status" ] && { [ -z "$needle" ] || echo "$OUT" | grep -qF -- "$needle"; }; then
        pass "$label"
    else
        fail "$label (status $STATUS, want $want_status)"
        echo "        ${OUT//$'\n'/$'\n'        }"
    fi
}

# 1. Clean: no occurrence at all.
scaffold
expect "a tree that says admin passes" 0 "No \"$W\" names an admin"

# 2. Catch: a doc comment calling an admin an operator.
scaffold
printf '/// An %s approves the login.\n' "$W" >> "$WORK/probe/crates/other/src/lib.rs"
expect "an admin called $W fails" 1 "crates/other/src/lib.rs:3"

# 3. Catch: any case, inside an identifier.
scaffold
printf 'pub struct Sole%s;\n' "Oper""ator" >> "$WORK/probe/crates/other/src/lib.rs"
expect "a capitalised identifier fails" 1 "crates/other/src/lib.rs:3"

# 4. Allow: the column name stays, anywhere.
scaffold
printf 'pub const COL: &str = "%s_user_id";\n' "$W" >> "$WORK/probe/crates/other/src/lib.rs"
expect "the admin_tokens column name passes" 0 ""

# 5. The span is removed, not the line: the column plus an admin called operator fails.
scaffold
printf '/// %s_user_id: the %s who approved\n' "$W" "$W" >> "$WORK/probe/crates/other/src/lib.rs"
expect "an allowlisted span does not excuse the rest of its line" 1 "crates/other/src/lib.rs:3"

# 5b. The column name is not a log field: a tracing key naming an admin fails,
# while the column in SQL (`= $1`) still passes.
scaffold
printf '    info!(%s_user_id = %%admin, "approved");\n' "$W" >> "$WORK/probe/crates/other/src/lib.rs"
expect "a log field named after the column fails" 1 "crates/other/src/lib.rs:3"
scaffold
printf 'const Q: &str = "WHERE %s_user_id = %s1";\n' "$W" "\$" >> "$WORK/probe/crates/other/src/lib.rs"
expect "the column in a SQL filter passes" 0 ""

# 6. A path-scoped entry holds only on its path.
scaffold
printf '/// like is the case-folding match %s.\n' "$W" > "$WORK/probe/crates/pierre-database/src/lib.rs"
expect "the SQL match $W passes in pierre-database" 0 ""
printf '/// like is the case-folding match %s.\n' "$W" >> "$WORK/probe/crates/other/src/lib.rs"
expect "the same span elsewhere fails" 1 "crates/other/src/lib.rs:3"

# 7. Fail closed: a path that does not exist.
OUT="$( cd "$WORK/probe" && "$CHECK" no-such-dir 2>&1 )"; STATUS=$?
if [ "$STATUS" -ne 0 ] && echo "$OUT" | grep -q "MISSING no-such-dir"; then
    pass "a missing path fails rather than passing"
else
    fail "a missing path did not fail (status $STATUS)"
fi

# 8. Fail closed: outside a git repository nothing can be read.
mkdir -p "$WORK/plain/crates"
OUT="$( cd "$WORK/plain" && GIT_CEILING_DIRECTORIES="$WORK" "$CHECK" crates 2>&1 )"; STATUS=$?
if [ "$STATUS" -ne 0 ]; then
    pass "a tree outside git fails rather than passing"
else
    fail "a tree outside git passed"
fi

if [ "$FAILURES" -ne 0 ]; then
    echo "check-admin-vocabulary.test.sh: $FAILURES failure(s)"
    exit 1
fi
echo "check-admin-vocabulary.test.sh: all cases pass"
