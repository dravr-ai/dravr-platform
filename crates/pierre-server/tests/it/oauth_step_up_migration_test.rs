// ABOUTME: The step-up migration: oauth_grant_events keeps every row and index through SQLite's rebuild, and admits consume
// ABOUTME: Replays each lane's migrations up to it, plants receipts, runs it, and reads the table and its indexes back

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1544 slice 4b. 20261126100000 widens `oauth_grant_events.event`
//! to `consume` and `step_up`. SQLite cannot alter a CHECK, so it rebuilds
//! the table: no receipt may be lost or changed, all five indexes must come
//! back, and `consume` joins the terminal index, so a consumed child has
//! exactly one ending. PostgreSQL swaps the constraint and the index.
//!
//! Each lane has its own migration set, so the factory's URL decides which
//! one is replayed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::str::FromStr;

use pierre_test_support::db::create_test_db_url;
use sqlx::migrate::Migrator;
#[cfg(feature = "postgresql")]
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;

/// The migration under test; everything before it builds the old shape.
const STEP_UP_MIGRATION: i64 = 20_261_126_100_000;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
#[cfg(feature = "postgresql")]
static PG_MIGRATOR: Migrator = sqlx::migrate!("../../migrations_pg");

/// Two receipts of one grant, one of them signed and logged, planted before
/// the migration.
const RECEIPTS: &str = r#"
INSERT INTO oauth_grant_events (id, tenant_id, user_id, client_grant_id, event, actor, at, payload,
        payload_sha256, jws, logged_at, attempts, poisoned_at) VALUES
    ('receipt-issue', 'tenant-1', 'user-1', 'grant-1', 'issue', 'user', '2026-11-20T10:00:00+00:00',
     '{"event":"issue"}', 'sha-issue', 'jws-issue', '2026-11-20T10:00:05+00:00', 1, NULL),
    ('receipt-revoke', 'tenant-1', 'user-1', 'grant-1', 'revoke', 'user', '2026-11-21T10:00:00+00:00',
     '{"event":"revoke"}', 'sha-revoke', NULL, NULL, 3, NULL);
"#;

/// Every receipt, as the columns read back: id, event, actor, payload,
/// payload hash, signature, logged, attempts.
type Receipt = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    bool,
    i64,
);

fn planted() -> Vec<Receipt> {
    vec![
        (
            "receipt-issue".to_owned(),
            "issue".to_owned(),
            "user".to_owned(),
            r#"{"event":"issue"}"#.to_owned(),
            "sha-issue".to_owned(),
            Some("jws-issue".to_owned()),
            true,
            1,
        ),
        (
            "receipt-revoke".to_owned(),
            "revoke".to_owned(),
            "user".to_owned(),
            r#"{"event":"revoke"}"#.to_owned(),
            "sha-revoke".to_owned(),
            None,
            false,
            3,
        ),
    ]
}

/// A `consume` receipt of `grant`, `id`.
fn consume(id: &str, grant: &str) -> String {
    format!(
        "INSERT INTO oauth_grant_events (id, tenant_id, user_id, client_grant_id, event, actor, at,
                payload, payload_sha256, attempts)
         VALUES ('{id}', 'tenant-1', 'user-1', '{grant}', 'consume', 'auto',
                 '2026-11-22T10:00:00+00:00', '{{}}', 'sha-{id}', 0)"
    )
}

const RECEIPTS_SQL: &str = "SELECT id, event, actor, payload, payload_sha256, jws,
        logged_at IS NOT NULL AS logged, attempts
     FROM oauth_grant_events ORDER BY id";

#[tokio::test]
async fn the_rebuild_keeps_every_receipt_and_index_and_consume_ends_a_grant_once() {
    let database = create_test_db_url().await.unwrap();
    #[cfg(feature = "postgresql")]
    if database.url.starts_with("postgres") {
        on_postgres(&database.url).await;
        return;
    }
    on_sqlite(&database.url).await;
}

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

