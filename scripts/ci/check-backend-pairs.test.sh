#!/usr/bin/env bash
# ABOUTME: Pins both directions of check-backend-pairs.sh — it must fail a newly duplicated pair
# ABOUTME: and must not fail a converged one, whether the pair shares a basename or is paired by trait
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The check is diff-driven, so each case is a throwaway git repository with the
# pierre-database layout, a base commit and a HEAD commit — the same shape the
# real check sees on a push.

set -euo pipefail

CHECK="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-backend-pairs.sh"
pass=0
fail=0

scaffold() {
    local root="$1"
    mkdir -p "$root/crates/pierre-database/src/database" \
             "$root/crates/pierre-database/src/backends/postgres" \
             "$root/crates/pierre-database/src/repositories"
    git -C "$root" init -q
    git -C "$root" config user.email t@t; git -C "$root" config user.name t
    # A pair that is already converged, so the standing-stock scan has both shapes.
    cat > "$root/crates/pierre-database/src/repositories/converged.rs" <<'EOF'
pub(crate) const PICK_ONE_SQL: &str = "SELECT a FROM t WHERE id = $1";
EOF
    echo 'impl_converged_repository!(Database); // uses PICK_ONE_SQL' \
        > "$root/crates/pierre-database/src/database/converged.rs"
    echo 'impl_converged_repository!(PostgresDatabase); // uses PICK_ONE_SQL' \
        > "$root/crates/pierre-database/src/backends/postgres/converged.rs"
    git -C "$root" add -A && git -C "$root" commit -qm base
}

# run_case <name> <expected exit> [<base ref>] — the base defaults to HEAD~1,
# the shape the real check sees on a push.
run_case() {
    local name="$1" expected="$2" base="${3:-HEAD~1}" root
    root="$(mktemp -d)"
    scaffold "$root"
    "case_$name" "$root"
    git -C "$root" add -A && git -C "$root" commit -qm head
    local got=0
    ( cd "$root" && "$CHECK" "$base" >/dev/null 2>&1 ) || got=$?
    if [[ "$got" -eq "$expected" ]]; then
        echo "✅ $name (exit $got)"
        pass=$((pass + 1))
    else
        echo "❌ $name: expected exit $expected, got $got"
        fail=$((fail + 1))
    fi
    rm -rf "$root"
}

# A pair added with its SQL written out on both sides must fail.
case_new_duplicate_pair_fails() {
    local r="$1"
    cat > "$r/crates/pierre-database/src/database/dup.rs" <<'EOF'
sqlx::query("SELECT code FROM short_links WHERE code = ?1");
EOF
    cat > "$r/crates/pierre-database/src/backends/postgres/dup.rs" <<'EOF'
sqlx::query("SELECT code FROM short_links WHERE code = $1");
EOF
    echo 'pub trait DupRepository {}' > "$r/crates/pierre-database/src/repositories/dup.rs"
}

# A converged pair edited on one side must pass.
case_editing_converged_pair_passes() {
    local r="$1"
    echo '// an evergreen note' >> "$r/crates/pierre-database/src/database/converged.rs"
}

# The Postgres shell may name a backend-specific clause as a macro literal;
# that is not a second copy of the SQL (the resumable_turns case).
case_lock_clause_literal_is_not_sql() {
    local r="$1"
    cat > "$r/crates/pierre-database/src/repositories/locked.rs" <<'EOF'
pub(crate) const READ_ONE_SQL: &str = "SELECT id FROM turns WHERE id = $1";
EOF
    cat > "$r/crates/pierre-database/src/database/locked.rs" <<'EOF'
// ABOUTME: SQLite has no row locks, so the claim subquery carries no clause
impl_locked_repository!(Database, ""); // READ_ONE_SQL
EOF
    cat > "$r/crates/pierre-database/src/backends/postgres/locked.rs" <<'EOF'
// ABOUTME: the claim subquery takes FOR UPDATE SKIP LOCKED
impl_locked_repository!(PostgresDatabase, "FOR UPDATE SKIP LOCKED "); // READ_ONE_SQL
EOF
}

