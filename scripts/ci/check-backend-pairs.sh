#!/usr/bin/env bash
# ABOUTME: Compile-free backend-pair check — fails a push that adds or edits a mirrored SQLite/Postgres repository
# ABOUTME: file whose SQL is still written twice instead of once in the shared trait module
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# pierre-database keeps one implementation per backend: a SQLite impl under
# crates/pierre-database/src/database/<name>.rs and a PostgreSQL impl under
# crates/pierre-database/src/backends/postgres/<name>.rs, both implementing the
# trait in crates/pierre-database/src/repositories/. Where those two files each
# carry their own copy of the SQL, an edit lands on one and not the other, and
# nothing catches it: cargo check, clippy, the test suite and pre-push all run
# SQLite, so the Postgres half is first exercised in CI — or in production.
# Two sessions shipped that break on the same day (2026-09-03).
#
# The converged shape removes the second copy rather than policing it: the
# statements become `pub(crate) const *_SQL` in the trait module using $n
# placeholders (sqlx accepts $n on both drivers), the row extractor becomes one
# generic fn over `sqlx::Row`, and a `macro_rules!` emits the impl block for
# each backend from that single body. repositories/commitments.rs,
# repositories/resumable_turns.rs, repositories/short_links.rs and
# repositories/guardian_actions.rs are the worked examples.
#
# This check is diff-driven and compile-free, in the shape of Tiers 1c/1d/1e-move:
#   1. Find every mirrored pair the diff adds or edits on either side.
#   2. A pair is converged when its trait module declares shared SQL (a
#      `*_SQL` const or a `sql!`-style literal macro) AND both impl files
#      reference that module's shared items rather than holding SQL of their
#      own. A pair the diff touches that is not converged fails the push.
#   3. Pairs the diff does not touch are reported as standing stock, never
#      failed on — the conversion is incremental by design (carnet#436).
#
# House gate rules (CLAUDE.md): every tool's exit status is checked separately
# from any grep of its output, the scan fails closed when it resolved no pair
# directory at all, and the success path prints a marker rather than relying on
# the absence of a finding.

set -euo pipefail

BASE_REF="${1:-origin/main}"

DB_ROOT="crates/pierre-database/src"
SQLITE_DIR="$DB_ROOT/database"
PG_DIR="$DB_ROOT/backends/postgres"
TRAIT_DIR="$DB_ROOT/repositories"

# Fail closed: a scan that verified nothing must not pass as "in sync".
for d in "$SQLITE_DIR" "$PG_DIR" "$TRAIT_DIR"; do
    if [[ ! -d "$d" ]]; then
        echo "❌ backend-pairs: expected directory '$d' is missing — the layout moved and this check is blind."
        echo "FAIL: scan verified nothing. Fix the path in scripts/ci/check-backend-pairs.sh."
        exit 1
    fi
done

# Every mirrored basename that exists on both sides today, spelled with basename
# because BSD find has no -printf. mod.rs is the module
# wiring, not a repository implementation.
mapfile -t all_pairs < <(
    comm -12 \
        <(find "$SQLITE_DIR" -maxdepth 1 -name '*.rs' -exec basename {} \; | sort) \
        <(find "$PG_DIR" -maxdepth 1 -name '*.rs' -exec basename {} \; | sort) \
    | grep -v '^mod\.rs$' || true
)

