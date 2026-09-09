#!/usr/bin/env bash
# ABOUTME: Fixture test for check-twin-surfaces.sh — what it refuses and what it must leave alone
# ABOUTME: Pins both catch shapes, three no-false-positive shapes, the marker, and two fail-closed scans
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# A gate nobody has seen fail is a gate nobody knows works — the lockbud job
# reported green for nine months without once analysing this codebase
# (carnet#399). So every branch is fired against a fixture tree here.
#
# Two things about this gate make a fixture test load-bearing rather than
# decorative, and both were found by writing it:
#
#   1. The gate anchors on ITS OWN location, not the working directory:
#      `$SCRIPT_DIR/../../crates` wins over `git rev-parse --show-toplevel`.
#      So a fixture must HOST a copy of the script at <root>/scripts/ci/,
#      or the run silently scans the real repo and the assertion means nothing.
#      A first draft of this test did exactly that and "passed".
#   2. Bare mode NEVER fails — it reports the standing stock and exits 0. Only
#      the diff-scoped mode fails. A test that ran bare mode and asserted a
#      non-zero exit would be pinning a contract the gate does not have.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/check-twin-surfaces.sh}"

failures=0
pass() { echo "  ✅ $1"; }
fail() {
    echo "  ❌ $1"
    failures=$((failures + 1))
}

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

git_q() { git -c user.email=t@t -c user.name=t -c commit.gpgsign=false "$@"; }

# tree <name> — a minimal two-surface workspace hosting the gate under test.
# Both route crates declare a `.route(` so they are route-serving, and both
# handlers call the same pierre-services fn so R1 pairs them; their extractor
# shares a name so R2 pairs them too.
tree() {
    local root="$TMP/$1"
    mkdir -p "$root/scripts/ci" \
             "$root/crates/pierre-services/src" \
             "$root/crates/routes-bear/src" \
             "$root/crates/routes-cook/src"
    cp "$UNDER_TEST" "$root/scripts/ci/check-twin-surfaces.sh"
    printf 'pub mod pre_approval;\n' > "$root/crates/pierre-services/src/lib.rs"
    printf 'pub async fn allow() {}\n' > "$root/crates/pierre-services/src/pre_approval.rs"
    ( cd "$root" && git init -q . )
    echo "$root"
}

# surface <root> <crate> <route> <marker-or-empty> <field-lines>
surface() {
    local root="$1" crate="$2" route="$3" marker="$4" fields="$5"
    {
        [[ -n "$marker" ]] && printf '%s\n' "$marker"
        printf 'pub struct AllowReq {\n%s\n}\n' "$fields"
        printf 'pub fn routes() -> Router { Router::new().route("%s", post(handle)) }\n' "$route"
        printf 'pub async fn handle(Json(request): Json<AllowReq>) { pre_approval::allow().await; }\n'
    } > "$root/crates/$crate/src/lib.rs"
}

commit() { ( cd "$1" && git add -A && git_q commit -q -m "$2" ); }

# expect <label> <root> <want-exit> [base-ref]
expect() {
    local label="$1" root="$2" want="$3" base="${4:-}" got=0
    if [[ -n "$base" ]]; then
        ( cd "$root" && bash scripts/ci/check-twin-surfaces.sh "$base" ) >/dev/null 2>&1 || got=$?
    else
        ( cd "$root" && bash scripts/ci/check-twin-surfaces.sh ) >/dev/null 2>&1 || got=$?
    fi
    if [[ "$got" == "$want" ]]; then
        pass "$label (exit $got)"
    else
        fail "$label — wanted exit $want, got $got"
    fi
}

BEAR_TWO='    pub email: String,
    pub send_invite: bool,'
BEAR_ONE='    pub email: String,'

echo "==== check-twin-surfaces.sh fixture test ===="

# ---------------------------------------------------------------------------
# CATCH 1 — the diff DROPS a field from one twin while the other keeps it
# ---------------------------------------------------------------------------
root="$(tree drop)"
surface "$root" routes-bear /api/a "" "$BEAR_TWO"
surface "$root" routes-cook /api/b "" "$BEAR_TWO"
commit "$root" symmetric
surface "$root" routes-cook /api/b "" "$BEAR_ONE"
commit "$root" "drop the field from one twin"
expect "a diff that drops a field from one twin fails" "$root" 1 HEAD~

