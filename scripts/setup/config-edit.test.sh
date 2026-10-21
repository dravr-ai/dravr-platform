#!/usr/bin/env bash
# ABOUTME: Pins config-edit.sh: one backup, atomic line-anchored edits that keep comments, a ledger, a guarded undo
# ABOUTME: Runs in a throwaway directory with no network and no real .envrc
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=config-edit.sh
. "$HERE/config-edit.sh"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILURES=0
pass() { echo "  ok   — $1"; }
fail() { echo "  FAIL — $1"; FAILURES=$((FAILURES + 1)); }

echo "config-edit.sh"
ENVRC="$WORK/.envrc"
printf '# keep me\nexport OTHER="1"\nexport PIERRE_JWT_TOKEN="old"\n# trailing comment\n' >"$ENVRC"
chmod 600 "$ENVRC"
ORIGINAL="$(cat "$ENVRC")"

cfg_set_export "$ENVRC" PIERRE_JWT_TOKEN 'new&value|with/specials'
if grep -qxF 'export PIERRE_JWT_TOKEN="new&value|with/specials"' "$ENVRC" \
    && grep -qxF '# keep me' "$ENVRC" && grep -qxF '# trailing comment' "$ENVRC" \
    && [ "$(grep -c 'PIERRE_JWT_TOKEN' "$ENVRC")" -eq 1 ]; then
    pass "replaces the one line, keeps every other line and comment, takes the value literally"
else
    fail "edit changed more than the line: $(cat "$ENVRC")"
fi

if [ "$(stat -c %a "$WORK/.setup-state/.envrc.orig" 2>/dev/null || stat -f %Lp "$WORK/.setup-state/.envrc.orig")" = 600 ] \
    && [ "$(cat "$WORK/.setup-state/.envrc.orig")" = "$ORIGINAL" ]; then
    pass "backs up the original once, owner-only"
else
    fail "backup missing, wrong or readable by others"
fi

cfg_set_export "$ENVRC" ADDED 'x'
if [ "$(cat "$WORK/.setup-state/.envrc.orig")" = "$ORIGINAL" ] && grep -qxF 'export ADDED="x"' "$ENVRC"; then
    pass "a second edit appends a missing key and keeps the first backup"
else
    fail "second edit overwrote the backup or did not append"
fi

if [ "$(grep -c $'\tset ' "$WORK/.setup-state/ledger.tsv")" -eq 2 ]; then
    pass "records each edit in the ledger"
else
    fail "ledger: $(cat "$WORK/.setup-state/ledger.tsv")"
fi

echo 'export HAND="edited"' >>"$ENVRC"
cfg_undo "$ENVRC" 2>/dev/null
status=$?
if [ "$status" -eq 2 ] && grep -qxF 'export HAND="edited"' "$ENVRC"; then
    pass "refuses to undo over a hand edit made since"
else
    fail "undo overwrote a hand edit (status $status)"
fi

sed -i.bak '$d' "$ENVRC" && rm -f "$ENVRC.bak"
if cfg_undo "$ENVRC" && [ "$(cat "$ENVRC")" = "$ORIGINAL" ]; then
    pass "restores the original when the file is what the last edit left"
else
    fail "undo did not restore the original: $(cat "$ENVRC")"
fi

if [ "$FAILURES" -eq 0 ]; then echo "✅ all cases passed"; else echo "❌ $FAILURES case(s) failed"; exit 1; fi
