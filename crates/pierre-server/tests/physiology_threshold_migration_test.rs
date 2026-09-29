// ABOUTME: The one-threshold-source migration: LTHR kept, lactate threshold converted to a VO2max fraction
// ABOUTME: Replays each lane's migrations up to it, plants profiles and overrides, and checks every resulting row

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Before 20260929190500 `set_physiology` stored the lactate threshold as a
//! fraction of max HR and the TSS engine used `max_hr * fraction` as the LTHR,
//! while `analyze_training_load` and the recovery tools read FTP, threshold HR,
//! max and resting HR and weight from `update_user_configuration`'s overrides.
//! The migration keeps each LTHR a row described as a measured `threshold_hr`,
//! converts the fraction to one of `VO2max` (Swain et al. 1994), and moves the
//! overrides onto the profile of every tenant the athlete belongs to where the
//! profile has no value and the number passes `set_physiology`'s checks. A
//! wrong conversion is silent — nothing fails, the athlete's load just moves —
//! so it is proven on a real database of each lane.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::array;
use std::str::FromStr;

use pierre_database::database::test_utils::create_test_db_url;
use serde_json::{json, Value};
use sqlx::migrate::Migrator;
#[cfg(feature = "postgresql")]
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;
use uuid::Uuid;

/// The migration under test; everything before it builds the legacy shape.
const THRESHOLD_SOURCE_MIGRATION: i64 = 20_260_929_190_500;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
#[cfg(feature = "postgresql")]
static PG_MIGRATOR: Migrator = sqlx::migrate!("../../migrations_pg");

/// One legacy profile row: `(athlete, tenant, ftp, max_hr, resting_hr,
/// lactate_threshold_percentage as a fraction of max HR)`.
type LegacyProfile = (
    usize,
    usize,
    Option<i32>,
    Option<i32>,
    Option<i32>,
    Option<f64>,
);

/// The legacy world. Athletes by index:
///
/// - 0 (A): a profile at 0.88 of a 190 max, resting 50, FTP 250; overrides of
///   FTP 300, threshold HR 150 and weight 70, a `VO2max` and an age nothing read
///   from the configuration, plus a catalogue key. The LTHR it described (167)
///   is kept, the profile's own FTP and LTHR win, the weight moves, the unread
///   `VO2max` and age are stripped without moving, the catalogue key stays.
/// - 1 (B): 0.70 of a 150 max — LTHR 105 kept; 0.508 of `VO2max` clamps to 0.65.
/// - 2 (C): 0.66 of a 140 max — LTHR 92 is under 100, so it is not kept.
/// - 3 (D): no profile, a member of both tenants; every override moves into a
///   new profile in each.
/// - 4 (E): no profile; an FTP out of range and a max HR that is a string —
///   nothing moves, both are stripped.
/// - 5 (F): a 180 max and no resting HR; an override threshold HR of 185
///   would sit above the max and stays out, resting 45 moves.
struct World {
    tenants: [Uuid; 2],
    athletes: [Uuid; 6],
}

impl World {
    fn new() -> Self {
        Self {
            tenants: [Uuid::new_v4(), Uuid::new_v4()],
            athletes: array::from_fn(|_| Uuid::new_v4()),
        }
    }

    fn memberships() -> [(usize, usize); 7] {
        [(0, 0), (1, 0), (2, 0), (3, 0), (3, 1), (4, 0), (5, 0)]
    }

    fn profiles() -> [LegacyProfile; 4] {
        [
            (0, 0, Some(250), Some(190), Some(50), Some(0.88)),
            (1, 0, None, Some(150), None, Some(0.70)),
            (2, 0, None, Some(140), None, Some(0.66)),
            (5, 0, None, Some(180), None, None),
        ]
    }

    fn configurations() -> [(usize, Value); 4] {
        [
            (
                0,
                json!({"active_profile": "custom", "session_overrides": {
                    "ftp": 300, "threshold_hr": 150, "weight_kg": 70,
                    "vo2_max": 55.0, "age": 41,
                    "heart_rate.anaerobic_threshold": 88.0
                }}),
            ),
            (
                3,
                json!({"session_overrides": {
                    "ftp": 280, "max_hr": 185, "resting_hr": 48,
                    "lactate_threshold_hr": 168, "weight": 71.5,
                    "pace.easy_zone_low": 0.6
                }}),
            ),
            (
                4,
                json!({"session_overrides": {"ftp": 5000, "max_hr": "190"}}),
            ),
            (
                5,
                json!({"session_overrides": {"threshold_hr": 185, "resting_hr": 45}}),
            ),
        ]
    }
}

/// One profile row after the migration, by athlete and tenant index.
#[derive(Debug, PartialEq)]
struct Migrated {
    athlete: usize,
    tenant: usize,
    ftp: Option<i64>,
    max_hr: Option<i64>,
    resting_hr: Option<i64>,
    threshold_hr: Option<i64>,
    /// Rounded to four decimals.
    lactate: Option<f64>,
    weight: Option<f64>,
    fitness_level: String,
    primary_sport: String,
}

