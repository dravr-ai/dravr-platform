// ABOUTME: The A2A API-key drop migration: the link table and a2a_clients.api_key_hash go, no client row does
// ABOUTME: Replays each lane's migrations up to it, plants a client with a session, usage and a linked key, reads what is left
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Until 20261108120000 an A2A client could hold API keys through
//! `a2a_client_api_keys`, and `a2a_clients.api_key_hash` held its Ed25519
//! public key. An A2A client's only way in is now its `OAuth2` registration,
//! so both go. Three things must hold on each lane:
//!
//! - a key the link still named is retired, so the dropped link leaves no
//!   working key behind, and a key it never named keeps working;
//! - the column and the table are gone;
//! - every client, session and usage row stays. `SQLite` rebuilds
//!   `a2a_clients` to drop a UNIQUE column, and with foreign keys ON (as the
//!   production pool connects) dropping the old table cascades into
//!   `a2a_sessions` and `a2a_usage`; the migration must put them back.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::str::FromStr;

use pierre_test_support::db::create_test_db_url;
use sqlx::migrate::Migrator;
#[cfg(feature = "postgresql")]
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;

/// The migration under test; everything before it builds the legacy shape.
const A2A_API_KEY_DROP_MIGRATION: i64 = 20_261_108_120_000;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
#[cfg(feature = "postgresql")]
static PG_MIGRATOR: Migrator = sqlx::migrate!("../../migrations_pg");

/// What is left after the migration, read the same way on both lanes.
#[derive(Debug, PartialEq, Eq)]
struct Landed {
    /// `client_id` of every client row
    clients: Vec<String>,
    /// `session_token` of every session row
    sessions: Vec<String>,
    /// `endpoint session_token` of every usage row
    usage: Vec<String>,
    /// `id active|retired` of every API key
    keys: Vec<String>,
    /// Whether `a2a_clients` still has an `api_key_hash` column
    has_api_key_hash: bool,
    /// Whether `a2a_client_api_keys` still exists
    has_link_table: bool,
}

fn assert_landed(landed: &Landed) {
    assert_eq!(landed.clients, ["client-1"], "every client stays");
    assert_eq!(landed.sessions, ["session-1"], "every session stays");
    assert_eq!(
        landed.usage,
        ["/a2a session-1"],
        "every usage row stays, still naming its session"
    );
    assert_eq!(
        landed.keys,
        ["key-linked retired", "key-unlinked active"],
        "the key the link named is retired; a key it never named keeps working"
    );
    assert!(!landed.has_api_key_hash, "a2a_clients.api_key_hash is gone");
    assert!(!landed.has_link_table, "a2a_client_api_keys is gone");
}

#[tokio::test]
async fn the_link_table_and_key_column_go_and_every_client_session_and_usage_row_stays() {
    let database = create_test_db_url().await.unwrap();
    #[cfg(feature = "postgresql")]
    if database.url.starts_with("postgres") {
        migrates_on_postgres(&database.url).await;
        return;
    }
    migrates_on_sqlite(&database.url).await;
}

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

