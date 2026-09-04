#!/usr/bin/env bash
# ABOUTME: Mirrors dravr-contremaitre's canonical evidence/ and training/ into the in-tree
# ABOUTME: fallbacks and regenerates their include_str! tables, so an upstream change cannot red main
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# `check-contremaitre-sync.sh` requires EXACT bidirectional mirroring between two
# canonical contremaitre trees and their in-tree fallbacks:
#
#   evidence/sports_science/  ->  crates/pierre-evals/fixtures/sports_science
#                                 + EMBEDDED_PROPOSITIONS in
#                                   crates/pierre-services/src/claim_verification.rs
#   training/                 ->  training_catalogue/
#                                 + crates/pierre-contremaitre/src/training_catalogue_embedded.rs
#
# Nothing maintained the evidence mirror: `contremaitre-bump.yml` moved the pinned
# rev and nothing else, so every corpus addition upstream red main the moment the
# hourly bump fired. On 2026-09-04 that cost a red main, five interrupted sessions
# and three extra CI cycles (carnet#325). This is the missing half of the bump,
# and the training catalogue rides the same mechanism from birth (carnet#332).
#
# The training half: `rsync -a --delete` of the four catalogue shapes only —
# flavours/*.yaml, skeletons/*.yaml, workouts/*.toml, selection.yaml — so
# anything else under training_catalogue/ is removed. When the pinned checkout
# carries no training/ yet, the tree on disk is left untouched (the seed lands
# in the platform first; see the landing order in the spec) and the embedded
# table is STILL regenerated from disk, so the table can never disagree with
# the directory it describes.
#
# The embedded table is written in rustfmt's own layout (verified against
# rustfmt 1.9: a multi-line tuple per entry for two or more entries, the
# hugging `&[(` form for exactly one, `&[]` for none, and the selection line
# wrapped after `=` because the path pushes it past 100 columns), so `--check`
# can compare the rendered text with the committed file byte for byte.
#
# Idempotent: safe to run any time, and a no-op when already in sync.
#
# Usage:
#   scripts/ci/sync-contremaitre-fallback.sh            # sync in place
#   scripts/ci/sync-contremaitre-fallback.sh --check    # report drift, change nothing
#
# Exit 0 = in sync (or synced). Exit 1 = --check found drift. Exit 2 = cannot run.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

EVIDENCE_DEST="crates/pierre-evals/fixtures/sports_science"
EVIDENCE_TABLE="crates/pierre-services/src/claim_verification.rs"
TRAINING_DEST="training_catalogue"
TRAINING_TABLE="crates/pierre-contremaitre/src/training_catalogue_embedded.rs"
CHECK_ONLY=0
[ "${1:-}" = "--check" ] && CHECK_ONLY=1

# Resolve the canonical trees out of the PINNED rev, never "whatever is newest".
# The pin is the contract the rest of the checks read, so syncing against a
# different rev would swap one drift for another.
PINNED="$(grep -h 'dravr-contremaitre = { git' crates/*/Cargo.toml \
          | grep -oE 'rev = "[a-f0-9]{7,40}"' | grep -oE '[a-f0-9]{7,40}' | sort -u)"
if [ "$(printf '%s\n' "$PINNED" | wc -l | tr -d ' ')" != "1" ] || [ -z "$PINNED" ]; then
    echo "❌ consumers disagree on the dravr-contremaitre rev, or none pins it:" >&2
    printf '   %s\n' ${PINNED:-<none>} >&2
    echo "   Bump them to one rev before syncing the fallbacks." >&2
    exit 2
fi

# cargo's checkout directory is named by the SHORT sha. A contremaitre checkout
# always carries evidence/, so that is what identifies it.
CHECKOUT=""
for d in "${CARGO_HOME:-$HOME/.cargo}"/git/checkouts/dravr-contremaitre-*/"${PINNED:0:7}"; do
    [ -d "$d/evidence/sports_science" ] && CHECKOUT="$d" && break
done
if [ -z "$CHECKOUT" ]; then
    echo "❌ no checkout of dravr-contremaitre ${PINNED:0:8} under \$CARGO_HOME/git/checkouts." >&2
    echo "   Run 'cargo fetch' (or any cargo build) first — this script reads the" >&2
    echo "   pinned trees from disk rather than cloning them a second time." >&2
    exit 2
