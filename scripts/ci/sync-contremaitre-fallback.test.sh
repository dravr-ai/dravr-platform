#!/usr/bin/env bash
# ABOUTME: Fixture test for sync-contremaitre-fallback.sh — proves it repairs each drift shape
# ABOUTME: by creating the drift, against a synthetic canonical checkout, offline
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# A sync script only ever run on an already-synced tree proves nothing: it would
# report success while doing nothing at all. Each case here BREAKS one mirror in
# one specific way and asserts both that `--check` reports it and that a plain
# run repairs it (carnet#325 for the evidence corpus, carnet#332 for the
# training catalogue).
#
# Hermetic on purpose. Rather than reading the real dravr-contremaitre checkout
# — which needs network, private-repo auth and a cargo fetch, none of which the
# compile-free fast-gate job has — it builds a throwaway CARGO_HOME laid out the
# way cargo lays one out, and lets the script resolve it through the same code
# path production uses. No test-only escape hatch in the script.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
UNDER_TEST="$REPO_ROOT/scripts/ci/sync-contremaitre-fallback.sh"
DEST="$REPO_ROOT/crates/pierre-evals/fixtures/sports_science"
TABLE="$REPO_ROOT/crates/pierre-services/src/claim_verification.rs"
TRAINING="$REPO_ROOT/training_catalogue"
EMBEDDED="$REPO_ROOT/crates/pierre-contremaitre/src/training_catalogue_embedded.rs"

pass=0; fail=0
ok()  { pass=$((pass + 1)); echo "  ✓ $1"; }
bad() { fail=$((fail + 1)); echo "  ✗ $1"; }

# The real fixtures are a git working copy, so restore through git. A snapshot
# of the wrong thing would silently make every case pass.
restore() {
    git -C "$REPO_ROOT" checkout -- "$DEST" "$TABLE" "$TRAINING" "$EMBEDDED" 2>/dev/null || true
    git -C "$REPO_ROOT" checkout -- "$REPO_ROOT/crates" 2>/dev/null || true
    git -C "$REPO_ROOT" clean -fdq -- "$DEST" "$TRAINING" 2>/dev/null || true
}
cleanup() { restore; [ -n "${FAKE_HOME:-}" ] && rm -rf "$FAKE_HOME"; }
trap cleanup EXIT

# `status --porcelain` rather than `diff --quiet`: an untracked file under one
# of the trees is work too, and `git clean` below would delete it.
if [ -n "$(git -C "$REPO_ROOT" status --porcelain -- "$DEST" "$TABLE" "$TRAINING" "$EMBEDDED")" ]; then
    echo "REFUSING TO RUN: one of these has uncommitted changes:" >&2
    printf '   %s\n' "$DEST" "$TABLE" "$TRAINING" "$EMBEDDED" >&2
    echo "This test rewrites all four and restores them with git checkout and" >&2
    echo "git clean, which would destroy that work. Commit or stash it first." >&2
    exit 2
fi

# A CARGO_HOME shaped the way cargo shapes one, holding canonical trees we
# control. Seeded from the committed fallbacks, so both mirrors start in sync
# and every case below is the ONLY difference.
PINNED="$(grep -h 'dravr-contremaitre = { git' "$REPO_ROOT"/crates/*/Cargo.toml \
          | grep -oE 'rev = "[a-f0-9]{7,40}"' | grep -oE '[a-f0-9]{7,40}' | sort -u | head -1)"
FAKE_HOME="$(mktemp -d)"
CHECKOUT="$FAKE_HOME/git/checkouts/dravr-contremaitre-0000000000000000/${PINNED:0:7}"
CANON="$CHECKOUT/evidence/sports_science"
TRAINING_CANON="$CHECKOUT/training"
mkdir -p "$CANON" "$TRAINING_CANON"
rsync -a --exclude='README.md' "$DEST/" "$CANON/"
rsync -a "$TRAINING/" "$TRAINING_CANON/"
export CARGO_HOME="$FAKE_HOME"

echo "sync-contremaitre-fallback.sh  (synthetic checkout, pinned ${PINNED:0:8})"

# --- case 0: the harness itself is honest ------------------------------------
# If the synthetic checkout did not resolve, every case below would "pass" by
# doing nothing. Assert the script can see it before trusting any result.
if "$UNDER_TEST" --check >/dev/null 2>&1; then
    ok "harness: the script resolves the synthetic checkout and starts in sync"
else
    bad "harness: script cannot see the synthetic checkout — every case below is vacuous"
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