# ---------------------------------------------------------------------------
# CATCH 2 — the diff ADDS a field to one twin only
# ---------------------------------------------------------------------------
root="$(tree add)"
surface "$root" routes-bear /api/a "" "$BEAR_ONE"
surface "$root" routes-cook /api/b "" "$BEAR_ONE"
commit "$root" symmetric
surface "$root" routes-bear /api/a "" "$BEAR_TWO"
commit "$root" "add the field to one twin"
expect "a diff that adds a field to one twin only fails" "$root" 1 HEAD~

# ---------------------------------------------------------------------------
# NO FALSE POSITIVE — a symmetric change to BOTH twins
# ---------------------------------------------------------------------------
root="$(tree both)"
surface "$root" routes-bear /api/a "" "$BEAR_ONE"
surface "$root" routes-cook /api/b "" "$BEAR_ONE"
commit "$root" symmetric
surface "$root" routes-bear /api/a "" "$BEAR_TWO"
surface "$root" routes-cook /api/b "" "$BEAR_TWO"
commit "$root" "add the field to both twins"
expect "a field added to BOTH twins passes" "$root" 0 HEAD~

# ---------------------------------------------------------------------------
# NO FALSE POSITIVE — a registered gap, and it must name the field
# ---------------------------------------------------------------------------
root="$(tree marker)"
surface "$root" routes-bear /api/a "" "$BEAR_ONE"
surface "$root" routes-cook /api/b "" "$BEAR_ONE"
commit "$root" symmetric
surface "$root" routes-bear /api/a "" "$BEAR_TWO"
surface "$root" routes-cook /api/b \
    '/// LIMITATION(registre#999): send_invite is absent here on purpose.' "$BEAR_ONE"
commit "$root" "diverge, and register it"
expect "a LIMITATION marker naming the field passes" "$root" 0 HEAD~
# Captured to a variable, not piped: the gate's exit status and the text search
# are separate questions, and a pipeline conflates them.
marker_out="$( cd "$root" && bash scripts/ci/check-twin-surfaces.sh HEAD~ 2>&1 || true )"
if printf '%s' "$marker_out" | grep -q 'registre#999'; then
    pass "the run prints the registered issue number rather than hiding the gap"
else
    fail "a registered gap must still be printed with its issue number"
    printf '%s\n' "$marker_out" | sed 's/^/       /'
fi

# A marker that does NOT name the diverging field must not silence it — one
# marker cannot be a blanket exemption for its file.
root="$(tree marker_wrong_field)"
surface "$root" routes-bear /api/a "" "$BEAR_ONE"
surface "$root" routes-cook /api/b "" "$BEAR_ONE"
commit "$root" symmetric
surface "$root" routes-bear /api/a "" "$BEAR_TWO"
surface "$root" routes-cook /api/b \
    '/// LIMITATION(registre#999): some_other_field is absent here on purpose.' "$BEAR_ONE"
commit "$root" "diverge, and register the wrong field"
expect "a marker naming a DIFFERENT field does not silence the divergence" "$root" 1 HEAD~

# ---------------------------------------------------------------------------
# NO FALSE POSITIVE — bare mode never fails, it reports
# ---------------------------------------------------------------------------
root="$(tree bare)"
surface "$root" routes-bear /api/a "" "$BEAR_TWO"
surface "$root" routes-cook /api/b "" "$BEAR_ONE"
commit "$root" "standing divergence"
expect "bare mode reports the standing stock and exits 0" "$root" 0

# ---------------------------------------------------------------------------
# FAIL CLOSED — a scan that verified nothing must not report success
# ---------------------------------------------------------------------------
root="$TMP/no_services"
mkdir -p "$root/scripts/ci" "$root/crates/routes-bear/src"
cp "$UNDER_TEST" "$root/scripts/ci/check-twin-surfaces.sh"
printf 'pub fn routes() -> Router { Router::new().route("/api/a", post(handle)) }\n' \
    > "$root/crates/routes-bear/src/lib.rs"
( cd "$root" && git init -q . )
expect "a tree with no pierre-services fails closed" "$root" 1

root="$(tree lonely)"
surface "$root" routes-bear /api/a "" "$BEAR_TWO"
rm -rf "$root/crates/routes-cook"
commit "$root" "one surface only"
expect "a tree with no comparable twin pair fails closed" "$root" 1

echo ""
if [[ "$failures" -eq 0 ]]; then
    echo "✅ check-twin-surfaces.sh: every branch fired as specified"
    exit 0
fi
echo "❌ check-twin-surfaces.sh: $failures assertion(s) failed"
exit 1