# A converged pair whose Postgres shell regains statements of its own must
# fail — the regression the gate exists for. Written in the crate's raw-string
# layout, one clause per line, with the shared import still in place.
case_converged_pair_regaining_sql_fails() {
    local r="$1"
    cat > "$r/crates/pierre-database/src/backends/postgres/converged.rs" <<'EOF'
use crate::repositories::converged::PICK_ONE_SQL;
impl_converged_repository!(PostgresDatabase);
impl PostgresDatabase {
    async fn count_seen(&self, user_id: &str) -> AppResult<i64> {
        sqlx::query(
            r"
            SELECT COUNT(*) AS c
            FROM user_onboarding
            WHERE user_id = $1
            ",
        );
        sqlx::query(
            r"
            UPDATE user_onboarding SET
                status = 'seen'
            WHERE user_id = $1
            ",
        );
    }
}
EOF
}

# A file touched on only one backend, with no twin, is out of scope.
case_unmirrored_file_is_ignored() {
    local r="$1"
    cat > "$r/crates/pierre-database/src/database/sqlite_only.rs" <<'EOF'
sqlx::query("SELECT 1 FROM sqlite_master WHERE name = ?1");
EOF
}


# A pair mirrored under different names per backend (users.rs / user.rs) is
# paired by the trait each side implements. Converged, an edit to one side passes.
scaffold_named_converged() {
    local r="$1"
    cat > "$r/crates/pierre-database/src/repositories/users.rs" <<'EOF'
pub(crate) const PICK_USER_SQL: &str = "SELECT id FROM users WHERE id = $1";
macro_rules! impl_user_repository {
    ($ty:ty) => {
        impl UserRepository for $ty {}
    };
}
EOF
    echo 'impl_user_repository!(Database); // PICK_USER_SQL' \
        > "$r/crates/pierre-database/src/database/users.rs"
    echo 'impl_user_repository!(PostgresDatabase); // PICK_USER_SQL' \
        > "$r/crates/pierre-database/src/backends/postgres/user.rs"
}
case_differently_named_converged_pair_passes() {
    local r="$1"
    scaffold_named_converged "$r"
    git -C "$r" add -A && git -C "$r" commit -qm named-converged
    echo '// an evergreen note' >> "$r/crates/pierre-database/src/backends/postgres/user.rs"
}

# The same pair added with its SQL written out on both sides must fail, even
# though no basename matches across the two directories.
case_differently_named_duplicate_pair_fails() {
    local r="$1"
    cat > "$r/crates/pierre-database/src/database/users.rs" <<'EOF'
impl UserRepository for Database {
    sqlx::query("SELECT id FROM users WHERE id = ?1");
}
EOF
    cat > "$r/crates/pierre-database/src/backends/postgres/user.rs" <<'EOF'
impl UserRepository for PostgresDatabase {
    sqlx::query("SELECT id FROM users WHERE id = $1");
}
EOF
    echo 'pub trait UserRepository {}' > "$r/crates/pierre-database/src/repositories/users.rs"
}

# Half converted: the Postgres side is a shell whose macro the trait module
# resolves to UserRepository, while SQLite still carries a direct impl with its
# own SQL. The two spellings must still pair, and the pair must fail.
case_differently_named_half_converted_pair_fails() {
    local r="$1"
    cat > "$r/crates/pierre-database/src/repositories/users.rs" <<'EOF'
pub(crate) const PICK_USER_SQL: &str = "SELECT id FROM users WHERE id = $1";
macro_rules! impl_user_repository {
    ($ty:ty) => {
        impl UserRepository for $ty {}
    };
}
EOF
    cat > "$r/crates/pierre-database/src/database/users.rs" <<'EOF'
impl UserRepository for Database {
    sqlx::query("SELECT id FROM users WHERE id = ?1");
}
EOF
    echo 'impl_user_repository!(PostgresDatabase); // PICK_USER_SQL' \
        > "$r/crates/pierre-database/src/backends/postgres/user.rs"
}

run_case new_duplicate_pair_fails 1
run_case editing_converged_pair_passes 0
run_case lock_clause_literal_is_not_sql 0
run_case unmirrored_file_is_ignored 0
run_case converged_pair_regaining_sql_fails 1
run_case differently_named_converged_pair_passes 0
run_case differently_named_duplicate_pair_fails 1
run_case differently_named_half_converted_pair_fails 1

