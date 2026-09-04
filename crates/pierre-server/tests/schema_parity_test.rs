// ABOUTME: Enforces SQLite<->PostgreSQL schema parity so a table/column added to
// ABOUTME: one migration tree but not the other cannot ship silently (P2-13).
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Schema parity guard
//!
//! `migrations/` (SQLite) and `migrations_pg/` (PostgreSQL) are driven by two
//! independent `sqlx::migrate!` macros. Nothing else checks that they describe
//! equivalent schemas, so drift between backends is a standing risk: a column
//! added to one tree but not the other ships silently and only surfaces as a
//! runtime "no such column / relation does not exist" on whichever backend was
//! forgotten.
//!
//! This file installs a real guard:
//!
//! - [`columns_match_on_shared_tables`] applies *both* trees to fresh databases
//!   and asserts that, for every table present in **both** backends, the set of
//!   column names matches. Types are intentionally **not** compared — SQLite and
//!   PostgreSQL spell the same logical type differently (`TEXT` vs `VARCHAR`,
//!   `INTEGER` vs `BIGINT`, `BOOLEAN` storage, etc.); only column *presence* is
//!   portable and meaningful. The SQLite side runs always (in-memory); the
//!   PostgreSQL side runs only when a Postgres `DATABASE_URL`/`TEST_DATABASE_URL`
//!   is available and the `postgresql` feature is compiled in, so the test runs
//!   for real in `ci-postgres` and skips cleanly in the SQLite-only local lane.
//!
//! - [`whole_table_sets_have_no_unexpected_divergence`] is a portable, always-on
//!   fallback that replays the table-level DDL of both directories — `CREATE
//!   TABLE`, `ALTER TABLE … RENAME TO`, `DROP TABLE`, in migration order — and
//!   fails on any table that ends up in one tree and not the other. It catches
//!   the most common drift, a whole table/feature wired into a single backend,
//!   in every CI lane, even ones with no Postgres. Replaying renames and drops
//!   rather than only collecting `CREATE` is what makes the comparison about
//!   the *final* schema: SQLite's 12-step table rebuild creates a scratch table
//!   and renames it over the original, and a table created in 2025 and dropped
//!   in 2026 is not part of either schema.
//!
//! The trees do diverge in three places on purpose, and those three are pinned
//! by name in [`PG_ONLY_TABLES`] with the reason for each. The assertion is an
//! equality, not a subset: a new one-sided table fails the test, and so does
//! resolving one of the pinned three without updating the constant.

// `doc_markdown` would force backticks around every bare SQLite/PostgreSQL/SQLX
// mention in this file's prose, which hurts readability of the parity rationale.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::{create_sqlite_test_db, create_test_db};
use sqlx::Row;

/// Repo-root-relative path to the SQLite migration tree.
const SQLITE_MIGRATIONS: &str = "../../migrations";
/// Repo-root-relative path to the PostgreSQL migration tree.
const PG_MIGRATIONS: &str = "../../migrations_pg";

/// `_sqlx_migrations` is sqlx bookkeeping, not part of the application schema.
const SQLX_BOOKKEEPING_TABLE: &str = "_sqlx_migrations";

/// Tables the PostgreSQL tree ends up with and the SQLite tree does not.
///
/// A pinned fact, not an exception list: the test asserts the computed
/// PostgreSQL-only set *equals* this one, so a fourth one-sided table fails, and
/// so does removing one of these three without editing the constant. Sorted,
/// because a `BTreeSet` difference is.
///
/// - `authorization_codes` — the PostgreSQL OAuth2 authorization-code store,
///   read by `pierre_database::backends::postgres::oauth`. Both trees also
///   create `oauth2_auth_codes`; only PostgreSQL carries this second table.
/// - `coaches_orphaned` — quarantine target of the PostgreSQL-only migration
///   that converts `coaches.tenant_id` to a UUID foreign key. Rows whose
///   `tenant_id` is not a UUID are moved here instead of deleted. SQLite runs
///   no such conversion and has nothing to quarantine.
/// - `tenant_provider_usage` — per-tenant, per-provider request/error counters,
///   created only in the PostgreSQL tree.
const PG_ONLY_TABLES: [&str; 3] = [
    "authorization_codes",
    "coaches_orphaned",
    "tenant_provider_usage",
];

