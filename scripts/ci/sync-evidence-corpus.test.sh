#!/usr/bin/env bash
# ABOUTME: Fixture test for sync-evidence-corpus.sh — proves it repairs each drift shape
# ABOUTME: by creating the drift, against a synthetic canonical corpus, offline
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# A sync script only ever run on an already-synced tree proves nothing: it would
# report success while doing nothing at all. Each case here BREAKS the mirror in
# one specific way and asserts both that `--check` reports it and that a plain
# run repairs it (carnet#325).
#
# Hermetic on purpose. Rather than reading the real dravr-contremaitre checkout
# — which needs network, private-repo auth and a cargo fetch, none of which the
# compile-free fast-gate job has — it builds a throwaway CARGO_HOME laid out the
# way cargo lays one out, and lets the script resolve it through the same code
# path production uses. No test-only escape hatch in the script.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
UNDER_TEST="$REPO_ROOT/scripts/ci/sync-evidence-corpus.sh"
DEST="$REPO_ROOT/crates/pierre-evals/fixtures/sports_science"
TABLE="$REPO_ROOT/crates/pierre-services/src/claim_verification.rs"

pass=0; fail=0
ok()  { pass=$((pass + 1)); echo "  ✓ $1"; }
bad() { fail=$((fail + 1)); echo "  ✗ $1"; }

# The real fixtures are a git working copy, so restore through git. A snapshot
# of the wrong thing would silently make every case pass.
restore() {
    git -C "$REPO_ROOT" checkout -- "$DEST" "$TABLE" 2>/dev/null || true
    git -C "$REPO_ROOT" checkout -- "$REPO_ROOT/crates" 2>/dev/null || true
    git -C "$REPO_ROOT" clean -fdq -- "$DEST" 2>/dev/null || true
}
cleanup() { restore; [ -n "${FAKE_HOME:-}" ] && rm -rf "$FAKE_HOME"; }
trap cleanup EXIT

if ! git -C "$REPO_ROOT" diff --quiet -- "$DEST" "$TABLE"; then
    echo "REFUSING TO RUN: $DEST or $TABLE has uncommitted changes." >&2
    echo "This test rewrites both and restores them with git checkout, which" >&2
    echo "would destroy that work. Commit or stash it first." >&2
    exit 2
fi

# A CARGO_HOME shaped the way cargo shapes one, holding a canonical corpus we
# control. Seeded from the committed fixtures, so the tree starts in sync and
# every case below is the ONLY difference.
PINNED="$(grep -h 'dravr-contremaitre = { git' "$REPO_ROOT"/crates/*/Cargo.toml \
          | grep -oE 'rev = "[a-f0-9]{7,40}"' | grep -oE '[a-f0-9]{7,40}' | sort -u | head -1)"
FAKE_HOME="$(mktemp -d)"
CANON="$FAKE_HOME/git/checkouts/dravr-contremaitre-0000000000000000/${PINNED:0:7}/evidence/sports_science"
mkdir -p "$CANON"
rsync -a --exclude='README.md' "$DEST/" "$CANON/"
export CARGO_HOME="$FAKE_HOME"

echo "sync-evidence-corpus.sh  (synthetic corpus, pinned ${PINNED:0:8})"

# --- case 0: the harness itself is honest ------------------------------------
# If the synthetic corpus did not resolve, every case below would "pass" by
# doing nothing. Assert the script can see it before trusting any result.
if "$UNDER_TEST" --check >/dev/null 2>&1; then
    ok "harness: the script resolves the synthetic corpus and starts in sync"
else
    bad "harness: script cannot see the synthetic corpus — every case below is vacuous"
    echo "$pass passed, $fail failed"; exit 1
fi

# --- case 1: upstream adds a proposition (the carnet#325 shape) --------------
new_upstream="$CANON/training_prescription/zzz-upstream-addition.md"
cat > "$new_upstream" <<'MD'
---
id: doi:10.0000/upstream-addition
category: training_prescription
strength: moderate
citation: Upstream Addition 2026
---

A proposition that exists upstream and has not yet reached the in-tree fallback.
MD
if "$UNDER_TEST" --check >/dev/null 2>&1; then
    bad "upstream addition: --check passed while the fallback was missing a proposition"
else
    ok "upstream addition: --check reports it (this is what would red main)"