fi
EVIDENCE_CANON="$CHECKOUT/evidence/sports_science"
TRAINING_CANON="$CHECKOUT/training"

DRIFT=0

# ---------------------------------------------------------------------------
# Evidence half
# ---------------------------------------------------------------------------
if [ "$CHECK_ONLY" = "1" ]; then
    if diff -rq --exclude=README.md "$EVIDENCE_CANON" "$EVIDENCE_DEST" >/dev/null 2>&1; then
        echo "✅ evidence corpus in sync with ${PINNED:0:8}"
    else
        echo "❌ evidence corpus differs from canonical ${PINNED:0:8}:" >&2
        diff -rq --exclude=README.md "$EVIDENCE_CANON" "$EVIDENCE_DEST" 2>&1 | sed 's/^/   /' >&2
        DRIFT=1
    fi
else
    # Mirror. --delete so a proposition RETRACTED upstream leaves the fallback too:
    # the check is bidirectional, and a file we keep after it is withdrawn fails it
    # just as loudly as one we never copied. README.md is ours and is excluded.
    rsync -a --delete --exclude='README.md' "$EVIDENCE_CANON/" "$EVIDENCE_DEST/"

    # Regenerate the include_str! table from the tree that now exists on disk, so
    # the table cannot disagree with the directory it describes.
    python3 - "$EVIDENCE_DEST" "$EVIDENCE_TABLE" <<'PY'
import io, os, re, sys

dest, table_path = sys.argv[1], sys.argv[2]
files = sorted(
    os.path.relpath(os.path.join(dirpath, name), dest)
    for dirpath, _, names in os.walk(dest)
    for name in names
    if name.endswith(".md") and name != "README.md"
)

by_cat = {}
for rel in files:
    by_cat.setdefault(rel.split("/")[0], []).append(rel)

LABEL = {
    "injury_rehab": "Injury / rehab",
    "nutrition": "Nutrition",
    "physiological": "Physiological",
    "recovery": "Recovery",
    "supplement": "Supplement",
    "training_prescription": "Training prescription",
}

lines = ["const EMBEDDED_PROPOSITIONS: &[(&str, &str)] = &["]
for cat in sorted(by_cat):
    lines.append("    // %s (%d)" % (LABEL.get(cat, cat), len(by_cat[cat])))
    for rel in by_cat[cat]:
        lines.append("    (")
        lines.append('        "%s",' % rel)
        lines.append(
            '        include_str!("../../pierre-evals/fixtures/sports_science/%s"),' % rel
        )
        lines.append("    ),")
lines.append("];")

s = io.open(table_path, encoding="utf-8").read()
start = s.index("const EMBEDDED_PROPOSITIONS: &[(&str, &str)] = &[")
end = s.index("\n];\n", start) + len("\n];\n")
s = s[:start] + "\n".join(lines) + "\n" + s[end:]

# The counts named in prose alongside the table are part of the mirror.
s = re.sub(r"/// All \d+ embedded proposition files", "/// All %d embedded proposition files" % len(files), s, count=1)
s = re.sub(r"/// Parses the \d+ embedded markdown propositions", "/// Parses the %d embedded markdown propositions" % len(files), s, count=1)
s = re.sub(r"// ABOUTME: Embeds \d+ markdown propositions", "// ABOUTME: Embeds %d markdown propositions" % len(files), s, count=1)

io.open(table_path, "w", encoding="utf-8").write(s)
print("%d propositions across %s" % (len(files), ", ".join("%s %d" % (k, len(v)) for k, v in sorted(by_cat.items()))))
PY

    command -v cargo >/dev/null 2>&1 && cargo fmt -p pierre-services 2>/dev/null || true
    echo "✅ evidence corpus synced from dravr-contremaitre ${PINNED:0:8}"
fi

# ---------------------------------------------------------------------------
# Training half: the tree
# ---------------------------------------------------------------------------
if [ ! -d "$TRAINING_CANON" ]; then
    echo "ℹ️  dravr-contremaitre ${PINNED:0:8} carries no training/ — $TRAINING_DEST/ left as committed"
elif [ "$CHECK_ONLY" = "1" ]; then
    if diff -rq "$TRAINING_CANON" "$TRAINING_DEST" >/dev/null 2>&1; then
        echo "✅ training catalogue in sync with ${PINNED:0:8}"
    else
        echo "❌ training catalogue differs from canonical ${PINNED:0:8}:" >&2
        diff -rq "$TRAINING_CANON" "$TRAINING_DEST" 2>&1 | sed 's/^/   /' >&2
        DRIFT=1
    fi
