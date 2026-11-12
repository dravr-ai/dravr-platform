// ABOUTME: The backfill-job kinds migration, run after the import-row delete: every row left is a backfill, kind joins the key
// ABOUTME: Replays each lane's migrations up to the delete, plants rows of both old kinds, runs both, reads the kinds and key left

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Until 20261112130000 a job row was keyed on `(user_id, provider)` with
//! a nullable `kind`. It runs after 20261112120000, which deletes every row
//! the retired history import wrote (a kind or an outcome set). Planted
//! before that delete, the legacy rows go through both, and on each lane:
//!
//! - the import's rows are gone, and every row left is a `backfill`;
//! - make `kind` NOT NULL, since NULLs never collide in a UNIQUE;
//! - leave `(user_id, provider, kind)` the only unique key, so a second
//!   kind for one provider is a row of its own and a second row of one kind
//!   is still refused. On `PostgreSQL` the old key's name is asserted before
//!   the migration runs: a misspelled `DROP CONSTRAINT IF EXISTS` would
//!   silently leave it standing;
//! - start the new progress columns at zero.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::str::FromStr;

use pierre_test_support::db::create_test_db_url;
use sqlx::migrate::Migrator;
#[cfg(feature = "postgresql")]
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;

/// The delete of the retired import's rows; everything before it builds the
/// legacy shape the rows are planted in.
const IMPORT_ROWS_DELETE_MIGRATION: i64 = 20_261_112_120_000;

/// The migration under test, which must run after the delete.
const BACKFILL_JOB_KINDS_MIGRATION: i64 = 20_261_112_130_000;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
#[cfg(feature = "postgresql")]
static PG_MIGRATOR: Migrator = sqlx::migrate!("../../migrations_pg");

/// What is left after the migration, read the same way on both lanes.
#[derive(Debug, PartialEq, Eq)]
struct Landed {
    /// `provider kind outcome months_done rows_landed` of every row
    rows: Vec<String>,
    /// Whether a row with a NULL kind is refused
    null_kind_refused: bool,
    /// Whether a second kind for an existing `(user, provider)` is its own row
    second_kind_recorded: bool,
    /// Whether a second row of an existing kind is refused
    same_kind_refused: bool,
}

fn assert_landed(landed: &Landed) {
    assert_eq!(
        landed.rows,
        ["strava backfill - 0 0"],
        "the import's rows are deleted; the ordinary row stays a backfill, its progress at zero"
    );
    assert!(landed.null_kind_refused, "kind is NOT NULL");
    assert!(
        landed.second_kind_recorded,
        "a second kind for one (user, provider) is a row of its own"
    );
    assert!(
        landed.same_kind_refused,
        "a second row of one kind for one (user, provider) is still refused"
    );
}

#[tokio::test]
async fn kinds_are_renamed_made_not_null_and_joined_to_the_unique_key() {
    let database = create_test_db_url().await.unwrap();
    #[cfg(feature = "postgresql")]
    if database.url.starts_with("postgres") {
        migrates_on_postgres(&database.url).await;
        return;
    }
    migrates_on_sqlite(&database.url).await;
}

/// The legacy rows: an ordinary backfill (NULL kind), an owed import and an
/// import that ended incomplete, all for one athlete.
const LEGACY_ROWS: &str = "
INSERT INTO activity_backfill_jobs
    (id, tenant_id, user_id, provider, after_ts, created_at_ms, lease_token, outcome, kind)
VALUES
    ('job-backfill', 'bbbbbbbb-0000-4000-8000-0000000000b1', 'aaaaaaaa-0000-4000-8000-0000000000b1', 'strava',
     1700000000, 1, 'token-1', NULL, NULL),
    ('job-import', 'bbbbbbbb-0000-4000-8000-0000000000b1', 'aaaaaaaa-0000-4000-8000-0000000000b1', 'sciotte_garmin',
     946684800, 2, NULL, NULL, 'history_import'),
    ('job-ended', 'bbbbbbbb-0000-4000-8000-0000000000b1', 'aaaaaaaa-0000-4000-8000-0000000000b1', 'garmin',
     946684800, 3, NULL, 'incomplete', 'history_import');