fi
"$UNDER_TEST" >/dev/null 2>&1
if [ -f "$DEST/training_prescription/zzz-upstream-addition.md" ]; then
    ok "upstream addition: a plain run copies it in"
else
    bad "upstream addition: sync did not copy it in"
fi
if grep -q 'zzz-upstream-addition.md' "$TABLE"; then
    ok "upstream addition: the table gains its entry too"
else
    bad "upstream addition: file copied but EMBEDDED_PROPOSITIONS never learned about it"
fi
rm -f "$new_upstream"; restore

# --- case 2: upstream retracts a proposition ---------------------------------
# The direction a copy-only sync misses. The check is bidirectional, so a file
# kept after upstream withdrew it reds main as loudly as one never copied —
# which is why the rsync carries --delete.
victim_rel="$(cd "$CANON" && find . -name '*.md' | head -1 | sed 's|^\./||')"
mv "$CANON/$victim_rel" "$FAKE_HOME/held.md"
if "$UNDER_TEST" --check >/dev/null 2>&1; then
    bad "upstream retraction: --check passed while a withdrawn proposition lingered"
else
    ok "upstream retraction: --check reports it"
fi
"$UNDER_TEST" >/dev/null 2>&1
if [ -f "$DEST/$victim_rel" ]; then
    bad "upstream retraction: sync left $victim_rel behind (is --delete still on the rsync?)"
else
    ok "upstream retraction: a plain run removes it"
fi
mv "$FAKE_HOME/held.md" "$CANON/$victim_rel"; restore

# --- case 3: files right, Rust table stale -----------------------------------
# The table is hand-editable, so it can drift from the directory it describes
# even with every file present — leaving a proposition unreachable from Rust
# while every file-level check passes.
before="$(grep -c 'include_str!' "$TABLE")"
python3 - "$TABLE" <<'PY'
import io, sys
p = sys.argv[1]
s = io.open(p, encoding="utf-8").read()
start = s.index("const EMBEDDED_PROPOSITIONS: &[(&str, &str)] = &[")
end = s.index("\n];\n", start)
cut = s.rindex("    (", start, end)
io.open(p, "w", encoding="utf-8").write(s[:cut] + s[end + 1:])
PY
if [ "$(grep -c 'include_str!' "$TABLE")" -ge "$before" ]; then
    bad "stale table: fixture setup failed to shorten the table"
else
    "$UNDER_TEST" >/dev/null 2>&1
    if [ "$(grep -c 'include_str!' "$TABLE")" = "$before" ]; then
        ok "stale table: a plain run regenerates the dropped entry"
    else
        bad "stale table: still short after sync"
    fi
fi
restore

# --- case 4: idempotence -----------------------------------------------------
# Not idempotent means the bump lane commits a diff on every hourly no-op.
"$UNDER_TEST" >/dev/null 2>&1
"$UNDER_TEST" >/dev/null 2>&1
if git -C "$REPO_ROOT" diff --quiet -- "$DEST" "$TABLE"; then
    ok "idempotent: two consecutive runs leave the tree unchanged"
else
    bad "idempotent: a run on an in-sync tree produced a diff"
    git -C "$REPO_ROOT" diff --stat -- "$DEST" "$TABLE" | sed 's/^/      /'
fi
restore

# --- case 5: consumers disagreeing on the rev is refused, not guessed --------
# Syncing against "whichever rev I read first" would swap one drift for another,
# silently. The recurring pierre-chat-pipeline skew this lane's own comments
# describe is exactly that state, so the branch is reachable.
skew_manifest="$(grep -rl 'dravr-contremaitre = { git' "$REPO_ROOT"/crates --include=Cargo.toml | head -1)"
sed -i.bak -E 's|(dravr-contremaitre = \{ git = "[^"]+", rev = ")[a-f0-9]{7,40}"|\1deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"|' "$skew_manifest"
rm -f "${skew_manifest}.bak"
"$UNDER_TEST" --check >/dev/null 2>&1
rc=$?
if [ "$rc" = "2" ]; then
    ok "rev skew: refuses with exit 2 rather than picking a rev"
else
    bad "rev skew: expected exit 2, got $rc (it guessed a rev instead of refusing)"
fi
git -C "$REPO_ROOT" checkout -- "$skew_manifest" 2>/dev/null || true

echo
echo "$pass passed, $fail failed"
[ "$fail" -eq 0 ]
