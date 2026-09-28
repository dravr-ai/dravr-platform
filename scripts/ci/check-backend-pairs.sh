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
#   1. Find every mirrored pair the diff adds or edits on either side. A pair
#      is two files with the same basename, one per backend, or two files
#      implementing the same repository trait under different names (the
#      SQLite users.rs against the Postgres user.rs); a Postgres file that
#      implements three traits is in three pairs.
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

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# One resolution rule for every diff-scoped gate: an explicit base, else
# $GATE_BASE_REF, else origin/main — and HEAD~1 whenever that base is missing or
# is HEAD itself. CI passes `github.event.before`, which is all-zeros on a
# branch's first push and unreachable after a force-push beyond the fetch depth;
# locally, a fresh worktree may have no origin/main at all. Handed straight to
# `git diff`, each of those fails the diff, and a swallowed failure reads as
# "nothing touched" — the disarmed shape the house gate rule forbids.
# shellcheck source=scripts/ci/gate-base-ref.sh
. "$SCRIPT_DIR/gate-base-ref.sh"

if ! BASE_REF="$(resolve_gate_base_ref "${1:-}")"; then
    echo "❌ backend-pairs: HEAD is a root commit — there is no base to diff against."
    echo "FAIL: scan verified nothing."
    exit 1
fi

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
# A pair is spelled "<sqlite basename>|<postgres basename>" throughout, so a
# mirrored file and a differently-named one go through the same checks.
pair_label() {
    local sqlite_name="${1%%|*}" pg_name="${1##*|}"
    if [[ "$sqlite_name" == "$pg_name" ]]; then
        echo "$sqlite_name"
    else
        echo "$sqlite_name ↔ $pg_name"
    fi
}

# Every mirrored basename that exists on both sides today, spelled with basename
# because BSD find has no -printf. mod.rs is the module
# wiring, not a repository implementation.
#
# A SQLite file one level down (database/repositories/agents_assignments.rs)
# still pairs with its Postgres namesake: the pair is spelled with the path
# relative to each side's directory, and a file moved into a subdirectory stays
# in scope.
mapfile -t basename_pairs < <(
    join -t "$(printf '\t')" -j 1 \
        <(find "$SQLITE_DIR" -name '*.rs' | sed "s|^$SQLITE_DIR/||" \
            | awk '{ n = $0; sub(/.*\//, "", n); print n "\t" $0 }' | sort) \
        <(find "$PG_DIR" -name '*.rs' | sed "s|^$PG_DIR/||" \
            | awk '{ n = $0; sub(/.*\//, "", n); print n "\t" $0 }' | sort) \
    | awk -F '\t' '$1 != "mod.rs" { print $2 "|" $3 }' | sort -u || true
)