";

/// One more row for the planted athlete's `strava` pair: `$1` its id, `$2`
/// its kind.
const INSERT_STRAVA_SQL: &str =
    "INSERT INTO activity_backfill_jobs (id, tenant_id, user_id, provider, created_at_ms, kind) \
     VALUES ($1, 'bbbbbbbb-0000-4000-8000-0000000000b1', 'aaaaaaaa-0000-4000-8000-0000000000b1', \
             'strava', 4, $2)";

const LANDED_ROWS_SQL: &str = "SELECT provider || ' ' || kind || ' ' || COALESCE(outcome, '-') \
     || ' ' || CAST(months_done AS TEXT) || ' ' || CAST(rows_landed AS TEXT) AS r \
     FROM activity_backfill_jobs ORDER BY 1";

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

/// The SQL of every migration from the import-row delete through the one
/// under test, in order, from `migrator`; the delete must come first.
fn delete_through_kinds(migrator: &Migrator) -> Vec<(i64, String)> {
    let pending: Vec<(i64, String)> = migrator
        .iter()
        .filter(|m| {
            (IMPORT_ROWS_DELETE_MIGRATION..=BACKFILL_JOB_KINDS_MIGRATION).contains(&m.version)
        })
        .map(|m| (m.version, m.sql.to_string()))
        .collect();
    assert_eq!(
        pending.first().map(|(version, _)| *version),
        Some(IMPORT_ROWS_DELETE_MIGRATION),
        "the import-row delete is in the migrations"
    );
    assert_eq!(
        pending.last().map(|(version, _)| *version),
        Some(BACKFILL_JOB_KINDS_MIGRATION),
        "the kinds migration runs after the delete"
    );
    pending
}