async fn on_sqlite(url: &str) {
    let options = SqliteConnectOptions::from_str(url)
        .unwrap()
        .foreign_keys(false);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let mut sql = None;
    for migration in MIGRATOR.iter() {
        if migration.version == STEP_UP_MIGRATION {
            sql = Some(migration.sql.to_string());
            break;
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    let sql = sql.expect("the step-up migration is in migrations/");
    let indexes = |pool: sqlx::SqlitePool| async move {
        let mut names: Vec<String> = sqlx::query(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'oauth_grant_events'
             AND name NOT LIKE 'sqlite_autoindex%'",
        )
        .fetch_all(&pool)
        .await
        .unwrap()
        .iter()
        .map(|row| row.get("name"))
        .collect();
        names.sort();
        names
    };
    let before = indexes(pool.clone()).await;
    assert_eq!(before.len(), 5, "the old table's indexes: {before:?}");
    sqlx::raw_sql(RECEIPTS).execute(&pool).await.unwrap();

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();

    let rows: Vec<Receipt> = sqlx::query(RECEIPTS_SQL)
        .fetch_all(&pool)
        .await
        .unwrap()
        .iter()
        .map(|row| {
            (
                row.get("id"),
                row.get("event"),
                row.get("actor"),
                row.get("payload"),
                row.get("payload_sha256"),
                row.get("jws"),
                row.get::<i64, _>("logged") != 0,
                row.get("attempts"),
            )
        })
        .collect();
    assert_eq!(
        rows,
        planted(),
        "bug: the rebuild lost or changed a receipt"
    );
    assert_eq!(
        indexes(pool.clone()).await,
        before,
        "bug: the rebuild did not bring every index back"
    );

    sqlx::raw_sql(&consume("child-consume", "child-1"))
        .execute(&pool)
        .await
        .expect("bug: consume is refused by the widened CHECK");
    assert!(
        sqlx::raw_sql(&consume("child-consume-again", "child-1"))
            .execute(&pool)
            .await
            .is_err(),
        "bug: a consumed child took a second ending"
    );
    assert!(
        sqlx::raw_sql(&consume("grant-consume", "grant-1"))
            .execute(&pool)
            .await
            .is_err(),
        "bug: a revoked grant took a consume after its revoke"
    );
}

// ---------------------------------------------------------------------------
// PostgreSQL
// ---------------------------------------------------------------------------

#[cfg(feature = "postgresql")]
async fn on_postgres(url: &str) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .unwrap();
    sqlx::raw_sql("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
        .execute(&pool)
        .await
        .unwrap();
    let mut sql = None;
    for migration in PG_MIGRATOR.iter() {
        if migration.version == STEP_UP_MIGRATION {
            sql = Some(migration.sql.to_string());
            break;
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    let sql = sql.expect("the step-up migration is in migrations_pg/");
    sqlx::raw_sql(RECEIPTS).execute(&pool).await.unwrap();

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();

    let rows: Vec<Receipt> = sqlx::query(RECEIPTS_SQL)
        .fetch_all(&pool)
        .await
        .unwrap()
        .iter()
        .map(|row| {
            (
                row.get("id"),
                row.get("event"),
                row.get("actor"),
                row.get("payload"),
                row.get("payload_sha256"),
                row.get("jws"),
                row.get::<bool, _>("logged"),
                i64::from(row.get::<i32, _>("attempts")),
            )
        })
        .collect();
    assert_eq!(rows, planted(), "bug: the migration changed a receipt");
    sqlx::raw_sql(&consume("child-consume", "child-1"))
        .execute(&pool)
        .await
        .expect("bug: consume is refused by the widened CHECK");
    assert!(
        sqlx::raw_sql(&consume("child-consume-again", "child-1"))
            .execute(&pool)
            .await
            .is_err(),
        "bug: a consumed child took a second ending"
    );
}