# --- case 4: a catalogue file missing in-tree --------------------------------
# Upstream carries a flavour the platform copy lacks: the boot-time fallback
# would silently know fewer flavours than the catalogue declares.
missing_rel="flavours/$(ls "$TRAINING_CANON/flavours" | head -1)"
rm -f "$TRAINING/$missing_rel"
if "$UNDER_TEST" --check >/dev/null 2>&1; then
    bad "catalogue file missing in-tree: --check passed while $missing_rel was absent"
else
    ok "catalogue file missing in-tree: --check reports it"
fi
"$UNDER_TEST" >/dev/null 2>&1
if cmp -s "$TRAINING_CANON/$missing_rel" "$TRAINING/$missing_rel"; then
    ok "catalogue file missing in-tree: a plain run copies it back byte-identical"
else
    bad "catalogue file missing in-tree: $missing_rel still absent or differs after sync"
fi
if git -C "$REPO_ROOT" diff --quiet -- "$EMBEDDED"; then
    ok "catalogue file missing in-tree: the embedded table is regenerated to the committed shape"
else
    bad "catalogue file missing in-tree: the embedded table differs from the committed one after sync"
fi
restore

# --- case 5: extra files in-tree (proves --delete and the four-shape filter) --
# A workout upstream withdrew, and a README that never belonged under the tree:
# the first proves --delete, the second proves --delete-excluded removes what
# the shape filter refuses to carry.
cat > "$TRAINING/workouts/zzz-extra.toml" <<'TOML'
id = "00000000-0000-0000-0000-00000000ffff"
slug = "zzz-extra"
TOML
echo "stray" > "$TRAINING/README.md"
if "$UNDER_TEST" --check >/dev/null 2>&1; then
    bad "extra files in-tree: --check passed with a withdrawn workout and a stray README"
else
    ok "extra files in-tree: --check reports them"
fi
"$UNDER_TEST" >/dev/null 2>&1
if [ ! -f "$TRAINING/workouts/zzz-extra.toml" ]; then
    ok "extra files in-tree: a plain run removes the withdrawn workout (--delete)"
else
    bad "extra files in-tree: zzz-extra.toml survived the sync (is --delete still on the rsync?)"
fi
if [ ! -f "$TRAINING/README.md" ]; then
    ok "extra files in-tree: a plain run removes the stray README (--delete-excluded)"
else
    bad "extra files in-tree: README.md survived the sync (is --delete-excluded still on the rsync?)"
fi
if grep -q 'zzz-extra' "$EMBEDDED"; then
    bad "extra files in-tree: the embedded table still names the withdrawn workout"
else
    ok "extra files in-tree: the embedded table no longer names it"
fi
restore

# --- case 6: a catalogue body differs ----------------------------------------
# Same filename both sides, different bytes: a hand edit on the platform copy
# that never reached the canonical tree.
edited_rel="skeletons/$(ls "$TRAINING_CANON/skeletons" | head -1)"
printf '\n# a local edit that never reached contremaitre\n' >> "$TRAINING/$edited_rel"
if "$UNDER_TEST" --check >/dev/null 2>&1; then
    bad "catalogue body differs: --check passed while $edited_rel carried a local edit"
else
    ok "catalogue body differs: --check reports it"
fi
"$UNDER_TEST" >/dev/null 2>&1
if cmp -s "$TRAINING_CANON/$edited_rel" "$TRAINING/$edited_rel"; then
    ok "catalogue body differs: a plain run restores the canonical bytes"
else
    bad "catalogue body differs: $edited_rel still differs from canonical after sync"
fi
restore

# --- case 7: files right, embedded table stale -------------------------------
# The table is a generated artefact under version control, so a hand edit or a
# forgotten regeneration leaves a catalogue file unreachable from Rust while
# both trees still match. Unlike the evidence table, --check compares the
# rendered text with the committed file, so this drift is reported, not only
# repaired.
python3 - "$EMBEDDED" <<'PY'
import io, sys
p = sys.argv[1]
s = io.open(p, encoding="utf-8").read()
start = s.index("pub(crate) const EMBEDDED_WORKOUTS: &[(&str, &str)] = &[")
end = s.index("\n];\n", start)
cut = s.rindex("    (", start, end)
io.open(p, "w", encoding="utf-8").write(s[:cut] + s[end + 1:])
PY
if git -C "$REPO_ROOT" diff --quiet -- "$EMBEDDED"; then
    bad "stale embedded table: fixture setup failed to shorten the table"
else
    "$UNDER_TEST" --check >/dev/null 2>&1
    rc=$?
    if [ "$rc" = "1" ]; then
        ok "stale embedded table: --check reports it with exit 1"
    else
        bad "stale embedded table: --check exited $rc on a table missing an entry"
    fi
    "$UNDER_TEST" >/dev/null 2>&1
    if git -C "$REPO_ROOT" diff --quiet -- "$EMBEDDED"; then
        ok "stale embedded table: a plain run regenerates it to the committed shape"
    else
        bad "stale embedded table: still differs from the committed table after sync"
    fi