if [[ ${#basename_pairs[@]} -eq 0 ]]; then
    echo "❌ backend-pairs: no mirrored files found under '$SQLITE_DIR' and '$PG_DIR'."
    echo "FAIL: scan verified nothing — the two backends cannot both have vanished."
    exit 1
fi

# The trait an impl file carries, whichever way it spells it: a direct
# `impl XRepository for Database` / `for PostgresDatabase`, or the shared macro
# that emits that impl (`impl_x_repository!(Database)`). A macro is resolved
# to the trait its module emits (`impl XRepository for $ty` beside the
# `macro_rules!`), so a shell on one side still pairs with a direct impl on the
# other — the half-converted shape is exactly the one to catch. A macro whose
# module names no trait keys on the macro name itself, which both shells share.
# Six repositories are mirrored under different names per backend
# (users.rs/user.rs, api_keys.rs/api_key.rs, …); this is how the gate sees them.
trait_keys_of() {
    local file="$1" stripped macro module
    stripped="$(sed -E 's|//.*||' "$file")"
    grep -oE 'impl [A-Za-z0-9]+Repository for (Database|PostgresDatabase)\b' <<< "$stripped" \
        | awk '{print $2}' || true
    while read -r macro; do
        [[ -n "$macro" ]] || continue
        module="$(grep -lE "^macro_rules! ${macro}( |\{|$)" "$TRAIT_DIR"/*.rs 2>/dev/null | head -1 || true)"
        if [[ -n "$module" ]] && grep -qE "impl [A-Za-z0-9]+Repository for \\\$[a-z_]+" "$module"; then
            grep -oE "impl [A-Za-z0-9]+Repository for \\\$[a-z_]+" "$module" | awk '{print $2}'
        else
            echo "macro:$macro"
        fi
    done < <(grep -oE '\bimpl_[a-z0-9_]+!\(' <<< "$stripped" | sed -E 's/!\($//' | sort -u)
}

# "<key>\t<path relative to the side's directory>" for every impl file on one
# side, at any depth: the SQLite agents impl lives in database/repositories/,
# one level below the directory the basename scan reads, and an impl moved
# into a subdirectory must not fall out of the scan by moving.
keyed_files() {
    local dir="$1" f name key
    while IFS= read -r f; do
        name="${f#"$dir"/}"
        [[ "$(basename "$name")" != "mod.rs" ]] || continue
        while read -r key; do
            [[ -n "$key" ]] || continue
            printf '%s\t%s\n' "$key" "$name"
        done < <(trait_keys_of "$f")
    done < <(find "$dir" -name '*.rs' | sort) | sort -u
}

# Trait-keyed pairs whose two files are NOT the same basename; a same-named
# pair is already in basename_pairs. One Postgres file that implements three
# traits pairs with three SQLite files, and each of those is its own pair:
# editing that one file puts all three in scope, because the file holds all
# three copies.
mapfile -t trait_pairs < <(
    join -t "$(printf '\t')" -j 1 \
        <(keyed_files "$SQLITE_DIR") \
        <(keyed_files "$PG_DIR") \
    | awk -F '\t' '{ a = $2; b = $3; sub(/.*\//, "", a); sub(/.*\//, "", b); if (a != b) print $2 "|" $3 }' | sort -u || true
)

all_pairs=("${basename_pairs[@]}" "${trait_pairs[@]}")

# The trait module that holds a pair's shared body. Usually repositories/<name>.rs
# for one of the pair's two names; otherwise the impl files name it in their
# `use crate::repositories::<module>::` import.
trait_module_for() {
    local pair="$1" name stem via
    for name in "${pair%%|*}" "${pair##*|}"; do
        stem="${name%.rs}"
        if [[ -f "$TRAIT_DIR/$stem.rs" ]]; then
            echo "$TRAIT_DIR/$stem.rs"
            return
        fi
    done
    for name in "$SQLITE_DIR/${pair%%|*}" "$PG_DIR/${pair##*|}"; do
        via="$(grep -oE 'use crate::repositories::[a-z0-9_]+::' "$name" 2>/dev/null \
               | head -1 | sed -E 's|use crate::repositories::([a-z0-9_]+)::|\1|' || true)"
        if [[ -n "$via" && -f "$TRAIT_DIR/$via.rs" ]]; then
            echo "$TRAIT_DIR/$via.rs"
            return
        fi
    done
}

# Whether a file spells out SQL statements of its own. Matched on statement
# shape rather than on the bare keyword, and with comments dropped first, so a
# clause named in prose or passed as a macro literal ("FOR UPDATE SKIP LOCKED")
# is not mistaken for a copy of the SQL. Two shapes: a statement inline in a
# string literal, and the raw-string layout the crate writes its SQL in, where
# each clause opens its own line — `SELECT` with its `FROM` on the next line,
# `UPDATE t SET` closing a line. That second shape is matched on a statement
# keyword at line start, which Rust source never puts there outside a SQL
# literal. The file is read into a variable first: with pipefail, `grep -q`
# closing its end of a pipe early would surface as SIGPIPE from sed and the
# match would be lost.
carries_sql() {
    local stripped
    stripped="$(sed -E 's|//.*||' "$1")"
    grep -qiE '(INSERT[[:space:]]+(OR[[:space:]]+(REPLACE|IGNORE)[[:space:]]+)?INTO[[:space:]]+[a-z_]|SELECT[[:space:]].*[[:space:]]FROM[[:space:]]+[a-z_]|UPDATE[[:space:]]+[a-z_]+[[:space:]]+SET[[:space:]]|DELETE[[:space:]]+FROM[[:space:]]+[a-z_])' <<< "$stripped" \
        || grep -qiE '^[[:space:]]*(SELECT|INSERT[[:space:]]+(OR[[:space:]]+(REPLACE|IGNORE)[[:space:]]+)?INTO|UPDATE[[:space:]]+[a-z_]+|DELETE[[:space:]]+FROM)([[:space:]]|$)' <<< "$stripped"
}

# A pair is converged when the trait module declares shared SQL and both impl
# files reach for it instead of carrying statements of their own.
is_converged() {
    local pair="$1"
    local trait_file
    trait_file="$(trait_module_for "$pair")"
    [[ -n "$trait_file" ]] || return 1
    grep -qE '^pub\(crate\) const [A-Z0-9_]+_SQL|^macro_rules! [a-z0-9_]+_sql' "$trait_file" || return 1
    local side stripped
    for side in "$SQLITE_DIR/${pair%%|*}" "$PG_DIR/${pair##*|}"; do
        # An impl that still spells out its own statements has not converged,
        # whatever else it imports. Matched on statement shape rather than on
        # the bare keyword, and with comments dropped first, so a clause
        # named in prose or passed as a macro literal ("FOR UPDATE SKIP
        # LOCKED") is not mistaken for a second copy of the SQL. Two shapes:
        # a statement inline in a string literal, and the raw-string layout
        # the crate writes its SQL in, where each clause opens its own line —
        # `SELECT` with its `FROM` on the next line, `UPDATE t SET` closing a
        # line. That second shape is matched on a statement keyword at line
        # start, which Rust source never puts there outside a SQL literal.
        # The file is read into a variable first: with pipefail, `grep -q`
        # closing its end of a pipe early would surface as SIGPIPE from sed
        # and the match would be lost.
        if carries_sql "$side"; then
            return 1
        fi
        grep -qE '_SQL|_sql!' "$side" || return 1
    done
    return 0
}

# Pairs this push adds or edits on either side. The diff's exit status is
# checked on its own: a failed diff would otherwise leave `changed` empty and
# the success marker would print over a push the scan never looked at.
if ! changed_paths="$(git diff --no-renames --name-only --diff-filter=AM "$BASE_REF"...HEAD \
        -- "$SQLITE_DIR/*.rs" "$PG_DIR/*.rs")"; then
    echo "❌ backend-pairs: git diff against '$BASE_REF' failed."
    echo "FAIL: scan verified nothing."
    exit 1
fi
mapfile -t changed_sqlite < <(
    printf '%s\n' "$changed_paths" | grep "^$SQLITE_DIR/" | sed "s|^$SQLITE_DIR/||" | sort -u | grep -v '^mod\.rs$' || true
)
mapfile -t changed_pg < <(
    printf '%s\n' "$changed_paths" | grep "^$PG_DIR/" | sed "s|^$PG_DIR/||" | sort -u | grep -v '^mod\.rs$' || true
)

pair_touched() {
    local pair="$1"
    printf '%s\n' "${changed_sqlite[@]:-}" | grep -qx "${pair%%|*}" && return 0
    printf '%s\n' "${changed_pg[@]:-}" | grep -qx "${pair##*|}"
}

fail=0
touched=0
for pair in "${all_pairs[@]}"; do
    # Only a mirrored file is in scope; a single-backend file has no twin to drift from.
    pair_touched "$pair" || continue
    touched=$((touched + 1))
    if ! is_converged "$pair"; then
        echo "❌ '$(pair_label "$pair")' is written twice: $SQLITE_DIR/${pair%%|*} and $PG_DIR/${pair##*|} each carry their own SQL."
        fail=1
    fi
done

# Standing stock: reported so an unconverted pair stays visible, never failed on.
standing=()
for pair in "${all_pairs[@]}"; do
    is_converged "$pair" || standing+=("$(pair_label "$pair")")
done

# Pairs outside pierre-database: a repository trait implemented once over a
# SQLite pool and once over a Postgres pool somewhere else in the workspace
# (pierre-server's AdminConfigRepository is the standing case). The rule is the
# one above: a pair the push touches must not carry SQL on both sides. The scan
# covers every crate, because the backend directories above are only where the
# convention puts a pair, not where one can be written.
backend_of() {
    if grep -qE 'SqlitePool|Pool<Sqlite>|sqlx::sqlite' "$1"; then
        echo sqlite
    elif grep -qE 'PgPool|Pool<Postgres>|sqlx::postgres' "$1"; then
        echo pg
    fi
}
if ! all_changed="$(git diff --no-renames --name-only --diff-filter=AM "$BASE_REF"...HEAD -- 'crates/*.rs')"; then
    echo "❌ backend-pairs: git diff against '$BASE_REF' failed."
    echo "FAIL: scan verified nothing."
    exit 1
fi
mapfile -t crate_impls < <(
    grep -rlE --include='*.rs' 'impl [A-Za-z0-9]+Repository for [A-Za-z0-9_]+' crates/*/src 2>/dev/null \
        | grep -v "^$DB_ROOT/" | sort || true
)
crate_keyed=()
for f in "${crate_impls[@]}"; do
    backend="$(backend_of "$f")"
    [[ -n "$backend" ]] || continue
    while read -r trait; do
        [[ -n "$trait" ]] && crate_keyed+=("$trait"$'\t'"$backend"$'\t'"$f")
    done < <(sed -E 's|//.*||' "$f" | grep -oE 'impl [A-Za-z0-9]+Repository for' | awk '{print $2}' | sort -u)
done
mapfile -t crate_pairs < <(
    join -t "$(printf '\t')" -j 1 \
        <(printf '%s\n' "${crate_keyed[@]:-}" | awk -F '\t' '$2 == "sqlite" { print $1 "\t" $3 }' | sort) \
        <(printf '%s\n' "${crate_keyed[@]:-}" | awk -F '\t' '$2 == "pg" { print $1 "\t" $3 }' | sort) \
    | awk -F '\t' '{ print $2 "|" $3 }' | sort -u || true
)
for pair in "${crate_pairs[@]}"; do
    [[ -n "$pair" ]] || continue
    if carries_sql "${pair%%|*}" && carries_sql "${pair##*|}"; then
        if printf '%s\n' "$all_changed" | grep -qxF -e "${pair%%|*}" -e "${pair##*|}"; then
            echo "❌ '${pair%%|*} ↔ ${pair##*|}' is written twice: each side carries its own SQL."
            fail=1
        else
            standing+=("${pair%%|*} ↔ ${pair##*|}")
        fi
    fi
done

# A backend chosen at query time: a match on the factory's `Database` variants
# whose arms run their own SQL is the same duplication with no file boundary
# to pair on (claim_verdict_backfill.rs was the standing case). Whole-tree and
# fatal — the statement belongs in a repository written once. The factory's
# own dispatch and the test-database factory are exempt: neither holds a
# statement a production read runs.
mapfile -t query_time_splits < <(
    grep -rlE --include='*.rs' 'Database::(SQLite|PostgreSQL)\(' crates/*/src 2>/dev/null \
        | grep -v "^$DB_ROOT/backends/factory/" | grep -vx "$DB_ROOT/database/test_utils.rs" | sort || true
)
for f in "${query_time_splits[@]}"; do
    [[ -n "$f" ]] || continue
    if grep -q 'sqlx::query' "$f" && carries_sql "$f"; then
        echo "❌ '$f' picks a backend at query time and runs its own SQL in the arms."
        fail=1
    fi
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
    printf '     %s\n' "${standing[@]}" | head -20
    if [[ ${#standing[@]} -gt 20 ]]; then
        echo "     … and $(( ${#standing[@]} - 20 )) more"
    fi
fi

echo "✅ backend-pairs: ${touched} changed pair(s) checked, none newly duplicated"
