#!/usr/bin/env bash
# ABOUTME: Mirrors dravr-contremaitre's canonical evidence/ into the in-tree fallback and
# ABOUTME: regenerates EMBEDDED_PROPOSITIONS, so a corpus addition upstream cannot red main
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# `check-contremaitre-sync.sh` requires EXACT bidirectional mirroring between
# contremaitre's canonical `evidence/sports_science/` and this repo's
# `crates/pierre-evals/fixtures/sports_science`. Nothing maintained that mirror:
# `contremaitre-bump.yml` moved the pinned rev and nothing else, so every corpus
# addition upstream reds main the moment the hourly bump fires. On 2026-09-04
# that cost a red main, five interrupted sessions and three extra CI cycles
# (carnet#325).
#
# This is the missing half of the bump. Idempotent: safe to run any time, and a
# no-op when already in sync.
#
# Usage:
#   scripts/ci/sync-evidence-corpus.sh            # sync in place
#   scripts/ci/sync-evidence-corpus.sh --check    # report drift, change nothing
#
# Exit 0 = in sync (or synced). Exit 1 = --check found drift. Exit 2 = cannot run.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

DEST="crates/pierre-evals/fixtures/sports_science"
TABLE="crates/pierre-services/src/claim_verification.rs"
CHECK_ONLY=0
[ "${1:-}" = "--check" ] && CHECK_ONLY=1

# Resolve the canonical corpus out of the PINNED rev, never "whatever is newest".
# The pin is the contract the rest of the checks read, so syncing against a
# different rev would swap one drift for another.
PINNED="$(grep -h 'dravr-contremaitre = { git' crates/*/Cargo.toml \
          | grep -oE 'rev = "[a-f0-9]{7,40}"' | grep -oE '[a-f0-9]{7,40}' | sort -u)"
if [ "$(printf '%s\n' "$PINNED" | wc -l | tr -d ' ')" != "1" ] || [ -z "$PINNED" ]; then
    echo "❌ consumers disagree on the dravr-contremaitre rev, or none pins it:" >&2
    printf '   %s\n' ${PINNED:-<none>} >&2
    echo "   Bump them to one rev before syncing the corpus." >&2
    exit 2
fi

# cargo's checkout directory is named by the SHORT sha.
CANON=""
for d in "${CARGO_HOME:-$HOME/.cargo}"/git/checkouts/dravr-contremaitre-*/"${PINNED:0:7}"/evidence/sports_science; do
    [ -d "$d" ] && CANON="$d" && break
done
if [ -z "$CANON" ]; then
    echo "❌ no checkout of dravr-contremaitre ${PINNED:0:8} under \$CARGO_HOME/git/checkouts." >&2
    echo "   Run 'cargo fetch' (or any cargo build) first — this script reads the" >&2
    echo "   pinned corpus from disk rather than cloning it a second time." >&2
    exit 2
fi

if [ "$CHECK_ONLY" = "1" ]; then
    if diff -rq --exclude=README.md "$CANON" "$DEST" >/dev/null 2>&1; then
        echo "✅ evidence corpus in sync with ${PINNED:0:8}"
        exit 0
    fi
    echo "❌ evidence corpus differs from canonical ${PINNED:0:8}:" >&2
    diff -rq --exclude=README.md "$CANON" "$DEST" 2>&1 | sed 's/^/   /' >&2
    exit 1
fi

# Mirror. --delete so a proposition RETRACTED upstream leaves the fallback too:
# the check is bidirectional, and a file we keep after it is withdrawn fails it
# just as loudly as one we never copied. README.md is ours and is excluded.
rsync -a --delete --exclude='README.md' "$CANON/" "$DEST/"

# Regenerate the include_str! table from the tree that now exists on disk, so
# the table cannot disagree with the directory it describes.
python3 - "$DEST" "$TABLE" <<'PY'
import io, os, sys

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
import re
s = re.sub(r"/// All \d+ embedded proposition files", "/// All %d embedded proposition files" % len(files), s, count=1)
s = re.sub(r"/// Parses the \d+ embedded markdown propositions", "/// Parses the %d embedded markdown propositions" % len(files), s, count=1)
s = re.sub(r"// ABOUTME: Embeds \d+ markdown propositions", "// ABOUTME: Embeds %d markdown propositions" % len(files), s, count=1)

io.open(table_path, "w", encoding="utf-8").write(s)
print("%d propositions across %s" % (len(files), ", ".join("%s %d" % (k, len(v)) for k, v in sorted(by_cat.items()))))
PY

command -v cargo >/dev/null 2>&1 && cargo fmt -p pierre-services 2>/dev/null || true
echo "✅ evidence corpus synced from dravr-contremaitre ${PINNED:0:8}"
