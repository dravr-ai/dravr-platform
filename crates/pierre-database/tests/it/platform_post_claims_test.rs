// ABOUTME: The platform post claim keyed by subject: the rebuild keeps every claim, and per-item rows coexist
// ABOUTME: Replays each lane's migrations to the rebuild, and pins that a pre-rebuild claim statement now errors

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1486. 20261118220000 adds `subject` to the key of
//! `platform_post_claims`, so the coach's reaction to a synced workout can be
//! claimed once per ride (`activity_landed.frequency = per_ride`) beside the
//! once-per-day claim every other purpose keeps under
//! [`PostPurpose::NO_SUBJECT`]. The migration must carry every claim as it
//! was, with subject `''`, and the repository must keep each subject's row
//! apart while `platform_post_posted_at` still answers the purpose's latest
//! post. An older binary's statements name the old key, so they must fail
//! after the rebuild — a rollback skips posts, never doubles them.
//!
//! Each lane has its own migration set, so the factory's URL decides which
//! one is replayed.

use std::str::FromStr;

use chrono::{DateTime, Duration, TimeZone, Utc};
use pierre_core::models::{PostPurpose, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::{create_test_db, create_test_db_url};
use pierre_test_support::server::create_test_user;
use sqlx::migrate::Migrator;
#[cfg(feature = "postgresql")]
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;
use uuid::Uuid;

/// The migration under test; everything before it builds the legacy shape.
const SUBJECT_MIGRATION: i64 = 20_261_118_220_000;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
#[cfg(feature = "postgresql")]
static PG_MIGRATOR: Migrator = sqlx::migrate!("../../migrations_pg");

const POSTED: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const HELD: &str = "aaaaaaaa-0000-4000-8000-000000000002";

/// One claim posted at a known instant, one held under a lease and never
/// posted: the two shapes a live table holds.
const CLAIMS: &str = r"
INSERT INTO platform_post_claims (tenant_id, user_id, purpose, claimed_at_ms, posted_at_ms, lease_token) VALUES
    ('11111111-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000001', 'first_sync', 1000, 2000, 'token-posted'),
    ('11111111-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000002', 'check_in', 3000, NULL, 'token-held');
";

/// What the claim statements said before the rebuild: an older binary still
/// runs it during a rolling deploy, or after a rollback.
const PRE_REBUILD_CLAIM: &str = r"
INSERT INTO platform_post_claims (tenant_id, user_id, purpose, claimed_at_ms, lease_token)
VALUES ('11111111-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000002', 'check_in', 4000, 'token-old')
ON CONFLICT (tenant_id, user_id, purpose) DO UPDATE
    SET claimed_at_ms = excluded.claimed_at_ms, lease_token = excluded.lease_token
";

/// A claim row as `(user, purpose, subject, claimed, posted, lease)`.
type ClaimRow = (String, String, String, i64, Option<i64>, Option<String>);

fn carried() -> Vec<ClaimRow> {
    vec![
        (
            POSTED.to_owned(),
            "first_sync".to_owned(),
            String::new(),
            1000,
            Some(2000),
            Some("token-posted".to_owned()),
        ),
        (
            HELD.to_owned(),
            "check_in".to_owned(),
            String::new(),
            3000,
            None,
            Some("token-held".to_owned()),
        ),
    ]
}

#[tokio::test]
async fn the_rebuild_carries_every_claim_and_refuses_the_old_key() {
    let database = create_test_db_url().await.unwrap();
    #[cfg(feature = "postgresql")]
    if database.url.starts_with("postgres") {
        rebuild_on_postgres(&database.url).await;
        return;
    }
    rebuild_on_sqlite(&database.url).await;
}

/// A database migrated up to, not including, the migration under test, and
/// that migration's SQL. One connection: every pooled connection to an
/// in-memory database is its own empty database. Foreign keys off, as the
/// migrations themselves run.
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
        if migration.version == SUBJECT_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("bug: the claims subject migration is not in migrations/");
}

