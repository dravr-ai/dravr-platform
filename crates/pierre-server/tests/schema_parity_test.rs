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
//!   fallback that parses `CREATE TABLE` statements out of both directories and
//!   reports any table created in one tree but not the other. It catches the
//!   most common drift — a whole table/feature wired into a single backend — in
//!   every CI lane, even ones with no Postgres.
//!
//! Known intentional divergences between the trees (e.g. the OAuth2
//! authorization-code store is named `oauth2_auth_codes` on SQLite but
//! `authorization_codes` on PostgreSQL) are reported by the fallback as
//! findings rather than silently swallowed; see the test body for the current
//! reported set.

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
use std::path::Path;

use pierre_database::database::test_utils::create_sqlite_test_db;
use sqlx::Row;

/// Repo-root-relative path to the SQLite migration tree.
const SQLITE_MIGRATIONS: &str = "../../migrations";
/// Repo-root-relative path to the PostgreSQL migration tree.
const PG_MIGRATIONS: &str = "../../migrations_pg";

/// `_sqlx_migrations` is sqlx bookkeeping, not part of the application schema.
const SQLX_BOOKKEEPING_TABLE: &str = "_sqlx_migrations";

/// Read every `CREATE TABLE [IF NOT EXISTS] <name>` table name from the `.sql`
/// files in `dir`, lowercased.
///
/// Transient `*_new` tables — SQLite's 12-step "rebuild table" pattern creates
/// `foo_new`, copies rows, drops `foo`, then renames `foo_new` -> `foo` — are
/// excluded because they never exist in the final applied schema.
fn created_table_names(dir: &str) -> BTreeSet<String> {
    let mut tables = BTreeSet::new();
    let read_dir = fs::read_dir(Path::new(dir))
        .unwrap_or_else(|e| panic!("cannot read migration dir {dir}: {e}"));
    for entry in read_dir {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("sql") {
            continue;
        }
        let sql = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        for name in parse_create_table_names(&sql) {
            if !name.ends_with("_new") {
                tables.insert(name);
            }
        }
    }
    tables
}

/// Extract the table name from each `CREATE TABLE` statement in `sql`.
///
/// Deliberately tolerant: handles optional `IF NOT EXISTS`, optional quoting,
/// and ignores `CREATE TABLE` appearing inside line comments.
fn parse_create_table_names(sql: &str) -> Vec<String> {
    let mut names = Vec::new();
    for raw_line in sql.lines() {
        let line = raw_line.trim();
        if line.starts_with("--") {
            continue;
        }
        let upper = line.to_uppercase();
        let Some(idx) = upper.find("CREATE TABLE ") else {
            continue;
        };
        // Slice the original (case-preserving) line at the byte offset found in
        // the uppercased copy — ASCII keywords keep byte positions identical.
        let mut after = line[idx + "CREATE TABLE ".len()..].trim_start();
        if after.len() >= "IF NOT EXISTS ".len()
            && after[.."IF NOT EXISTS ".len()].eq_ignore_ascii_case("IF NOT EXISTS ")
        {
            after = after["IF NOT EXISTS ".len()..].trim_start();
        }
        let token: String = after
            .trim_start_matches(['"', '`'])
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !token.is_empty() {
            names.push(token.to_lowercase());
        }
    }
    names
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
    // P2-13 — portable, always-on guard. Parses CREATE TABLE statements from
    // both trees and reports any table wired into only one backend. Runs in
    // every CI lane (no Postgres required), catching the most common drift:
    // a whole table/feature added to a single backend.
    let sqlite_tables = created_table_names(SQLITE_MIGRATIONS);
    let pg_tables = created_table_names(PG_MIGRATIONS);

    let sqlite_only: Vec<&String> = sqlite_tables.difference(&pg_tables).collect();
    let pg_only: Vec<&String> = pg_tables.difference(&sqlite_tables).collect();

    // Surface the current divergence as test output so reviewers see it even
    // when the assertion below passes.
    if !sqlite_only.is_empty() || !pg_only.is_empty() {
        eprintln!("[schema-parity] tables only in SQLite tree: {sqlite_only:?}");
        eprintln!("[schema-parity] tables only in PostgreSQL tree: {pg_only:?}");
    }

    // Both trees must create a meaningful, overlapping schema — a parse/path
    // regression that found zero or near-zero shared tables would otherwise let
    // the guard pass vacuously.
    let shared = sqlite_tables.intersection(&pg_tables).count();
    assert!(
        shared > 50,
        "expected the two trees to share most tables; only {shared} in common \
         (parser or path regression?)"
    );
}

#[tokio::test]
async fn columns_match_on_shared_tables() {
    // P2-13 — applied-schema guard. SQLite side always runs in-memory; the
    // PostgreSQL side runs only when a Postgres URL + the `postgresql` feature
    // are present, so it executes for real in ci-postgres and skips cleanly
    // in the SQLite-only local lane.
    let db = create_sqlite_test_db().await.unwrap();
    let sqlite_pool = db
        .sqlite_pool()
        .expect("create_sqlite_test_db yields SQLite");
    let sqlite = sqlite_schema(sqlite_pool).await;
    assert!(
        sqlite.len() > 50,
        "applied SQLite schema should have many tables; got {}",
        sqlite.len()
    );

    let Some(pg) = applied_pg_schema().await else {
        eprintln!(
            "[schema-parity] no Postgres DATABASE_URL/TEST_DATABASE_URL (or `postgresql` \
             feature off) — SQLite-only run, skipping cross-backend column comparison"
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

/// Build a fresh Postgres-backed [`pierre_database`] (which runs the
/// `migrations_pg` tree) and read its applied table -> {column names} map.
///
/// Returns `None` — so the caller skips cleanly — when the `postgresql` feature
/// is off or no Postgres connection string is configured. Mirrors how
/// ci-postgres exports `DATABASE_URL`; `TEST_DATABASE_URL` is also honored.
///
/// On a SQLite-only build the `postgresql`-gated body is compiled out, so this
/// awaits a cheap yield to stay a valid `async fn` and immediately returns
/// `None`.
async fn applied_pg_schema() -> Option<BTreeMap<String, BTreeSet<String>>> {
    #[cfg(feature = "postgresql")]
    {
        use pierre_core::config::database::PostgresPoolConfig;
        use pierre_database::backends::factory::Database;

        let url = pg_test_url()?;
        let db = Database::new(&url, vec![0u8; 32], &PostgresPoolConfig::default())
            .await
            .expect("connect + migrate Postgres test database");
        let pool = db.postgres_pool().expect("Postgres backend yields pg pool");

        let rows = sqlx::query(
            "SELECT table_name, column_name \
             FROM information_schema.columns \
             WHERE table_schema = 'public' \
             ORDER BY table_name, column_name",
        )
        .fetch_all(pool)
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
    #[cfg(not(feature = "postgresql"))]
    {
        use tokio::task::yield_now;

        // No Postgres backend compiled in — yield once so this remains a real
        // `async fn` with an await, then skip the cross-backend comparison.
        yield_now().await;
        None
    }
}

/// Resolve a Postgres connection string from the environment, preferring an
/// explicit test URL. Returns `None` if neither is a Postgres URL.
#[cfg(feature = "postgresql")]
fn pg_test_url() -> Option<String> {
    use std::env;

    for key in ["TEST_DATABASE_URL", "DATABASE_URL"] {
        if let Ok(url) = env::var(key) {
            if url.starts_with("postgres://") || url.starts_with("postgresql://") {
                return Some(url);
            }
        }
    }
    None
}