if [[ ${#all_pairs[@]} -eq 0 ]]; then
    echo "❌ backend-pairs: no mirrored files found under '$SQLITE_DIR' and '$PG_DIR'."
    echo "FAIL: scan verified nothing — the two backends cannot both have vanished."
    exit 1
fi

# The trait module that holds a pair's shared body. Usually repositories/<name>.rs;
# a few pairs implement a trait declared in a differently-named module, in which
# case the impl files name it in their `use crate::repositories::<module>::` import.
trait_module_for() {
    local name="$1"          # e.g. short_links.rs
    local stem="${name%.rs}"
    if [[ -f "$TRAIT_DIR/$stem.rs" ]]; then
        echo "$TRAIT_DIR/$stem.rs"
        return
    fi
    # Follow the SQLite impl's shared-module import, when it has one.
    local via
    via="$(grep -oE 'use crate::repositories::[a-z0-9_]+::' "$SQLITE_DIR/$name" 2>/dev/null \
           | head -1 | sed -E 's|use crate::repositories::([a-z0-9_]+)::|\1|' || true)"
    if [[ -n "$via" && -f "$TRAIT_DIR/$via.rs" ]]; then
        echo "$TRAIT_DIR/$via.rs"
    fi
}

# A pair is converged when the trait module declares shared SQL and both impl
# files reach for it instead of carrying statements of their own.
is_converged() {
    local name="$1"
    local trait_file
    trait_file="$(trait_module_for "$name")"
    [[ -n "$trait_file" ]] || return 1
    grep -qE '^pub\(crate\) const [A-Z0-9_]+_SQL|^macro_rules! [a-z0-9_]+_sql' "$trait_file" || return 1
    local side
    for side in "$SQLITE_DIR/$name" "$PG_DIR/$name"; do
        # An impl that still spells out its own statements has not converged,
        # whatever else it imports. Matched on statement shape rather than on
        # the bare keyword, and with comment lines dropped first, so a clause
        # named in prose or passed as a macro literal ("FOR UPDATE SKIP
        # LOCKED") is not mistaken for a second copy of the SQL.
        if sed -E 's|//.*||' "$side" \
            | grep -qiE '(INSERT[[:space:]]+(OR[[:space:]]+(REPLACE|IGNORE)[[:space:]]+)?INTO[[:space:]]+[a-z_]|SELECT[[:space:]].*[[:space:]]FROM[[:space:]]+[a-z_]|UPDATE[[:space:]]+[a-z_]+[[:space:]]+SET[[:space:]]|DELETE[[:space:]]+FROM[[:space:]]+[a-z_])'; then
            return 1
        fi
        grep -qE '_SQL|_sql!' "$side" || return 1
    done
    return 0
}

# Pairs this push adds or edits on either side.
mapfile -t changed < <(
    git diff --no-renames --name-only --diff-filter=AM "$BASE_REF"...HEAD \
        -- "$SQLITE_DIR/*.rs" "$PG_DIR/*.rs" 2>/dev/null \
    | xargs -r -n1 basename | sort -u | grep -v '^mod\.rs$' || true
)

fail=0
touched=0
for name in "${changed[@]:-}"; do
    [[ -n "$name" ]] || continue
    # Only a mirrored file is in scope; a single-backend file has no twin to drift from.
    printf '%s\n' "${all_pairs[@]}" | grep -qx "$name" || continue
    touched=$((touched + 1))
    if ! is_converged "$name"; then
        echo "❌ '$name' is written twice: $SQLITE_DIR/$name and $PG_DIR/$name each carry their own SQL."
        fail=1
    fi
done

# Standing stock: reported so an unconverted pair stays visible, never failed on.
standing=()
for name in "${all_pairs[@]}"; do
    is_converged "$name" || standing+=("$name")
done

if [[ "$fail" -ne 0 ]]; then
    echo ""
    echo "FAIL: a mirrored repository file changed while its SQL still lives in both backends."
    echo "Move the statements into crates/pierre-database/src/repositories/<name>.rs as"
    echo "\`pub(crate) const *_SQL\` with \$n placeholders, make the row extractor generic over"
    echo "\`sqlx::Row\`, and emit both impls from one \`macro_rules!\`. Worked examples:"
    echo "  repositories/short_links.rs      (no per-backend difference)"
    echo "  repositories/guardian_actions.rs (a DateTime bind that suits both)"
    echo "  repositories/resumable_turns.rs  (one backend-specific clause, as a macro literal)"
    exit 1
fi

if [[ ${#standing[@]} -gt 0 ]]; then
    echo "   backend-pairs: ${#standing[@]} of ${#all_pairs[@]} pair(s) still written twice (carnet#436):"
    printf '     %s\n' "${standing[@]}" | head -12
    if [[ ${#standing[@]} -gt 12 ]]; then
        echo "     … and $(( ${#standing[@]} - 12 )) more"
    fi
fi

echo "✅ backend-pairs: ${touched} changed pair(s) checked, none newly duplicated"