/// The legacy rows: a client with a session, one usage row on it, a key the
/// link names and one it does not.
const SQLITE_FIXTURE: &str = "
INSERT INTO users (id, email, password_hash, created_at, last_active)
    VALUES ('aaaaaaaa-0000-4000-8000-0000000000a2', 'owner@a2a-drop.test', 'x',
            '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO a2a_clients (client_id, user_id, name, api_key_hash, created_at, updated_at)
    VALUES ('client-1', 'aaaaaaaa-0000-4000-8000-0000000000a2', 'agent', 'ed25519-public-key',
            '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
INSERT INTO a2a_sessions (session_token, client_id, granted_scopes, created_at, expires_at,
        last_active_at)
    VALUES ('session-1', 'client-1', '[]', '2026-01-01T00:00:00Z', '2099-01-01T00:00:00Z',
            '2026-01-01T00:00:00Z');
INSERT INTO a2a_usage (id, client_id, session_token, timestamp, endpoint, status_code)
    VALUES ('usage-1', 'client-1', 'session-1', '2026-01-01T00:00:00Z', '/a2a', 200);
INSERT INTO api_keys (id, user_id, name, key_hash, key_prefix, tier, created_at)
    VALUES ('key-linked', 'aaaaaaaa-0000-4000-8000-0000000000a2', 'A2A Client: agent', 'hash-1',
            'prefix-1', 'professional', '2026-01-01T00:00:00Z'),
           ('key-unlinked', 'aaaaaaaa-0000-4000-8000-0000000000a2', 'own key', 'hash-2',
            'prefix-2', 'starter', '2026-01-01T00:00:00Z');
INSERT INTO a2a_client_api_keys (client_id, api_key_id, created_at)
    VALUES ('client-1', 'key-linked', '2026-01-01T00:00:00Z');
";

/// A database migrated up to, not including, the migration under test, and
/// that migration's SQL. One connection: every pooled connection to an
/// in-memory database is its own empty database. The replay runs with
/// foreign keys off, as `team_membership_migration_test` replays it; the
/// migration itself then runs with them on.
async fn sqlite_before_migration(url: &str) -> (sqlx::SqlitePool, String) {
    let options = SqliteConnectOptions::from_str(url)
        .unwrap()
        .foreign_keys(false);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    for migration in MIGRATOR.iter() {
        if migration.version == A2A_API_KEY_DROP_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the A2A API-key drop migration is in migrations/");
}

async fn migrates_on_sqlite(url: &str) {
    let (pool, sql) = sqlite_before_migration(url).await;
    sqlx::raw_sql(SQLITE_FIXTURE).execute(&pool).await.unwrap();
    sqlx::raw_sql("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();

    // As sqlx applies it: one transaction, foreign keys ON.
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql(&sql).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();

    assert_landed(&sqlite_landed(&pool).await);
    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(violations.is_empty(), "no row is left pointing nowhere");
}

async fn sqlite_texts(pool: &sqlx::SqlitePool, sql: &str) -> Vec<String> {
    sqlx::query(sql)
        .fetch_all(pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
        .iter()
        .map(|row| row.get::<String, _>("r"))
        .collect()
}

async fn sqlite_landed(pool: &sqlx::SqlitePool) -> Landed {
    Landed {
        clients: sqlite_texts(pool, "SELECT client_id AS r FROM a2a_clients ORDER BY 1").await,
        sessions: sqlite_texts(
            pool,
            "SELECT session_token AS r FROM a2a_sessions ORDER BY 1",
        )
        .await,
        usage: sqlite_texts(
            pool,
            "SELECT endpoint || ' ' || COALESCE(session_token, '-') AS r FROM a2a_usage ORDER BY 1",
        )
        .await,
        keys: sqlite_texts(
            pool,
            "SELECT id || ' ' || CASE WHEN is_active THEN 'active' ELSE 'retired' END AS r
             FROM api_keys ORDER BY 1",
        )
        .await,
        has_api_key_hash: !sqlite_texts(
            pool,
            "SELECT name AS r FROM pragma_table_info('a2a_clients') WHERE name = 'api_key_hash'",
        )
        .await
        .is_empty(),
        has_link_table: !sqlite_texts(
            pool,
            "SELECT name AS r FROM sqlite_master WHERE name = 'a2a_client_api_keys'",
        )
        .await
        .is_empty(),
    }
}

// ---------------------------------------------------------------------------
// PostgreSQL
// ---------------------------------------------------------------------------

#[cfg(feature = "postgresql")]
const POSTGRES_FIXTURE: &str = "
INSERT INTO users (id, email, password_hash)
    VALUES ('aaaaaaaa-0000-4000-8000-0000000000a2'::uuid, 'owner@a2a-drop.test', 'x');
INSERT INTO a2a_clients (client_id, user_id, name, api_key_hash)
    VALUES ('client-1', 'aaaaaaaa-0000-4000-8000-0000000000a2'::uuid, 'agent', 'ed25519-public-key');
INSERT INTO a2a_sessions (session_token, client_id, user_id, expires_at)
    VALUES ('session-1', 'client-1', 'aaaaaaaa-0000-4000-8000-0000000000a2'::uuid,
            '2099-01-01T00:00:00Z');
INSERT INTO a2a_usage (client_id, session_token, endpoint, status_code)
    VALUES ('client-1', 'session-1', '/a2a', 200);
INSERT INTO api_keys (id, user_id, name, key_hash, key_prefix, tier, rate_limit_requests,
        rate_limit_window_seconds)
    VALUES ('key-linked', 'aaaaaaaa-0000-4000-8000-0000000000a2'::uuid, 'A2A Client: agent',
            'hash-1', 'prefix-1', 'professional', 1000, 3600),
           ('key-unlinked', 'aaaaaaaa-0000-4000-8000-0000000000a2'::uuid, 'own key',
            'hash-2', 'prefix-2', 'starter', 1000, 3600);
INSERT INTO a2a_client_api_keys (client_id, api_key_id, created_at)
    VALUES ('client-1', 'key-linked', now());
";

/// A database migrated up to, not including, the migration under test, and
/// that migration's SQL. The factory hands out a clone of the migrated
/// template, so start again from an empty schema.
#[cfg(feature = "postgresql")]
async fn postgres_before_migration(url: &str) -> (sqlx::PgPool, String) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .unwrap();
    sqlx::raw_sql("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
        .execute(&pool)
        .await
        .unwrap();
    for migration in PG_MIGRATOR.iter() {
        if migration.version == A2A_API_KEY_DROP_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the A2A API-key drop migration is in migrations_pg/");
}

#[cfg(feature = "postgresql")]
async fn migrates_on_postgres(url: &str) {
    let (pool, sql) = postgres_before_migration(url).await;
    sqlx::raw_sql(POSTGRES_FIXTURE)
        .execute(&pool)
        .await
        .unwrap();

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    let landed = postgres_landed(&pool).await;
    assert_landed(&landed);

    // The PostgreSQL half is idempotent as a whole: a second run changes nothing.
    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    assert_eq!(
        postgres_landed(&pool).await,
        landed,
        "a re-run changes nothing"
    );
}

#[cfg(feature = "postgresql")]
async fn postgres_texts(pool: &sqlx::PgPool, sql: &str) -> Vec<String> {
    sqlx::query(sql)
        .fetch_all(pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
        .iter()
        .map(|row| row.get::<String, _>("r"))
        .collect()
}

#[cfg(feature = "postgresql")]
async fn postgres_landed(pool: &sqlx::PgPool) -> Landed {
    Landed {
        clients: postgres_texts(pool, "SELECT client_id AS r FROM a2a_clients ORDER BY 1").await,
        sessions: postgres_texts(
            pool,
            "SELECT session_token AS r FROM a2a_sessions ORDER BY 1",
        )
        .await,
        usage: postgres_texts(
            pool,
            "SELECT endpoint || ' ' || COALESCE(session_token, '-') AS r FROM a2a_usage ORDER BY 1",
        )
        .await,
        keys: postgres_texts(
            pool,
            "SELECT id || ' ' || CASE WHEN is_active THEN 'active' ELSE 'retired' END AS r
             FROM api_keys ORDER BY 1",
        )
        .await,
        has_api_key_hash: !postgres_texts(
            pool,
            "SELECT column_name::text AS r FROM information_schema.columns
             WHERE table_name = 'a2a_clients' AND column_name = 'api_key_hash'",
        )
        .await
        .is_empty(),
        has_link_table: !postgres_texts(
            pool,
            "SELECT COALESCE(to_regclass('a2a_client_api_keys')::text, '') AS r
             WHERE to_regclass('a2a_client_api_keys') IS NOT NULL",
        )
        .await
        .is_empty(),
    }
}
