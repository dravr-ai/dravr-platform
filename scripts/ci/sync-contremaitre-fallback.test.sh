#!/usr/bin/env bash
# ABOUTME: Fixture test for sync-contremaitre-fallback.sh — proves it repairs each string drift shape
# ABOUTME: by creating the drift, against a synthetic canonical checkout, offline
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# A sync script only ever run on an already-synced tree proves nothing: it would
# report success while doing nothing at all. Each case here BREAKS the copy in
# one specific way and asserts both that `--check` reports it and that a plain
# run repairs it (carnet#593).
#
# Hermetic on purpose. Rather than reading the real dravr-contremaitre checkout
# — which needs network, private-repo auth and a cargo fetch, none of which the
# compile-free fast-gate job has — it builds a throwaway CARGO_HOME laid out the
# way cargo lays one out, and lets the script resolve it through the same code
# path production uses. No test-only escape hatch in the script.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
UNDER_TEST="$REPO_ROOT/scripts/ci/sync-contremaitre-fallback.sh"
CATALOGUE="$REPO_ROOT/packages/i18n/src/locales"

pass=0; fail=0
ok()  { pass=$((pass + 1)); echo "  ✓ $1"; }
bad() { fail=$((fail + 1)); echo "  ✗ $1"; }

# The catalogue is a git working copy, so restore through git. A snapshot of
# the wrong thing would silently make every case pass.
restore() {
    git -C "$REPO_ROOT" checkout -- "$CATALOGUE" 2>/dev/null || true
    git -C "$REPO_ROOT" clean -fdq -- "$CATALOGUE" 2>/dev/null || true
}
cleanup() { restore; [ -n "${FAKE_HOME:-}" ] && rm -rf "$FAKE_HOME"; }
trap cleanup EXIT

# `status --porcelain` rather than `diff --quiet`: an untracked file under the
# catalogue is work too, and `git clean` below would delete it.
if [ -n "$(git -C "$REPO_ROOT" status --porcelain -- "$CATALOGUE")" ]; then
    echo "REFUSING TO RUN: $CATALOGUE has uncommitted changes." >&2
    echo "This test rewrites it and restores it with git checkout and git clean," >&2
    echo "which would destroy that work. Commit it first." >&2
    exit 2
fi

# A CARGO_HOME shaped the way cargo shapes one, holding a strings/ tree we
# control. Seeded from the committed catalogue, so both sides start in sync and
# every case below is the ONLY difference.
PINNED="$(grep -h 'dravr-contremaitre = { git' "$REPO_ROOT"/crates/*/Cargo.toml "$REPO_ROOT"/Cargo.toml \
          | grep -oE 'rev = "[a-f0-9]{7,40}"' | grep -oE '[a-f0-9]{7,40}' | sort -u | head -1)"
