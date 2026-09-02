// ABOUTME: The predicate_code migration — legacy free-text rows become codes with the athlete's words intact
// ABOUTME: Replays each lane's migrations up to the cutover, plants the old shape, applies it, asserts every row
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The backfill is the one moment the old and new shapes meet, so it is
//! proven on a real database rather than trusted from the SQL: the seven
//! server-authored phrases map to their codes with the object untouched, and
//! every other row keeps the athlete's words under `states`. Each lane has
//! its own migration set, so the factory's URL decides which one is replayed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::str::FromStr;

#[cfg(feature = "postgresql")]
use chrono::{TimeZone, Utc};
use pierre_database::database::test_utils::create_test_db_url;
use sqlx::migrate::Migrator;
#[cfg(feature = "postgresql")]
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;

/// The migration under test; everything before it builds the legacy shape.
const PREDICATE_CODE_MIGRATION: i64 = 20_260_902_000_001;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
#[cfg(feature = "postgresql")]
static PG_MIGRATOR: Migrator = sqlx::migrate!("../../migrations_pg");

/// (id, kind, subject, predicate, object) as the pre-code prompt and the
/// server used to write them, and what each must become.
const LEGACY: [(&str, &str, &str, &str, &str, &str, &str); 7] = [
    (
        "north",
        "north_star",
        "you",
        "train because",
        "je veux rester en forme",
        "train_because",
        "je veux rester en forme",
    ),
    (
        "sport",
        "preference",
        "you",
        "primarily train",
        "Trail Running",
        "primarily_train",
        "Trail Running",
    ),
    (
        "goal",
        "goal",
        "you",
        "are working toward",
        "un ultra de 26 km",
        "working_toward",
        "un ultra de 26 km",
    ),
    (
        "parq",
        "medical",
        "you",
        "answered yes (PAR-Q)",
        "heart_condition",
        "parq_yes",
        "heart_condition",
    ),
    (
        "race",
        "goal",
        "you",
        "target race",
        "Boston (run) on 2026-04-20",
        "target_race",
        "Boston (run) on 2026-04-20",
    ),
    (
        "free",
        "goal",
        "you",
        "are racing",
        "Big Red on 2026-08-08",
        "states",
        "are racing Big Red on 2026-08-08",
    ),
    (
        "third",
        "other",
        "Coach Sarah",
        "recommends",
        "cadence drills weekly",
        "states",
        "Coach Sarah recommends cadence drills weekly",
    ),
];

#[tokio::test]
async fn legacy_rows_become_codes_and_keep_the_athletes_words() {
    let database = create_test_db_url().await.unwrap();
    #[cfg(feature = "postgresql")]
    if database.url.starts_with("postgres") {
        backfill_on_postgres(&database.url).await;
        return;
    }
    backfill_on_sqlite(&database.url).await;
}

async fn backfill_on_sqlite(url: &str) {
    // One connection: every pooled connection to an in-memory database is its
    // own empty database. Foreign keys off: the planted rows name no coach.
    let options = SqliteConnectOptions::from_str(url)
        .unwrap()
        .foreign_keys(false);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let mut applied_target = false;
    for migration in MIGRATOR.iter() {
        if migration.version == PREDICATE_CODE_MIGRATION {
            for (id, kind, subject, predicate, object, _, _) in LEGACY {
                sqlx::query(
                    "INSERT INTO user_facts (id, tenant_id, user_id, coach_id, scope, kind, subject, predicate, object, confidence, source, created_at, updated_at)
                     VALUES ($1, 't', 'u', NULL, 'user', $2, $3, $4, $5, 1.0, 'onboarding', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                )
                .bind(id)
                .bind(kind)
                .bind(subject)
                .bind(predicate)
                .bind(object)
                .execute(&pool)
                .await
                .unwrap();
            }
            sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
            applied_target = true;
            break;
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    assert!(
        applied_target,
        "the predicate_code migration is in migrations/"
    );

    for (id, _, _, _, _, code, object) in LEGACY {
        let row = sqlx::query("SELECT predicate_code, object FROM user_facts WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("predicate_code"), code, "{id}");
        assert_eq!(row.get::<String, _>("object"), object, "{id}");
    }
    // The free-text columns are gone, not kept "for compat".
    let columns: Vec<String> = sqlx::query("PRAGMA table_info(user_facts)")
        .fetch_all(&pool)
        .await
        .unwrap()
        .iter()
        .map(|r| r.get::<String, _>("name"))
        .collect();
    assert!(
        columns.contains(&"predicate_code".to_owned()),
        "{columns:?}"
    );
    assert!(
        !columns.contains(&"subject".to_owned()) && !columns.contains(&"predicate".to_owned()),
        "{columns:?}"
    );
    // An unknown code is refused by the schema itself.
    let refused = sqlx::query(
        "INSERT INTO user_facts (id, tenant_id, user_id, scope, kind, predicate_code, object, confidence, source, created_at, updated_at)
         VALUES ('bad', 't', 'u', 'user', 'goal', 'targets', 'x', 1.0, 'coach', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await;
    assert!(refused.is_err(), "the CHECK constraint rejects a phrase");
}

#[cfg(feature = "postgresql")]
async fn backfill_on_postgres(url: &str) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .unwrap();
    // The factory hands out a clone of the migrated template; the backfill
    // only happens as the migration runs, so start again from an empty schema.
    sqlx::raw_sql("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
        .execute(&pool)
        .await
        .unwrap();
    let planted_at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let mut applied_target = false;
    for migration in PG_MIGRATOR.iter() {
        if migration.version == PREDICATE_CODE_MIGRATION {
            for (id, kind, subject, predicate, object, _, _) in LEGACY {
                sqlx::query(
                    "INSERT INTO user_facts (id, tenant_id, user_id, coach_id, scope, kind, subject, predicate, object, confidence, source, created_at, updated_at)
                     VALUES ($1, $2, $3, NULL, 'user', $4, $5, $6, $7, 1.0, 'onboarding', $8, $8)",
                )
                .bind(id.to_owned())
                .bind("t".to_owned())
                .bind("u".to_owned())
                .bind(kind)
                .bind(subject)
                .bind(predicate)
                .bind(object)
                .bind(planted_at)
                .execute(&pool)
                .await
                .unwrap();
            }
            sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
            applied_target = true;
            break;
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    assert!(
        applied_target,
        "the predicate_code migration is in migrations_pg/"
    );

    for (_, _, _, _, planted, code, object) in LEGACY {
        // Rows are found by the object they were planted with; the seven
        // server phrases keep it, the free-text ones fold it into the new one.
        let row = sqlx::query(
            "SELECT predicate_code, object FROM user_facts WHERE object = $1 OR object LIKE '%' || $1",
        )
        .bind(planted)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.get::<String, _>("predicate_code"), code, "{planted}");
        assert_eq!(row.get::<String, _>("object"), object, "{planted}");
    }
    let columns: Vec<String> = sqlx::query(
        "SELECT column_name FROM information_schema.columns WHERE table_name = 'user_facts'",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .iter()
    .map(|r| r.get::<String, _>("column_name"))
    .collect();
    assert!(
        columns.contains(&"predicate_code".to_owned()),
        "{columns:?}"
    );
    assert!(
        !columns.contains(&"subject".to_owned()) && !columns.contains(&"predicate".to_owned()),
        "{columns:?}"
    );
}
