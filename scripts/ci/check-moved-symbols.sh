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

# The shared base rule: the argument, else $GATE_BASE_REF, else origin/main, and
# HEAD~1 whenever that is missing or equals HEAD.
# shellcheck source=scripts/ci/gate-base-ref.sh
. "$(dirname "${BASH_SOURCE[0]}")/gate-base-ref.sh"
if ! BASE_REF="$(resolve_gate_base_ref "${1:-}")"; then
    echo "✅ moved-symbols: HEAD is a root commit — nothing to diff against"
    exit 0
fi

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
#
# Qualified with the crate module, because a leaf name alone is not a path: a
# dozen crates here have a `coaches` or a `types` module, and grepping for
# `coaches::` matched `pierre_core::models::coaches::` in files that had never
# heard of the item that moved.
module_suffix() {
    local f="$1"
    local rel="${f#crates/*/src/}"
    local crate_dir
    crate_dir="$(echo "$f" | sed -E 's|^crates/([^/]+)/src/.*|\1|')"
    local crate_mod="${crate_dir//-/_}"
    case "$rel" in
        lib.rs) echo "$crate_mod" ;;
        */mod.rs) echo "${crate_mod}::$(echo "${rel%/mod.rs}" | tr '/' ':' | sed 's/:/::/g')" ;;
        *) echo "${crate_mod}::$(echo "${rel%.rs}" | tr '/' ':' | sed 's/:/::/g')" ;;
    esac
}

# One declaration keyword class, shared by the removal and re-add patterns.
DECL='(fn|struct|enum|trait|const|static|type)'
PUBVIS='pub([[:space:]]*\([^)]*\))?'
MODS='((const|async|unsafe)[[:space:]]+)*'

# The commit the three-dot diff compares HEAD against.
MERGE_BASE="$(git merge-base "$BASE_REF" HEAD 2>/dev/null || echo "$BASE_REF")"

# Every column-0 `pub use` statement of file $2 at revision $1, each joined
# onto one line. rustfmt wraps a brace group that outgrows the line, so a
# re-export's names are read from whole statements: a group that only gained a
# name reads in the diff as removed on one line and re-added across several,
# and a name dropped from a wrapped group sits on a line that does not start
# with `pub use`. A file absent at that revision has none.
pub_uses_at() {
    git show "$1:$2" 2>/dev/null | awk '
        /^pub([[:space:]]*\([^)]*\))?[[:space:]]+use[[:space:]]/ { stmt = ""; collecting = 1 }
        collecting { stmt = stmt " " $0 }
        collecting && /;/ { print stmt; collecting = 0 }
    ' || true
}

# The names joined `pub use` statements on stdin export. Brace re-exports
# (`pub use a::{b, c};`) yield every braced name.
reexported_names() {
    sed -E "s/^[[:space:]]*${PUBVIS}[[:space:]]+use[[:space:]]+//; s/;.*$//" \
        | sed -E 's/.*::([^:]*)$/\1/; s/[{}]//g; s/,/ /g' \
        | tr ' ' '\n' | grep -E '^[A-Za-z_][A-Za-z0-9_]*$' | sort -u || true
}

# Pub item names a change to file $2 removes: declarations its diff $1 drops,
# and names the file re-exported at the merge base and no longer does. The
# declaration match is anchored at column 0 and taken whole, so the item name
# is always the last field (`pub const fn new` yields `new`, not `fn`).
removed_pub_names() {
    local diff="$1" file="$2"
    {
        echo "$diff" \
            | grep -oE "^-${PUBVIS}[[:space:]]+${MODS}${DECL}[[:space:]]+[A-Za-z_][A-Za-z0-9_]*" \
            | awk '{print $NF}' || true
        comm -23 \
            <(pub_uses_at "$MERGE_BASE" "$file" | reexported_names) \
            <(pub_uses_at HEAD "$file" | reexported_names)
    } | sort -u
}