/// A database migrated up to, not including, the import-row delete, and the
/// migrations from it through the one under test. One connection: every
/// pooled connection to an in-memory database is its own empty database.
async fn sqlite_before_migration(url: &str) -> (sqlx::SqlitePool, Vec<(i64, String)>) {
    let options = SqliteConnectOptions::from_str(url)
        .unwrap()
        .foreign_keys(false);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    for migration in MIGRATOR.iter() {
        if migration.version == IMPORT_ROWS_DELETE_MIGRATION {
            return (pool, delete_through_kinds(&MIGRATOR));
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the import-row delete is in migrations/");
}

async fn migrates_on_sqlite(url: &str) {
    let (pool, pending) = sqlite_before_migration(url).await;
    sqlx::raw_sql(LEGACY_ROWS).execute(&pool).await.unwrap();

    // As sqlx applies them: one transaction each, in order.
    for (_, sql) in &pending {
        let mut tx = pool.begin().await.unwrap();
        sqlx::raw_sql(sql).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
    }

    let index: Vec<String> = sqlx::query(
        "SELECT name AS r FROM sqlite_master \
         WHERE tbl_name = 'activity_backfill_jobs' AND name = 'idx_activity_backfill_jobs_due'",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .iter()
    .map(|row| row.get::<String, _>("r"))
    .collect();
    assert_eq!(
        index,
        ["idx_activity_backfill_jobs_due"],
        "the due index is rebuilt with the table"
    );

    let rows = sqlx::query(LANDED_ROWS_SQL)
        .fetch_all(&pool)
        .await
        .unwrap()
        .iter()
        .map(|row| row.get::<String, _>("r"))
        .collect();
    let insert = |id: &'static str, kind: Option<&'static str>| {
        sqlx::query(INSERT_STRAVA_SQL)
            .bind(id)
            .bind(kind)
            .execute(&pool)
    };
    let null_kind_refused = insert("job-null", None).await.is_err();
    let second_kind_recorded = insert("job-walk", Some("history_walk")).await.is_ok();
    let same_kind_refused = insert("job-again", Some("backfill")).await.is_err();
    assert_landed(&Landed {
        rows,
        null_kind_refused,
        second_kind_recorded,
        same_kind_refused,
    });
}

// ---------------------------------------------------------------------------
// PostgreSQL
// ---------------------------------------------------------------------------

/// `name definition` of every UNIQUE constraint on the table.
#[cfg(feature = "postgresql")]
const UNIQUE_CONSTRAINTS_SQL: &str =
    "SELECT conname::text || ' ' || pg_get_constraintdef(oid) AS r FROM pg_constraint \
     WHERE conrelid = 'activity_backfill_jobs'::regclass AND contype = 'u' ORDER BY 1";

/// A database migrated up to, not including, the import-row delete, and the
/// migrations from it through the one under test. The factory hands out a
/// clone of the migrated template, so start again from an empty schema.
#[cfg(feature = "postgresql")]
async fn postgres_before_migration(url: &str) -> (sqlx::PgPool, Vec<(i64, String)>) {
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
        if migration.version == IMPORT_ROWS_DELETE_MIGRATION {
            return (pool, delete_through_kinds(&PG_MIGRATOR));
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the import-row delete is in migrations_pg/");
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

/// Whether one more `strava` row of `kind` is accepted, inside a transaction
/// so a refusal leaves the connection usable.
#[cfg(feature = "postgresql")]
async fn postgres_accepts(pool: &sqlx::PgPool, id: &str, kind: Option<&str>) -> bool {
    let mut tx = pool.begin().await.unwrap();
    let accepted = sqlx::query(INSERT_STRAVA_SQL)
        .bind(id)
        .bind(kind)
        .execute(&mut *tx)
        .await
        .is_ok();
    if accepted {
        tx.commit().await.unwrap();
    } else {
        tx.rollback().await.unwrap();
    }
    accepted
}

#[cfg(feature = "postgresql")]
async fn migrates_on_postgres(url: &str) {
    let (pool, pending) = postgres_before_migration(url).await;
    assert_eq!(
        postgres_texts(&pool, UNIQUE_CONSTRAINTS_SQL).await,
        ["activity_backfill_jobs_user_id_provider_key UNIQUE (user_id, provider)"],
        "the old key carries the name the migration drops"
    );
    sqlx::raw_sql(LEGACY_ROWS).execute(&pool).await.unwrap();

    for (_, sql) in &pending {
        sqlx::raw_sql(sql).execute(&pool).await.unwrap();
    }
    let kinds_sql = &pending
        .last()
        .expect("the kinds migration is among them")
        .1;
    let after = postgres_texts(&pool, UNIQUE_CONSTRAINTS_SQL).await;
    assert_eq!(
        after,
        ["activity_backfill_jobs_user_kind_key UNIQUE (user_id, provider, kind)"],
        "the new key is the only unique key left"
    );
    let rows = postgres_texts(&pool, LANDED_ROWS_SQL).await;

    // The PostgreSQL half is idempotent as a whole: a second run changes nothing.
    sqlx::raw_sql(kinds_sql).execute(&pool).await.unwrap();
    assert_eq!(
        postgres_texts(&pool, UNIQUE_CONSTRAINTS_SQL).await,
        after,
        "a re-run leaves the one key"
    );
    assert_eq!(
        postgres_texts(&pool, LANDED_ROWS_SQL).await,
        rows,
        "a re-run changes no row"
    );

    let null_kind_refused = !postgres_accepts(&pool, "job-null", None).await;
    let second_kind_recorded = postgres_accepts(&pool, "job-walk", Some("history_walk")).await;
    let same_kind_refused = !postgres_accepts(&pool, "job-again", Some("backfill")).await;
    assert_landed(&Landed {
        rows,
        null_kind_refused,
        second_kind_recorded,
        same_kind_refused,
    });
}