/// One expected row: `(athlete, tenant, ftp, max_hr, resting_hr,
/// threshold_hr, lactate_threshold_percentage, weight)`.
type ExpectedRow = (
    usize,
    usize,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<f64>,
    Option<f64>,
);

/// Every profile row after the migration, ordered by athlete then tenant.
fn expected() -> Vec<Migrated> {
    let rows: [ExpectedRow; 6] = [
        // (0.88 - 0.37182) / 0.6463 = 0.786291
        (
            0,
            0,
            Some(250),
            Some(190),
            Some(50),
            Some(167),
            Some(0.7863),
            Some(70.0),
        ),
        (1, 0, None, Some(150), None, Some(105), Some(0.65), None),
        (2, 0, None, Some(140), None, None, Some(0.65), None),
        (
            3,
            0,
            Some(280),
            Some(185),
            Some(48),
            Some(168),
            None,
            Some(71.5),
        ),
        (
            3,
            1,
            Some(280),
            Some(185),
            Some(48),
            Some(168),
            None,
            Some(71.5),
        ),
        (5, 0, None, Some(180), Some(45), None, None, None),
    ];
    rows.into_iter()
        .map(
            |(athlete, tenant, ftp, max_hr, resting_hr, threshold_hr, lactate, weight)| Migrated {
                athlete,
                tenant,
                ftp,
                max_hr,
                resting_hr,
                threshold_hr,
                lactate,
                weight,
                fitness_level: "\"Recreational\"".to_owned(),
                primary_sport: "\"run\"".to_owned(),
            },
        )
        .collect()
}

/// Each configuration's overrides after the migration, by athlete index.
fn expected_overrides() -> Vec<(usize, Value)> {
    vec![
        (0, json!({"heart_rate.anaerobic_threshold": 88.0})),
        (3, json!({"pace.easy_zone_low": 0.6})),
        (4, json!({})),
        (5, json!({})),
    ]
}

fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

fn index_of(ids: &[Uuid], id: &str) -> usize {
    let id = Uuid::parse_str(id).unwrap();
    ids.iter().position(|x| *x == id).unwrap()
}

#[tokio::test]
async fn thresholds_move_to_the_profile_in_the_units_every_reader_uses() {
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

/// A database migrated up to, not including, the migration under test, and
/// that migration's SQL. One connection: every pooled connection to an
/// in-memory database is its own empty database. Foreign keys off: the planted
/// rows name users and tenants the test does not build.
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
        if migration.version == THRESHOLD_SOURCE_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the threshold-source migration is in migrations/");
}