fi
restore
rm -f "$EMBEDDED"
if "$UNDER_TEST" --check >/dev/null 2>&1; then
    bad "stale embedded table: --check passed with the table file deleted outright"
else
    ok "stale embedded table: --check reports a table that does not exist"
fi
"$UNDER_TEST" >/dev/null 2>&1
if [ -f "$EMBEDDED" ] && git -C "$REPO_ROOT" diff --quiet -- "$EMBEDDED"; then
    ok "stale embedded table: a plain run recreates a deleted table"
else
    bad "stale embedded table: the table was not recreated to the committed shape"
fi
restore

# --- case 8: the pinned rev carries no training/ yet --------------------------
# The landing order: the platform seed lands before contremaitre carries the
# tree. The mirror half must skip without touching the in-tree copy, and the
# table must still be regenerated from whatever is on disk.
mv "$TRAINING_CANON" "$FAKE_HOME/held-training"
if "$UNDER_TEST" --check >/dev/null 2>&1; then
    ok "no training/ upstream: --check passes on the committed tree"
else
    bad "no training/ upstream: --check failed although the table matched the tree"
fi
cat > "$TRAINING/workouts/zzz-local-only.toml" <<'TOML'
id = "00000000-0000-0000-0000-00000000fffe"
slug = "zzz-local-only"
TOML
python3 - "$EMBEDDED" <<'PY'
import io, sys
p = sys.argv[1]
s = io.open(p, encoding="utf-8").read()
start = s.index("pub(crate) const EMBEDDED_SKELETONS: &[(&str, &str)] = &[")
end = s.index("\n];\n", start)
cut = s.rindex("    (", start, end)
io.open(p, "w", encoding="utf-8").write(s[:cut] + s[end + 1:])
PY
"$UNDER_TEST" >/dev/null 2>&1
if [ -f "$TRAINING/workouts/zzz-local-only.toml" ] && git -C "$REPO_ROOT" diff --quiet -- "$TRAINING"; then
    ok "no training/ upstream: a plain run leaves the in-tree catalogue untouched"
else
    bad "no training/ upstream: a plain run modified the in-tree catalogue with nothing to mirror"
fi
if grep -q 'zzz-local-only' "$EMBEDDED" \
   && [ "$(grep -c 'include_str!(.*/skeletons/' "$EMBEDDED")" = "$(ls "$TRAINING/skeletons" | wc -l | tr -d ' ')" ]; then
    ok "no training/ upstream: the embedded table is still regenerated from the tree on disk"
else
    bad "no training/ upstream: the embedded table was not regenerated from disk"
fi
mv "$FAKE_HOME/held-training" "$TRAINING_CANON"; restore

# --- case 9: idempotence -----------------------------------------------------
# Not idempotent means the bump lane commits a diff on every hourly no-op.
"$UNDER_TEST" >/dev/null 2>&1
"$UNDER_TEST" >/dev/null 2>&1
if git -C "$REPO_ROOT" diff --quiet -- "$DEST" "$TABLE" "$TRAINING" "$EMBEDDED" \
   && [ -z "$(git -C "$REPO_ROOT" status --porcelain -- "$DEST" "$TRAINING")" ]; then
    ok "idempotent: two consecutive runs leave every mirrored path unchanged"
else
    bad "idempotent: a run on an in-sync tree produced a diff"
    git -C "$REPO_ROOT" status --porcelain -- "$DEST" "$TABLE" "$TRAINING" "$EMBEDDED" | sed 's/^/      /'
fi
restore

# --- case 10: the generated table is rustfmt-canonical -----------------------
# CI's clippy job runs `cargo fmt --all -- --check`, so a generator that
# emitted a non-canonical layout would red main on the first bump. rustfmt is
# invoked on the file itself because `cargo fmt` only walks modules a `mod`
# declaration reaches, and a fresh generated file may not be declared yet.
if command -v rustfmt >/dev/null 2>&1; then
    "$UNDER_TEST" >/dev/null 2>&1
    if rustfmt --edition 2021 --check "$EMBEDDED" >/dev/null 2>&1; then
        ok "rustfmt: the generated table is already canonical"
    else
        bad "rustfmt: the generated table is not rustfmt-canonical"
        rustfmt --edition 2021 --check "$EMBEDDED" 2>&1 | sed 's/^/      /' | head -20
    fi
    restore
else
    echo "  - rustfmt: not on PATH, canonical layout not verified here"
fi

# --- case 11: consumers disagreeing on the rev is refused, not guessed -------
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