FAKE_HOME="$(mktemp -d)"
CHECKOUT="$FAKE_HOME/git/checkouts/dravr-contremaitre-0000000000000000/${PINNED:0:7}"
CANON="$CHECKOUT/strings"
mkdir -p "$CANON"
for dir in "$CATALOGUE"/*/; do
    locale="$(basename "$dir")"
    cp "$dir/translation.json" "$CANON/$locale.json"
done
export CARGO_HOME="$FAKE_HOME"
LOCALE_COUNT="$(ls "$CANON" | wc -l | tr -d ' ')"

echo "sync-contremaitre-fallback.sh  (synthetic checkout, pinned ${PINNED:0:8}, ${LOCALE_COUNT} locales)"

# --- case 0: the harness itself is honest ------------------------------------
# If the synthetic checkout did not resolve, every case below would "pass" by
# doing nothing. Assert the script sees it, and compared every locale, before
# trusting any result.
out="$("$UNDER_TEST" --check 2>&1)"
if [ $? = 0 ] && printf '%s' "$out" | grep -qF "${LOCALE_COUNT} locale(s) byte-identical"; then
    ok "harness: the script resolves the synthetic checkout and compares all ${LOCALE_COUNT} locales"
else
    bad "harness: script cannot see the synthetic checkout — every case below is vacuous"
    printf '%s\n' "$out" | sed 's/^/      /'
    echo "$pass passed, $fail failed"; exit 1
fi

# --- case 1: upstream changes a string (the pin bump's everyday shape) --------
printf '\n' >> "$CANON/en.json"
if "$UNDER_TEST" --check >/dev/null 2>&1; then
    bad "upstream edit: --check passed while en differed by one byte"
else
    ok "upstream edit: --check reports a one-byte difference"
fi
"$UNDER_TEST" >/dev/null 2>&1
if cmp -s "$CANON/en.json" "$CATALOGUE/en/translation.json"; then
    ok "upstream edit: a plain run copies it in byte-identical"
else
    bad "upstream edit: the platform copy still differs after sync"
fi
git -C "$REPO_ROOT" show "HEAD:packages/i18n/src/locales/en/translation.json" > "$CANON/en.json"
restore

# --- case 2: a string edited in the platform copy first -----------------------
# The shape the byte-identity check exists for: a hand edit that never reached
# contremaitre, which the next pin bump would silently revert.
printf ' ' >> "$CATALOGUE/fr/translation.json"
rc=0; "$UNDER_TEST" --check >/dev/null 2>&1 || rc=$?
if [ "$rc" = "1" ]; then
    ok "hand edit: --check reports it with exit 1"
else
    bad "hand edit: --check exited $rc on a platform-side edit"
fi
"$UNDER_TEST" >/dev/null 2>&1
if git -C "$REPO_ROOT" diff --quiet -- "$CATALOGUE"; then
    ok "hand edit: a plain run restores the contremaitre bytes"
else
    bad "hand edit: the platform edit survived the sync"
fi
restore

# --- case 3: a locale on one side only is refused, not created ----------------
cp "$CANON/en.json" "$CANON/it.json"
rc=0; "$UNDER_TEST" >/dev/null 2>&1 || rc=$?
if [ "$rc" = "2" ] && [ ! -e "$CATALOGUE/it" ]; then
    ok "extra upstream locale: refused with exit 2, nothing created"
else
    bad "extra upstream locale: exit $rc, and $CATALOGUE/it exists: $([ -e "$CATALOGUE/it" ] && echo yes || echo no)"
fi
rm -f "$CANON/it.json"
mv "$CANON/pt.json" "$FAKE_HOME/held-pt.json"
rc=0; "$UNDER_TEST" --check >/dev/null 2>&1 || rc=$?
if [ "$rc" = "2" ]; then
    ok "missing upstream locale: --check refuses with exit 2 rather than comparing four"
else
    bad "missing upstream locale: --check exited $rc"
fi
mv "$FAKE_HOME/held-pt.json" "$CANON/pt.json"
restore

# --- case 4: an upstream rev with no strings at all fails closed --------------
mv "$CANON" "$FAKE_HOME/held-strings"
mkdir -p "$CANON"
rc=0; "$UNDER_TEST" --check >/dev/null 2>&1 || rc=$?
if [ "$rc" = "2" ]; then
    ok "empty strings/: --check refuses with exit 2 (zero locales compared is not a pass)"
else
    bad "empty strings/: --check exited $rc"
fi
rmdir "$CANON"; mv "$FAKE_HOME/held-strings" "$CANON"
restore

# --- case 5: idempotence -----------------------------------------------------
# Not idempotent means the bump lane commits a diff on every hourly no-op.
"$UNDER_TEST" >/dev/null 2>&1
"$UNDER_TEST" >/dev/null 2>&1
if [ -z "$(git -C "$REPO_ROOT" status --porcelain -- "$CATALOGUE")" ]; then
    ok "idempotent: two consecutive runs leave the catalogue unchanged"
else
    bad "idempotent: a run on an in-sync tree produced a diff"
    git -C "$REPO_ROOT" status --porcelain -- "$CATALOGUE" | sed 's/^/      /'
fi
restore

# --- case 6: consumers disagreeing on the rev is refused, not guessed --------
skew_manifest="$(grep -rl 'dravr-contremaitre = { git' "$REPO_ROOT"/crates "$REPO_ROOT"/Cargo.toml --include=Cargo.toml | head -1)"
sed -i.bak -E 's|(dravr-contremaitre = \{ git = "[^"]+", rev = ")[a-f0-9]{7,40}"|\1deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"|' "$skew_manifest"
rm -f "${skew_manifest}.bak"
rc=0; "$UNDER_TEST" --check >/dev/null 2>&1 || rc=$?
if [ "$rc" = "2" ]; then
    ok "rev skew: refuses with exit 2 rather than picking a rev"
else
    bad "rev skew: expected exit 2, got $rc (it guessed a rev instead of refusing)"
fi
git -C "$REPO_ROOT" checkout -- "$skew_manifest" 2>/dev/null || true

echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