/// Tables the SQLite tree ends up with and the PostgreSQL tree does not.
///
/// Empty: every table the SQLite tree leaves behind has a PostgreSQL
/// counterpart. Pinned the same way as [`PG_ONLY_TABLES`] — the first
/// SQLite-only table to appear fails the test.
const SQLITE_ONLY_TABLES: [&str; 0] = [];

/// A table-level DDL statement that changes which tables a tree ends up with.
enum TableOp {
    Create(String),
    Rename { from: String, to: String },
    Drop(String),
}

/// Replay the table-level DDL of every `.sql` file in `dir`, in migration order,
/// and return the table names the tree ends up with, lowercased.
///
/// Migration files are visited in filename order, which is timestamp order, so a
/// later file's `DROP`/`RENAME` sees what an earlier file created. That is what
/// makes the result the *final* schema rather than a union of every name ever
/// written: SQLite's 12-step rebuild creates `foo_new` and renames it over
/// `foo`, and a table created in one migration and dropped in a later one
/// belongs to neither backend's schema.
fn final_table_names(dir: &str) -> BTreeSet<String> {
    let read_dir = fs::read_dir(Path::new(dir))
        .unwrap_or_else(|e| panic!("cannot read migration dir {dir}: {e}"));
    let mut paths: Vec<PathBuf> = read_dir
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("sql"))
        .collect();
    paths.sort();

    let mut tables = BTreeSet::new();
    for path in paths {
        let sql = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        for op in parse_table_ops(&sql) {
            match op {
                TableOp::Create(name) => {
                    tables.insert(name);
                }
                TableOp::Rename { from, to } => {
                    tables.remove(&from);
                    tables.insert(to);
                }
                TableOp::Drop(name) => {
                    tables.remove(&name);
                }
            }
        }
    }
    tables
}

/// Extract the table-level DDL statements from `sql`, in source order.
///
/// Deliberately tolerant: handles optional `IF NOT EXISTS` / `IF EXISTS`,
/// optional quoting, and ignores statements appearing inside line comments.
/// Matching runs against an uppercased copy of the line and the extracted
/// identifier is lowercased on the way out, so no byte offset is ever taken
/// against the original text.
fn parse_table_ops(sql: &str) -> Vec<TableOp> {
    let mut ops = Vec::new();
    for raw_line in sql.lines() {
        let line = raw_line.trim();
        if line.starts_with("--") {
            continue;
        }
        let upper = line.to_uppercase();

        if let Some(rest) = keyword_tail(&upper, "CREATE TABLE ") {
            let rest = rest
                .strip_prefix("IF NOT EXISTS ")
                .unwrap_or(rest)
                .trim_start();
            if let Some(name) = leading_identifier(rest) {
                ops.push(TableOp::Create(name));
            }
        } else if let Some(rest) = keyword_tail(&upper, "ALTER TABLE ") {
            if let Some(idx) = rest.find(" RENAME TO ") {
                let from = leading_identifier(rest);
                let to = leading_identifier(rest[idx + " RENAME TO ".len()..].trim_start());
                if let (Some(from), Some(to)) = (from, to) {
                    ops.push(TableOp::Rename { from, to });
                }
            }
        } else if let Some(rest) = keyword_tail(&upper, "DROP TABLE ") {
            let rest = rest.strip_prefix("IF EXISTS ").unwrap_or(rest).trim_start();
            if let Some(name) = leading_identifier(rest) {
                ops.push(TableOp::Drop(name));
            }
        }
    }
    ops
}

/// What follows the first occurrence of `keyword` in `text`, left-trimmed.
///
/// `keyword` is ASCII, so the offset `find` returns plus its length is always a
/// character boundary.
fn keyword_tail<'a>(text: &'a str, keyword: &str) -> Option<&'a str> {
    text.find(keyword)
        .map(|idx| text[idx + keyword.len()..].trim_start())
}

/// The identifier `text` opens with, unquoted and lowercased.
fn leading_identifier(text: &str) -> Option<String> {
    let token: String = text
        .trim_start_matches(['"', '`'])
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if token.is_empty() {
        None
    } else {
        Some(token.to_lowercase())
    }
}