# A base the diff cannot resolve — the all-zeros sha CI passes on a branch's
# first push, or a ref a fresh worktree lacks — must not read as "nothing
# touched": the tip commit is still inspected and its duplicated pair refused.
run_case new_duplicate_pair_fails 1 0000000000000000000000000000000000000000
run_case new_duplicate_pair_fails 1 no-such-ref

# Fail closed: a root commit has no base at all, so the scan cannot run.
root="$(mktemp -d)"
mkdir -p "$root/crates/pierre-database/src/database" \
         "$root/crates/pierre-database/src/backends/postgres" \
         "$root/crates/pierre-database/src/repositories"
git -C "$root" init -q
git -C "$root" config user.email t@t; git -C "$root" config user.name t
# The pair is converged, so the only reason left to exit non-zero is the
# missing base.
cat > "$root/crates/pierre-database/src/repositories/converged.rs" <<'EOF'
pub(crate) const PICK_ONE_SQL: &str = "SELECT a FROM t WHERE id = $1";
EOF
echo 'impl_converged_repository!(Database); // uses PICK_ONE_SQL' \
    > "$root/crates/pierre-database/src/database/converged.rs"
echo 'impl_converged_repository!(PostgresDatabase); // uses PICK_ONE_SQL' \
    > "$root/crates/pierre-database/src/backends/postgres/converged.rs"
git -C "$root" add -A && git -C "$root" commit -qm root
got=0; out="$( cd "$root" && "$CHECK" origin/main 2>&1 )" || got=$?
if [[ "$got" -eq 1 && "$out" == *"no base to diff against"* ]]; then
    echo "✅ root_commit_fails_closed (exit 1)"; pass=$((pass + 1))
else
    echo "❌ root_commit_fails_closed: expected exit 1, got $got"; fail=$((fail + 1))
fi
rm -rf "$root"

# Fail closed: the check must refuse to pass when the layout it scans is gone.
root="$(mktemp -d)"; scaffold "$root"
rm -rf "$root/crates/pierre-database/src/backends/postgres"
git -C "$root" add -A && git -C "$root" commit -qm head
got=0; ( cd "$root" && "$CHECK" HEAD~1 >/dev/null 2>&1 ) || got=$?
if [[ "$got" -eq 1 ]]; then
    echo "✅ missing_backend_dir_fails_closed (exit 1)"; pass=$((pass + 1))
else
    echo "❌ missing_backend_dir_fails_closed: expected exit 1, got $got"; fail=$((fail + 1))
fi
rm -rf "$root"

# The standing-stock report counts a differently-named pair that is still
# written twice, under both of its names, so it cannot hide behind the
# basename scan. The pair is committed in the base so the diff never touches it.
root="$(mktemp -d)"; scaffold "$root"
cat > "$root/crates/pierre-database/src/database/tenants.rs" <<'EOF'
impl TenantRepository for Database {
    sqlx::query("SELECT id FROM tenants WHERE id = ?1");
}
EOF
cat > "$root/crates/pierre-database/src/backends/postgres/tenant.rs" <<'EOF'
impl TenantRepository for PostgresDatabase {
    sqlx::query("SELECT id FROM tenants WHERE id = $1");
}
EOF
echo 'pub trait TenantRepository {}' > "$root/crates/pierre-database/src/repositories/tenants.rs"
git -C "$root" add -A && git -C "$root" commit -qm named-standing
echo '// an evergreen note' >> "$root/crates/pierre-database/src/database/converged.rs"
git -C "$root" add -A && git -C "$root" commit -qm head
got=0; out="$( cd "$root" && "$CHECK" HEAD~1 2>&1 )" || got=$?
if [[ "$got" -eq 0 && "$out" == *"1 of 2 pair(s) still written twice"* && "$out" == *"tenants.rs ↔ tenant.rs"* ]]; then
    echo "✅ differently_named_pair_is_standing_stock (exit 0)"; pass=$((pass + 1))
else
    echo "❌ differently_named_pair_is_standing_stock: expected exit 0 naming 'tenants.rs ↔ tenant.rs' among 2 pairs, got $got:"; printf '%s\n' "$out"; fail=$((fail + 1))
fi
rm -rf "$root"

echo ""
echo "backend-pairs check: $pass passed, $fail failed"
[[ "$fail" -eq 0 ]]