async fn plant_sqlite(pool: &sqlx::SqlitePool, world: &World) {
    for (athlete, tenant) in World::memberships() {
        sqlx::query(
            "INSERT INTO tenant_users (id, tenant_id, user_id, role, invited_at) \
             VALUES ($1, $2, $3, 'member', '2026-01-01T00:00:00Z')",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(world.tenants[tenant].to_string())
        .bind(world.athletes[athlete].to_string())
        .execute(pool)
        .await
        .unwrap();
    }
    for (athlete, tenant, ftp, max_hr, resting_hr, lactate) in World::profiles() {
        sqlx::query(
            "INSERT INTO user_physiological_profiles (user_id, tenant_id, ftp_watts, max_hr, \
             resting_hr, lactate_threshold_percentage, fitness_level, primary_sport) \
             VALUES ($1, $2, $3, $4, $5, $6, '\"Recreational\"', '\"run\"')",
        )
        .bind(world.athletes[athlete].to_string())
        .bind(world.tenants[tenant].to_string())
        .bind(ftp)
        .bind(max_hr)
        .bind(resting_hr)
        .bind(lactate)
        .execute(pool)
        .await
        .unwrap();
    }
    for (athlete, document) in World::configurations() {
        sqlx::query(
            "INSERT INTO user_configurations (user_id, config_data, created_at, updated_at) \
             VALUES ($1, $2, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(world.athletes[athlete].to_string())
        .bind(document.to_string())
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn migrates_on_sqlite(url: &str) {
    let (pool, sql) = sqlite_before_migration(url).await;
    let world = World::new();
    plant_sqlite(&pool, &world).await;

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();

    let mut rows: Vec<Migrated> = sqlx::query(
        "SELECT user_id, tenant_id, ftp_watts, max_hr, resting_hr, threshold_hr, \
         lactate_threshold_percentage, weight, fitness_level, primary_sport \
         FROM user_physiological_profiles",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .iter()
    .map(|r| Migrated {
        athlete: index_of(&world.athletes, &r.get::<String, _>(0)),
        tenant: index_of(&world.tenants, &r.get::<String, _>(1)),
        ftp: r.get(2),
        max_hr: r.get(3),
        resting_hr: r.get(4),
        threshold_hr: r.get(5),
        lactate: r.get::<Option<f64>, _>(6).map(round4),
        weight: r.get(7),
        fitness_level: r.get(8),
        primary_sport: r.get(9),
    })
    .collect();
    rows.sort_by_key(|m| (m.athlete, m.tenant));
    assert_eq!(rows, expected());

    let mut overrides: Vec<(usize, Value)> =
        sqlx::query("SELECT user_id, config_data FROM user_configurations")
            .fetch_all(&pool)
            .await
            .unwrap()
            .iter()
            .map(|r| {
                let document: Value = serde_json::from_str(&r.get::<String, _>(1)).unwrap();
                (
                    index_of(&world.athletes, &r.get::<String, _>(0)),
                    document["session_overrides"].clone(),
                )
            })
            .collect();
    overrides.sort_by_key(|(athlete, _)| *athlete);
    assert_eq!(overrides, expected_overrides());
}

// ---------------------------------------------------------------------------
// PostgreSQL
// ---------------------------------------------------------------------------

/// A database migrated up to, not including, the migration under test, and
/// that migration's SQL. The factory hands out a clone of the migrated
/// template; start again from an empty schema so the legacy shape exists.
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
        if migration.version == THRESHOLD_SOURCE_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the threshold-source migration is in migrations_pg/");
}

/// Foreign keys are real here: users and tenants exist before memberships
/// and configurations can name them.
#[cfg(feature = "postgresql")]
async fn plant_postgres(pool: &sqlx::PgPool, world: &World) {
    for athlete in world.athletes {
        sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, 'x')")
            .bind(athlete)
            .bind(format!("{athlete}@threshold.test"))
            .execute(pool)
            .await
            .unwrap();
    }
    for tenant in world.tenants {
        sqlx::query("INSERT INTO tenants (id, name, slug) VALUES ($1, 'Threshold', $2)")
            .bind(tenant)
            .bind(format!("threshold-{tenant}"))
            .execute(pool)
            .await
            .unwrap();
    }
    for (athlete, tenant) in World::memberships() {
        sqlx::query(
            "INSERT INTO tenant_users (tenant_id, user_id, invited_at) VALUES ($1, $2, now())",
        )
        .bind(world.tenants[tenant])
        .bind(world.athletes[athlete])
        .execute(pool)
        .await
        .unwrap();
    }
    for (athlete, tenant, ftp, max_hr, resting_hr, lactate) in World::profiles() {
        sqlx::query(
            "INSERT INTO user_physiological_profiles (user_id, tenant_id, ftp_watts, max_hr, \
             resting_hr, lactate_threshold_percentage, fitness_level, primary_sport) \
             VALUES ($1, $2, $3, $4, $5, $6, '\"Recreational\"', '\"run\"')",
        )
        .bind(world.athletes[athlete])
        .bind(world.tenants[tenant])
        .bind(ftp)
        .bind(max_hr)
        .bind(resting_hr)
        .bind(lactate)
        .execute(pool)
        .await
        .unwrap();
    }
    for (athlete, document) in World::configurations() {
        sqlx::query(
            "INSERT INTO user_configurations (user_id, config_data, created_at, updated_at) \
             VALUES ($1, $2, now(), now())",
        )
        .bind(world.athletes[athlete])
        .bind(document.to_string())
        .execute(pool)
        .await
        .unwrap();
    }
}

#[cfg(feature = "postgresql")]
async fn migrates_on_postgres(url: &str) {
    let (pool, sql) = postgres_before_migration(url).await;
    let world = World::new();
    plant_postgres(&pool, &world).await;

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();

    let mut rows: Vec<Migrated> = sqlx::query(
        "SELECT user_id::text, tenant_id::text, ftp_watts, max_hr, resting_hr, threshold_hr, \
         lactate_threshold_percentage, weight, fitness_level, primary_sport \
         FROM user_physiological_profiles",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .iter()
    .map(|r| Migrated {
        athlete: index_of(&world.athletes, &r.get::<String, _>(0)),
        tenant: index_of(&world.tenants, &r.get::<String, _>(1)),
        ftp: r.get::<Option<i32>, _>(2).map(i64::from),
        max_hr: r.get::<Option<i32>, _>(3).map(i64::from),
        resting_hr: r.get::<Option<i32>, _>(4).map(i64::from),
        threshold_hr: r.get::<Option<i32>, _>(5).map(i64::from),
        lactate: r.get::<Option<f64>, _>(6).map(round4),
        weight: r.get(7),
        fitness_level: r.get(8),
        primary_sport: r.get(9),
    })
    .collect();
    rows.sort_by_key(|m| (m.athlete, m.tenant));
    assert_eq!(rows, expected());

    let mut overrides: Vec<(usize, Value)> =
        sqlx::query("SELECT user_id::text, config_data FROM user_configurations")
            .fetch_all(&pool)
            .await
            .unwrap()
            .iter()
            .map(|r| {
                let document: Value = serde_json::from_str(&r.get::<String, _>(1)).unwrap();
                (
                    index_of(&world.athletes, &r.get::<String, _>(0)),
                    document["session_overrides"].clone(),
                )
            })
            .collect();
    overrides.sort_by_key(|(athlete, _)| *athlete);
    assert_eq!(overrides, expected_overrides());
}