/// Read the applied table -> {column names} map from a live SQLite pool.
async fn sqlite_schema(pool: &sqlx::Pool<sqlx::Sqlite>) -> BTreeMap<String, BTreeSet<String>> {
    let table_rows =
        sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .fetch_all(pool)
            .await
            .unwrap();
    let mut schema = BTreeMap::new();
    for row in table_rows {
        let table: String = row.get("name");
        let lower = table.to_lowercase();
        if lower.starts_with("sqlite_") || lower == SQLX_BOOKKEEPING_TABLE {
            continue;
        }
        // PRAGMA cannot bind params; the table name comes from sqlite_master,
        // not from user input, so interpolation here is safe.
        let col_rows = sqlx::query(&format!("PRAGMA table_info(\"{table}\")"))
            .fetch_all(pool)
            .await
            .unwrap();
        let cols = col_rows
            .into_iter()
            .map(|r| {
                let c: String = r.get("name");
                c.to_lowercase()
            })
            .collect::<BTreeSet<String>>();
        schema.insert(lower, cols);
    }
    schema
}

#[tokio::test]
async fn whole_table_sets_have_no_unexpected_divergence() {
    // P2-13 — portable, always-on guard. Replays both trees' table-level DDL and
    // fails on any table wired into only one backend beyond the pinned three.
    // Runs in every CI lane (no Postgres required), catching the most common
    // drift: a whole table/feature added to a single backend.
    let sqlite_tables = final_table_names(SQLITE_MIGRATIONS);
    let pg_tables = final_table_names(PG_MIGRATIONS);

    // Both trees must describe a meaningful, overlapping schema — a parse/path
    // regression that found zero or near-zero shared tables would otherwise let
    // the guard pass vacuously.
    let shared = sqlite_tables.intersection(&pg_tables).count();
    assert!(
        shared > 50,
        "expected the two trees to share most tables; only {shared} in common \
         (parser or path regression?)"
    );

    let sqlite_only: Vec<&str> = sqlite_tables
        .difference(&pg_tables)
        .map(String::as_str)
        .collect();
    let pg_only: Vec<&str> = pg_tables
        .difference(&sqlite_tables)
        .map(String::as_str)
        .collect();

    assert_eq!(
        sqlite_only, SQLITE_ONLY_TABLES,
        "SQLite-only tables changed. A table created in `migrations/` with no \
         counterpart in `migrations_pg/` ships as a runtime \"relation does not \
         exist\" on PostgreSQL: add it to the PostgreSQL tree. If the divergence \
         is deliberate, document it in SQLITE_ONLY_TABLES with its reason."
    );
    assert_eq!(
        pg_only, PG_ONLY_TABLES,
        "PostgreSQL-only tables changed. A table created in `migrations_pg/` \
         with no counterpart in `migrations/` ships as a runtime \"no such \
         table\" on SQLite: add it to the SQLite tree. If the divergence is \
         deliberate, document it in PG_ONLY_TABLES with its reason; if one of \
         the pinned three was resolved, drop it from that constant."
    );
}

#[tokio::test]
async fn columns_match_on_shared_tables() {
    // P2-13 — applied-schema guard. The SQLite side always runs in-memory —
    // this is a comparison of the two dialects, so it opens SQLite explicitly
    // whatever `DATABASE_URL` names. The PostgreSQL side is whatever the
    // factory opens: real PostgreSQL in ci-postgres, and a clean skip in the
    // SQLite-only local lane.
    let db = create_sqlite_test_db().await.unwrap();
    let sqlite = match &db {
        Database::SQLite(sqlite_db) => sqlite_schema(sqlite_db.pool()).await,
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(_) => panic!("create_sqlite_test_db yields SQLite"),
    };
    assert!(
        sqlite.len() > 50,
        "applied SQLite schema should have many tables; got {}",
        sqlite.len()
    );

    let Some(pg) = applied_pg_schema().await else {
        eprintln!(
            "[schema-parity] DATABASE_URL names no PostgreSQL server (or the `postgresql` \
             feature is off) — SQLite-only run, skipping cross-backend column comparison"
        );
        return;
    };

    assert!(
        pg.len() > 50,
        "applied Postgres schema should have many tables; got {} \
         (migration/connection regression?)",
        pg.len()
    );

    // For every table present in BOTH backends, the column-name sets must match.
    // Type spelling differs across engines and is intentionally not compared.
    let mut shared_tables = 0usize;
    let mut mismatches: Vec<String> = Vec::new();
    for (table, sqlite_cols) in &sqlite {
        let Some(pg_cols) = pg.get(table) else {
            continue; // whole-table divergence is reported by the other test
        };
        shared_tables += 1;
        let missing_in_pg: Vec<&String> = sqlite_cols.difference(pg_cols).collect();
        let missing_in_sqlite: Vec<&String> = pg_cols.difference(sqlite_cols).collect();
        if !missing_in_pg.is_empty() || !missing_in_sqlite.is_empty() {
            mismatches.push(format!(
                "table `{table}`: columns only in SQLite={missing_in_pg:?}, \
                 only in Postgres={missing_in_sqlite:?}"
            ));
        }
    }

    // Guard against a vacuous pass where the two schemas share (almost) no
    // tables — that would mean the comparison silently checked nothing.
    assert!(
        shared_tables > 50,
        "expected the backends to share most tables; only {shared_tables} compared"
    );
    assert!(
        mismatches.is_empty(),
        "SQLite<->Postgres column drift on shared tables:\n  {}",
        mismatches.join("\n  ")
    );
}