async fn rebuild_on_sqlite(url: &str) {
    let (pool, sql) = sqlite_before_migration(url).await;
    for user in [POSTED, HELD] {
        sqlx::query(
            "INSERT INTO users (id, email, password_hash, created_at, last_active)
             VALUES ($1, $2, 'x', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(user)
        .bind(format!("{user}@claims.test"))
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::raw_sql(CLAIMS).execute(&pool).await.unwrap();

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    let rows: Vec<ClaimRow> = sqlx::query(
        "SELECT user_id, purpose, subject, claimed_at_ms, posted_at_ms, lease_token
         FROM platform_post_claims ORDER BY user_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .iter()
    .map(|row| {
        (
            row.get("user_id"),
            row.get("purpose"),
            row.get("subject"),
            row.get("claimed_at_ms"),
            row.get("posted_at_ms"),
            row.get("lease_token"),
        )
    })
    .collect();
    assert_eq!(
        rows,
        carried(),
        "bug: the rebuild must keep every claim's columns and give it subject ''"
    );

    let old = sqlx::raw_sql(PRE_REBUILD_CLAIM).execute(&pool).await;
    assert!(
        old.is_err(),
        "bug: an older binary's ON CONFLICT (tenant_id, user_id, purpose) must fail on the \
         rebuilt key, so a rollback skips a post rather than doubling it; it did {old:?}"
    );
}

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
        if migration.version == SUBJECT_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("bug: the claims subject migration is not in migrations_pg/");
}

#[cfg(feature = "postgresql")]
async fn postgres_rows(pool: &sqlx::PgPool) -> Vec<ClaimRow> {
    sqlx::query(
        "SELECT user_id::text AS user_id, purpose, subject, claimed_at_ms, posted_at_ms, lease_token
         FROM platform_post_claims ORDER BY user_id::text",
    )
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .map(|row| {
        (
            row.get("user_id"),
            row.get("purpose"),
            row.get("subject"),
            row.get("claimed_at_ms"),
            row.get("posted_at_ms"),
            row.get("lease_token"),
        )
    })
    .collect()
}

#[cfg(feature = "postgresql")]
async fn rebuild_on_postgres(url: &str) {
    let (pool, sql) = postgres_before_migration(url).await;
    let key: String = sqlx::query(
        "SELECT conname::text AS conname FROM pg_constraint
         WHERE conrelid = 'platform_post_claims'::regclass AND contype = 'p'",
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .get("conname");
    assert_eq!(
        key, "platform_post_claims_pkey",
        "bug: the legacy key is expected under PostgreSQL's default name"
    );
    for user in [POSTED, HELD] {
        sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1::uuid, $2, 'x')")
            .bind(user)
            .bind(format!("{user}@claims.test"))
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::raw_sql(CLAIMS).execute(&pool).await.unwrap();

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    let rows = postgres_rows(&pool).await;
    assert_eq!(
        rows,
        carried(),
        "bug: the rebuild must keep every claim's columns and give it subject ''"
    );

    // A second run finds the subject key in place and changes nothing.
    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    assert_eq!(
        postgres_rows(&pool).await,
        rows,
        "bug: a re-run of the rebuild must change nothing"
    );

    let old = sqlx::raw_sql(PRE_REBUILD_CLAIM).execute(&pool).await;
    assert!(
        old.is_err(),
        "bug: an older binary's ON CONFLICT (tenant_id, user_id, purpose) must fail on the \
         rebuilt key, so a rollback skips a post rather than doubling it; it did {old:?}"
    );
}

/// A distinct user per call; the table's foreign key needs the row.
async fn fresh_user(repos: &RepositoryRegistry) -> Uuid {
    let user = create_test_user(
        &format!("claims-{}@example.com", Uuid::new_v4()),
        Some("Claims Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

fn noon() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 18, 12, 0, 0).unwrap()
}

const LANDED: &str = "activity_landed";
const MORNING_RIDE: &str = "strava:101";
const EVENING_RIDE: &str = "garmin:202";

#[tokio::test]
async fn each_subject_is_claimed_once_and_apart_from_the_others() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let claims = &repos.platform_post_claims;
    let user_id = fresh_user(repos).await;
    let tenant = TenantId::generate();
    let lease = Duration::minutes(15);
    let claim = |subject, at: DateTime<Utc>| {
        claims.claim_platform_post_once(tenant, user_id, LANDED, subject, at - lease, at)
    };

    let morning = claim(MORNING_RIDE, noon())
        .await
        .unwrap()
        .expect("bug: the morning ride's first claim must be taken");
    assert!(
        claim(MORNING_RIDE, noon() + Duration::minutes(1))
            .await
            .unwrap()
            .is_none(),
        "bug: a second claim of the same ride must find it taken"
    );
    let evening = claim(EVENING_RIDE, noon() + Duration::minutes(2))
        .await
        .unwrap()
        .expect("bug: another ride's claim must not be blocked by the first ride's row");

    assert!(
        claims
            .mark_platform_post_posted(
                tenant,
                user_id,
                LANDED,
                MORNING_RIDE,
                &morning,
                noon() + Duration::minutes(3)
            )
            .await
            .unwrap(),
        "bug: the morning ride's claim must be marked posted"
    );
    assert!(
        !claims
            .mark_platform_post_posted(
                tenant,
                user_id,
                LANDED,
                MORNING_RIDE,
                &evening,
                noon() + Duration::minutes(3)
            )
            .await
            .unwrap(),
        "bug: one ride's token must not mark another ride's row"
    );
    assert!(
        claims
            .mark_platform_post_posted(
                tenant,
                user_id,
                LANDED,
                EVENING_RIDE,
                &evening,
                noon() + Duration::hours(6)
            )
            .await
            .unwrap(),
        "bug: the evening ride's claim must be marked posted"
    );
    assert_eq!(
        claims
            .platform_post_posted_at(tenant, user_id, LANDED)
            .await
            .unwrap(),
        Some(noon() + Duration::hours(6)),
        "bug: posted_at must answer the latest post across every subject"
    );
    assert!(
        claim(MORNING_RIDE, noon() + Duration::days(2))
            .await
            .unwrap()
            .is_none(),
        "bug: a posted ride's claim must never be retaken"
    );
}

#[tokio::test]
async fn the_day_claim_and_a_ride_claim_of_one_purpose_never_block_each_other() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let claims = &repos.platform_post_claims;
    let user_id = fresh_user(repos).await;
    let tenant = TenantId::generate();
    let midnight = Utc.with_ymd_and_hms(2026, 11, 18, 0, 0, 0).unwrap();

    let ride = claims
        .claim_platform_post_once(
            tenant,
            user_id,
            LANDED,
            MORNING_RIDE,
            noon() - Duration::minutes(15),
            noon(),
        )
        .await
        .unwrap();
    assert!(ride.is_some(), "bug: the per-ride claim must be taken");

    let day = claims
        .claim_platform_post_since(
            tenant,
            user_id,
            LANDED,
            PostPurpose::NO_SUBJECT,
            midnight,
            noon() + Duration::minutes(1),
        )
        .await
        .unwrap();
    assert!(
        day.is_some(),
        "bug: the day's claim must not be blocked by a ride's row of the same purpose"
    );
    assert!(
        claims
            .claim_platform_post_since(
                tenant,
                user_id,
                LANDED,
                PostPurpose::NO_SUBJECT,
                midnight,
                noon() + Duration::minutes(2),
            )
            .await
            .unwrap()
            .is_none(),
        "bug: the day's claim must still be taken once per day"
    );
    assert!(
        claims
            .claim_platform_post_once(
                tenant,
                user_id,
                LANDED,
                EVENING_RIDE,
                noon() - Duration::minutes(12),
                noon() + Duration::minutes(3),
            )
            .await
            .unwrap()
            .is_some(),
        "bug: a ride's claim must not be blocked by the day's row of the same purpose"
    );
}

#[tokio::test]
async fn nothing_posted_reads_as_no_landing() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let claims = &repos.platform_post_claims;
    let user_id = fresh_user(repos).await;
    let tenant = TenantId::generate();
    claims
        .claim_platform_post_once(
            tenant,
            user_id,
            LANDED,
            MORNING_RIDE,
            noon() - Duration::minutes(15),
            noon(),
        )
        .await
        .unwrap()
        .expect("bug: the ride's first claim must be taken");
    assert_eq!(
        claims
            .platform_post_posted_at(tenant, user_id, LANDED)
            .await
            .unwrap(),
        None,
        "bug: a held claim that never posted must read as no landing"
    );
}