else
    # Only the four catalogue shapes cross; --delete-excluded removes anything
    # else that has crept into the in-tree copy, since the contract is that
    # nothing else lives under training/.
    rsync -a --delete --delete-excluded \
        --include='/flavours/' --include='/flavours/*.yaml' \
        --include='/skeletons/' --include='/skeletons/*.yaml' \
        --include='/workouts/' --include='/workouts/*.toml' \
        --include='/selection.yaml' \
        --exclude='*' \
        "$TRAINING_CANON/" "$TRAINING_DEST/"
    echo "✅ training catalogue synced from dravr-contremaitre ${PINNED:0:8}"
fi

# ---------------------------------------------------------------------------
# Training half: the embedded table, always rendered from the tree on disk
# ---------------------------------------------------------------------------
if python3 - "$TRAINING_DEST" "$TRAINING_TABLE" "$CHECK_ONLY" <<'PY'
import difflib, io, os, sys

dest, table_path, check_only = sys.argv[1], sys.argv[2], sys.argv[3] == "1"
# The include_str! paths are relative to the table's own directory.
rel_root = os.path.relpath(dest, os.path.dirname(table_path))


def listing(subdir, ext):
    d = os.path.join(dest, subdir)
    if not os.path.isdir(d):
        return []
    return sorted(n[: -len(ext)] for n in os.listdir(d) if n.endswith(ext))


def table(name, doc, subdir, ext):
    entries = [(slug, "%s/%s/%s%s" % (rel_root, subdir, slug, ext)) for slug in listing(subdir, ext)]
    lines = ["/// (slug, %s) for every `training_catalogue/%s/*%s`, sorted by slug." % (doc, subdir, ext)]
    head = "pub(crate) const %s: &[(&str, &str)] = &[" % name
    if not entries:
        lines.append(head + "];")
    elif len(entries) == 1:
        slug, path = entries[0]
        lines += [head + "(", '    "%s",' % slug, '    include_str!("%s"),' % path, ")];"]
    else:
        lines.append(head)
        for slug, path in entries:
            lines += ["    (", '        "%s",' % slug, '        include_str!("%s"),' % path, "    ),"]
        lines.append("];")
    return lines


lines = [
    "// ABOUTME: Generated by scripts/ci/sync-contremaitre-fallback.sh — the compiled-in training catalogue",
    "// ABOUTME: Mirrors training_catalogue/ at the repo root; edit the files and regenerate, never this table",
    "//",
    "// SPDX-License-Identifier: MIT OR Apache-2.0",
    "// Copyright (c) 2026 dravr.ai",
    "",
]
lines += table("EMBEDDED_FLAVOURS", "yaml", "flavours", ".yaml")
lines += table("EMBEDDED_SKELETONS", "yaml", "skeletons", ".yaml")
lines += table("EMBEDDED_WORKOUTS", "toml", "workouts", ".toml")
lines += [
    "/// `training_catalogue/selection.yaml`.",
    "pub(crate) const EMBEDDED_SELECTION: &str =",
    '    include_str!("%s/selection.yaml");' % rel_root,
]
rendered = "\n".join(lines) + "\n"

counts = "%d flavours, %d skeletons, %d workouts" % (
    len(listing("flavours", ".yaml")), len(listing("skeletons", ".yaml")), len(listing("workouts", ".toml")),
)
if check_only:
    on_disk = io.open(table_path, encoding="utf-8").read() if os.path.isfile(table_path) else ""
    if on_disk == rendered:
        print("✅ embedded training catalogue table matches the tree (%s)" % counts)
        sys.exit(0)
    sys.stderr.write("❌ %s is stale against %s/ (%s):\n" % (table_path, dest, counts))
    for line in difflib.unified_diff(
        on_disk.splitlines(), rendered.splitlines(), "committed", "rendered", lineterm="", n=1
    ):
        sys.stderr.write("   %s\n" % line)
    sys.stderr.write("   Run scripts/ci/sync-contremaitre-fallback.sh to regenerate it.\n")
    sys.exit(1)

io.open(table_path, "w", encoding="utf-8").write(rendered)
print("✅ embedded training catalogue table regenerated (%s)" % counts)
PY
then :; else DRIFT=1; fi

[ "$DRIFT" = "0" ]
