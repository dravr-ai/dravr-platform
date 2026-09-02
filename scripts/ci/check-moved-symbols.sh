#!/usr/bin/env bash
# ABOUTME: Compile-free moved-symbol check — fails a push that strands importers of a pub item's old path
# ABOUTME: Closes carnet#197: a library symbol move touches no server test, so Tier 1e compiles nothing
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# A push that moves or removes a `pub` item from a library module touches no
# file under crates/pierre-server/tests/, so Tier 1e's changed-test clippy
# runs zero targets — while every test (or straggler src file) importing the
# old path fails to compile 10+ minutes later in CI's full-workspace job
# (the data.rs → data_helpers split, 2026-09-02, fixed in 4869295ec).
#
# This check is diff-driven and compile-free, in the shape of Tiers 1c/1d:
#   1. For each changed/deleted crates/*/src file, collect the pub item names
#      the diff REMOVES (fn/struct/enum/trait/const/static/type, and pub use
#      re-exports) that it does not re-add in the same file. Only column-0
#      declarations count: an indented `pub fn` is a method or an associated
#      item, reached through its type rather than the module path, so deleting
#      one strands no importer of the module.
#   2. Derive the file's module path suffix (src/a/b.rs → "a::b",
#      src/a/mod.rs → "a", src/lib.rs → the crate name).
#   3. A file anywhere under crates/ that still references BOTH the old module
#      path and the removed name is a stranded importer — fail and name it.
#
# Two-stage matching (module path first, then the bare name inside those
# files) keeps brace imports (`use x::y::{a, b}`) visible to a line grep.

set -euo pipefail

BASE_REF="${1:-origin/main}"

fail=0
checked=0

# Changed or deleted library sources. Renamed files (R) appear as D+A pairs
# under --no-renames, which is exactly the shape we want to inspect.
mapfile -t changed_src < <(git diff --no-renames --name-only --diff-filter=MD "$BASE_REF"...HEAD -- 'crates/*/src/**/*.rs' 'crates/*/src/*.rs' 2>/dev/null || true)

if [[ ${#changed_src[@]} -eq 0 ]]; then
    echo "✅ moved-symbols: no library sources changed"
    exit 0
fi

# Module path suffix for a source file, for import-path matching.
module_suffix() {
    local f="$1"
    local rel="${f#crates/*/src/}"
    local crate_dir
    crate_dir="$(echo "$f" | sed -E 's|^crates/([^/]+)/src/.*|\1|')"
    local crate_mod="${crate_dir//-/_}"
    case "$rel" in
        lib.rs) echo "$crate_mod" ;;
        */mod.rs) echo "${rel%/mod.rs}" | tr '/' ':' | sed 's/:/::/g' ;;
        *) echo "${rel%.rs}" | tr '/' ':' | sed 's/:/::/g' ;;
    esac
}

# One declaration keyword class, shared by the removal and re-add patterns.
DECL='(fn|struct|enum|trait|const|static|type)'
PUBVIS='pub([[:space:]]*\([^)]*\))?'
MODS='((const|async|unsafe)[[:space:]]+)*'

# Pub item names a diff removes: declarations and re-exports. The declaration
# match is anchored at column 0 and taken whole, so the item name is always
# the last field (`pub const fn new` yields `new`, not `fn`). Brace re-exports
# (`pub use a::{b, c};`) yield every braced name.
removed_pub_names() {
    local diff="$1"
    {
        echo "$diff" \
            | grep -oE "^-${PUBVIS}[[:space:]]+${MODS}${DECL}[[:space:]]+[A-Za-z_][A-Za-z0-9_]*" \
            | awk '{print $NF}' || true
        echo "$diff" \
            | grep -E "^-${PUBVIS}[[:space:]]+use[[:space:]]" \
            | sed -E "s/^-${PUBVIS}[[:space:]]+use[[:space:]]+//; s/;.*$//" \
            | sed -E 's/.*::([^:]*)$/\1/; s/[{}]//g; s/,/ /g' \
            | tr ' ' '\n' | grep -E '^[A-Za-z_][A-Za-z0-9_]*$' || true
    } | sort -u
}

for f in "${changed_src[@]}"; do
    diff_text="$(git diff --no-renames "$BASE_REF"...HEAD -- "$f" 2>/dev/null || true)"
    [[ -z "$diff_text" ]] && continue

    names="$(removed_pub_names "$diff_text")"
    [[ -z "$names" ]] && continue

    suffix="$(module_suffix "$f")"
    [[ -z "$suffix" ]] && continue

    while IFS= read -r name; do
        [[ -z "$name" ]] && continue
        # Re-added in the same file (an in-place refactor, not a move)?
        if echo "$diff_text" | grep -Eq "^\+${PUBVIS}[[:space:]]+${MODS}${DECL}[[:space:]]+${name}([^A-Za-z0-9_]|$)"; then
            continue
        fi
        if echo "$diff_text" | grep -Eq "^\+${PUBVIS}[[:space:]]+use[[:space:]].*[^A-Za-z0-9_]${name}([^A-Za-z0-9_]|$)"; then
            continue
        fi
        checked=$((checked + 1))

        # Files still importing the old module path AND naming the item.
        # The changed file itself is excluded — its own references are the
        # compiler's job, and on a deleted file they no longer exist.
        hits="$(grep -rln --include='*.rs' "${suffix}::" crates/ 2>/dev/null \
            | grep -v -F "$f" \
            | while IFS= read -r candidate; do
                if grep -Eq "(^|[^A-Za-z0-9_])${name}([^A-Za-z0-9_]|$)" "$candidate"; then
                    echo "$candidate"
                fi
              done || true)"
        if [[ -n "$hits" ]]; then
            echo "❌ '${name}' left '${suffix}' but these still import the old path:"
            while IFS= read -r hit; do
                grep -n "${suffix}::" "$hit" | head -2 | sed "s|^|     $hit:|"
            done <<< "$hits"
            fail=1
        fi
    done <<< "$names"
done

if [[ "$fail" -ne 0 ]]; then
    echo ""
    echo "FAIL: a pub item moved or was removed while survivors import its old path."
    echo "Repoint the importers (or restore a 'pub use' re-export at the old path)."
    exit 1
fi

echo "✅ moved-symbols: ${checked} removed pub item(s) checked, no stranded importers"