for f in "${changed_src[@]}"; do
    diff_text="$(git diff --no-renames "$BASE_REF"...HEAD -- "$f" 2>/dev/null || true)"
    [[ -z "$diff_text" ]] && continue

    names="$(removed_pub_names "$diff_text" "$f")"
    [[ -z "$names" ]] && continue
    pub_uses="$(pub_uses_at HEAD "$f")"

    suffix="$(module_suffix "$f")"
    [[ -z "$suffix" ]] && continue

    # The crate that owns the file, and the module path inside it (empty for a
    # crate root, which has no `crate::<inner>` spelling of its own).
    crate_dir="$(echo "$f" | sed -E 's|^crates/([^/]+)/src/.*|\1|')"
    inner="${suffix#*::}"
    [[ "$inner" == "$suffix" ]] && inner=""

    while IFS= read -r name; do
        [[ -z "$name" ]] && continue
        # Re-added in the same file (an in-place refactor, not a move)?
        if echo "$diff_text" | grep -Eq "^\+${PUBVIS}[[:space:]]+${MODS}${DECL}[[:space:]]+${name}([^A-Za-z0-9_]|$)"; then
            continue
        fi
        # Still re-exported from the same file, however rustfmt wrapped it?
        if echo "$pub_uses" | grep -Eq "[^A-Za-z0-9_]${name}([^A-Za-z0-9_]|$)"; then
            continue
        fi
        checked=$((checked + 1))

        # Files still importing the old module path AND naming the item.
        # The changed file itself is excluded — its own references are the
        # compiler's job, and on a deleted file they no longer exist.
        #
        # Two spellings reach one module: another crate writes the fully
        # qualified path, the owning crate writes `crate::`. The second is
        # searched only inside the owning crate, where it means that module.
        hits="$({
            grep -rln --include='*.rs' "${suffix}::" crates/ 2>/dev/null || true
            if [[ -n "$inner" ]]; then
                grep -rln --include='*.rs' "crate::${inner}::" "crates/${crate_dir}/" 2>/dev/null || true
            fi
          } | sort -u \
            | grep -v -F "$f" \
            | while IFS= read -r candidate; do
                # The name has to appear in the SAME `use` statement as the old
                # module path. Matching it anywhere in the file flagged every
                # file that imports one item from the old module and the moved
                # item from its new home — `data.rs` imports
                # `resolve_provider_for_tool` from `provider_helpers` and
                # `build_activities_success_response` from `fitness_support`,
                # and read as an offender for a path it does not use.
                #
                # `use` statements run to their `;`, so join them first: a
                # rustfmt-wrapped group puts the path and the name on
                # different lines.
                #
                # And the name has to be imported AT that path: `path::item`,
                # or a top-level entry of a `path::{...}` group. A removed
                # crate-root re-export leaves `dravr_cageux::analyzer::
                # ActivityAnalyzer` compiling, and reading the name anywhere
                # after `path::` flagged it as stranded.
                if awk -v path="$suffix" -v inner="$inner" -v item="$name" '
                    function imports_at(stmt, prefix,    rest, pos, depth, i, c, tok, n, k, toks) {
                        pos = index(stmt, prefix)
                        while (pos > 0) {
                            rest = substr(stmt, pos + length(prefix))
                            if (match(rest, "^" item "([^A-Za-z0-9_]|$)")) return 1
                            if (substr(rest, 1, 1) == "{") {
                                depth = 0; tok = ""; n = 0
                                for (i = 1; i <= length(rest); i++) {
                                    c = substr(rest, i, 1)
                                    if (c == "{") { depth++; if (depth == 1) continue }
                                    if (c == "}") { depth--; if (depth == 0) { toks[++n] = tok; break } }
                                    if (c == "," && depth == 1) { toks[++n] = tok; tok = ""; continue }
                                    tok = tok c
                                }
                                for (k = 1; k <= n; k++) {
                                    gsub(/^[[:space:]]+|[[:space:]]+$/, "", toks[k])
                                    if (match(toks[k], "^" item "([[:space:]]+as[[:space:]]|$)")) return 1
                                }
                            }
                            rest = substr(stmt, pos + length(prefix))
                            k = index(rest, prefix)
                            pos = (k > 0) ? pos + length(prefix) + k - 1 : 0
                        }
                        return 0
                    }
                    /^[[:space:]]*(pub[[:space:]]+)?use[[:space:]]/ { stmt = $0; collecting = 1 }
                    collecting && !/^[[:space:]]*(pub[[:space:]]+)?use[[:space:]]/ { stmt = stmt " " $0 }
                    collecting && /;/ {
                        collecting = 0
                        if (imports_at(stmt, path "::") ||
                            (inner != "" && imports_at(stmt, "crate::" inner "::"))) {
                            found = 1
                        }
                    }
                    END { exit(found ? 0 : 1) }
                  ' "$candidate"; then
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
