#!/usr/bin/env bash
# ABOUTME: Fails when a declared test target is not built — the silent-skip hole that required-features opens
# ABOUTME: Compares cargo metadata's declared test targets against the binaries a --all-features build produced
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# `required-features` on a [[test]] target makes cargo SKIP that target when the
# features are absent: no compile error, no `running 0 tests`, no row in the
# output at all. That is the desired behaviour for a build that legitimately has
# the features off, and a silent loss of coverage everywhere else — a feature
# renamed, a typo in the list, or a gate that no profile can satisfy retires the
# test with nothing going red.
#
# This asks the two sides independently. `cargo metadata` states which test
# targets a package DECLARES; a `--all-features --no-run` build states which it
# actually PRODUCED. Under --all-features every required-features list is
# satisfiable by construction, so the two sets must be equal. Anything declared
# and not built is a test that stopped running without anyone deciding it should.
#
# Usage:  check-leaf-test-targets.sh -p <pkg> [-p <pkg> ...]

set -uo pipefail

if [ "$#" -eq 0 ]; then
    echo "::error::No packages passed — refusing to report green."
    exit 1
fi

PKG_FLAGS=("$@")
PKGS=()
for arg in "$@"; do
    [ "$arg" = "-p" ] || PKGS+=("$arg")
done

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# ---- side 1: what the manifests DECLARE -------------------------------------
# `cargo metadata` compiles nothing, so this side is independent of the build.
if ! cargo metadata --no-deps --format-version 1 > "$WORK/metadata.json" 2>"$WORK/metadata.err"; then
    echo "::error::cargo metadata failed — cannot establish the declared target set."
    cat "$WORK/metadata.err"
    exit 1
fi

if ! python3 - "$WORK/metadata.json" "$WORK/declared.txt" "${PKGS[@]}" <<'PY'
import json, sys
meta_path, out_path, *wanted = sys.argv[1:]
with open(meta_path) as fh:
    meta = json.load(fh)
seen = set()
rows = []
for pkg in meta["packages"]:
    if pkg["name"] not in wanted:
        continue
    seen.add(pkg["name"])
    for target in pkg["targets"]:
        if "test" in target["kind"]:
            rows.append(f'{pkg["manifest_path"]}\t{target["name"]}')
missing_pkgs = sorted(set(wanted) - seen)
if missing_pkgs:
    print(f"PACKAGES NOT IN WORKSPACE: {', '.join(missing_pkgs)}", file=sys.stderr)
    sys.exit(1)
with open(out_path, "w") as fh:
    fh.write("\n".join(sorted(rows)) + "\n")
PY
then
    echo "::error::Could not read declared test targets from cargo metadata."
    exit 1
fi

DECLARED_COUNT=$(grep -c . < "$WORK/declared.txt")
if [ "$DECLARED_COUNT" -eq 0 ]; then
    echo "::error::The selected packages declare no test targets at all — refusing to report green."
    exit 1
fi

# ---- side 2: what a --all-features build PRODUCES ----------------------------
# Exit status is checked on its own line. Piping cargo into the parser would
# report the PARSER's status and a crashed cargo would read as "nothing missing".
cargo test "${PKG_FLAGS[@]}" --all-features --tests --no-run \
    --message-format=json > "$WORK/artifacts.json" 2>"$WORK/build.err"
BUILD_STATUS=$?
if [ "$BUILD_STATUS" -ne 0 ]; then
    echo "::error::The --all-features test build failed (exit $BUILD_STATUS); target coverage is unknown."
    tail -40 "$WORK/build.err"
    exit 1
fi

if ! python3 - "$WORK/artifacts.json" "$WORK/built.txt" <<'PY'
import json, sys
src, out = sys.argv[1], sys.argv[2]
rows = set()
with open(src) as fh:
    for line in fh:
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if msg.get("reason") != "compiler-artifact" or not msg.get("executable"):
            continue
        target = msg.get("target", {})
        if "test" not in target.get("kind", []):
            continue
        # manifest_path is the join key: package_id's shape has changed
        # between cargo versions, manifest_path has not, and cargo metadata
        # states the same absolute path for the same package.
        manifest = msg.get("manifest_path")
        if not manifest:
            continue
        rows.add(f'{manifest}\t{target["name"]}')
with open(out, "w") as fh:
    fh.write("\n".join(sorted(rows)) + "\n")
PY
then
    echo "::error::Could not read built test targets from the cargo build log."
    exit 1
fi

BUILT_COUNT=$(grep -c . < "$WORK/built.txt")
if [ "$BUILT_COUNT" -eq 0 ]; then
    echo "::error::The build produced no test binaries at all — refusing to report green."
    exit 1
fi

# ---- compare ----------------------------------------------------------------
MISSING="$(comm -23 "$WORK/declared.txt" "$WORK/built.txt")"
if [ -n "$MISSING" ]; then
    echo "::error::Declared test targets were not built. required-features on these"
    echo "targets cannot be satisfied even with --all-features, so they run nowhere:"
    echo ""
    printf '%s\n' "$MISSING" | sed 's#^.*/crates/#  #; s#/Cargo.toml\t# #'
    echo ""
    echo "Fix the required-features list in the crate's Cargo.toml (a renamed or"
    echo "misspelled feature never becomes satisfiable), or delete the target."
    exit 1
fi

echo "check-leaf-test-targets: OK — $DECLARED_COUNT declared test targets, all $BUILT_COUNT built."