/// Open the factory's database and, when it is Postgres-backed (so the
/// `migrations_pg` tree is what was applied), read its applied
/// table -> {column names} map.
///
/// Returns `None` — so the caller skips cleanly — when the factory opened
/// SQLite instead: the `postgresql` feature is off, or `DATABASE_URL` names no
/// Postgres server.
async fn applied_pg_schema() -> Option<BTreeMap<String, BTreeSet<String>>> {
    let db = create_test_db()
        .await
        .expect("open the factory's test database");
    match &db {
        Database::SQLite(_) => None,
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(pg) => {
            let rows = sqlx::query(
                "SELECT table_name, column_name \
                 FROM information_schema.columns \
                 WHERE table_schema = 'public' \
                 ORDER BY table_name, column_name",
            )
            .fetch_all(pg.pool())
            .await
            .expect("introspect Postgres information_schema");

            let mut schema: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
            for row in rows {
                let table: String = row.get("table_name");
                let lower = table.to_lowercase();
                if lower == SQLX_BOOKKEEPING_TABLE {
                    continue;
                }
                let column: String = row.get("column_name");
                schema
                    .entry(lower)
                    .or_default()
                    .insert(column.to_lowercase());
            }
            Some(schema)
        }
    }
}

/// The value set a `CHECK (col IN (...))` admits, read from the LAST migration in
/// `dir` that constrains `table.column` — later rebuilds supersede earlier ones.
fn check_in_values(dir: &str, table: &str, column: &str) -> Option<BTreeSet<String>> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "sql"))
        .collect();
    files.sort();

    let needle = format!("{column} ");
    let mut found = None;
    for path in files {
        let sql = fs::read_to_string(&path).unwrap_or_default();
        if !sql.contains(table) {
            continue;
        }
        for line in sql.lines() {
            let trimmed = line.trim_start();
            if !trimmed.starts_with(&needle) {
                continue;
            }
            let Some(open) = line
                .find("CHECK (")
                .and_then(|i| line[i..].find('(').map(|j| i + j))
            else {
                continue;
            };
            let Some(list_open) = line[open + 1..].find('(').map(|j| open + 1 + j) else {
                continue;
            };
            let Some(list_close) = line[list_open..].find(')').map(|j| list_open + j) else {
                continue;
            };
            found = Some(
                line[list_open + 1..list_close]
                    .split(',')
                    .map(|v| v.trim().trim_matches('\'').to_owned())
                    .filter(|v| !v.is_empty())
                    .collect::<BTreeSet<String>>(),
            );
        }
    }
    found
}

/// `tenant_users.role` reaches `TenantContext::is_admin` through
/// `TenantRole::from_db_string`, so a backend that admits a different role set than
/// the other is an authorization divergence, not a cosmetic one. It shipped
/// unnoticed because the column-name parity check above cannot see a constraint:
/// SQLite accepted 'viewer' and rejected 'billing' while PostgreSQL did the
/// reverse, so the same write succeeded in dev and constraint-failed in
/// production.
#[test]
fn tenant_users_role_check_agrees_across_backends() {
    let sqlite = check_in_values(SQLITE_MIGRATIONS, "tenant_users", "role")
        .expect("SQLite constrains tenant_users.role");
    let pg = check_in_values(PG_MIGRATIONS, "tenant_users", "role")
        .expect("PostgreSQL constrains tenant_users.role");

    assert_eq!(
        sqlite, pg,
        "tenant_users.role admits different values per backend — sqlite={sqlite:?} pg={pg:?}"
    );

    // And both must be exactly the TenantRole variants, so neither drifts away
    // from the enum together.
    let expected: BTreeSet<String> = ["owner", "admin", "billing", "member"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    assert_eq!(
        sqlite, expected,
        "the role CHECK must name exactly the TenantRole variants"
    );
}
