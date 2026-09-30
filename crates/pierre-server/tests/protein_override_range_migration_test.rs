// ABOUTME: The protein-override range migration: athlete-protein rows outside 1.2-2.0 g/kg/day go, the rest stay
// ABOUTME: Replays each lane's migrations up to it, plants overrides of every scope, and checks which survive

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `nutrition.protein_athlete_g_per_kg` was read by nothing while the
//! catalogue accepted 1.4-2.5. It now sets `calculate_daily_nutrition`'s
//! athlete protein target within 1.2-2.0, and the reader refuses a stored
//! value outside that range. A row written in the old range above 2.0 would
//! fail the tool for everyone it covers, so migration 20260929201700 removes
//! the athlete-protein rows outside the range and leaves every other row —
//! in-range ones included — as stored. Proven on a real database of each lane.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::str::FromStr;

use pierre_test_support::db::create_test_db_url;
use sqlx::migrate::Migrator;
#[cfg(feature = "postgresql")]
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use uuid::Uuid;

/// The migration under test; everything before it builds the table.
const RANGE_MIGRATION: i64 = 20_260_929_201_700;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
#[cfg(feature = "postgresql")]
static PG_MIGRATOR: Migrator = sqlx::migrate!("../../migrations_pg");

const KEY: &str = "nutrition.protein_athlete_g_per_kg";

/// Which scope a planted row belongs to.
#[derive(Clone, Copy)]
enum Scope {
    Global,
    Tenant(usize),
    User,
}

/// One planted override: id, category, key, JSON-encoded value, scope.
struct Row {
    id: &'static str,
    category: &'static str,
    key: &'static str,
    value: &'static str,
    scope: Scope,
}

const ROWS: [Row; 7] = [
    // Written under the old 1.4-2.5 range, above the new ceiling.
    Row {
        id: "global-above",
        category: "nutrition",
        key: KEY,
        value: "2.4",
        scope: Scope::Global,
    },
    // Hand-edited below the floor.
    Row {
        id: "tenant-below",
        category: "nutrition",
        key: KEY,
        value: "1.1",
        scope: Scope::Tenant(0),
    },
    // Not a number at all.
    Row {
        id: "tenant-not-a-number",
        category: "nutrition",
        key: KEY,
        value: "\"high\"",
        scope: Scope::Tenant(1),
    },
    // Inside the range: kept, and from now on applied.
    Row {
        id: "tenant-inside",
        category: "nutrition",
        key: KEY,
        value: "1.3",
        scope: Scope::Tenant(2),
    },
    // Both bounds are inside.
    Row {
        id: "user-ceiling",
        category: "nutrition",
        key: KEY,
        value: "2.0",
        scope: Scope::User,
    },
    Row {
        id: "tenant-floor",
        category: "nutrition",
        key: KEY,
        value: "1.2",
        scope: Scope::Tenant(3),
    },
    // Another knob with the same number is none of this migration's business.
    Row {
        id: "other-knob",
        category: "sleep_recovery",
        key: "sleep_recovery.sleep_debt_hours_threshold",
        value: "2.4",
        scope: Scope::Global,
    },
];

/// The rows that survive, by id.
const SURVIVORS: [&str; 4] = [
    "other-knob",
    "tenant-floor",
    "tenant-inside",
    "user-ceiling",
];

const INSERT_SQL: &str = "INSERT INTO admin_config_overrides (id, category, config_key, \
     config_value, data_type, tenant_id, user_id, created_by, created_at, updated_at, reason) \
     VALUES ($1, $2, $3, $4, 'float', $5, $6, $7, $8, $8, 'planted')";

/// The ids planted rows reference: four tenants and one user, who is also
/// the admin who wrote every row.
struct World {
    tenants: [Uuid; 4],
    user: Uuid,
}

impl World {
    fn new() -> Self {
        Self {
            tenants: [
                Uuid::new_v4(),
                Uuid::new_v4(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            ],
            user: Uuid::new_v4(),
        }
    }

    fn tenant_of(&self, scope: Scope) -> Option<String> {
        match scope {
            Scope::Tenant(i) => Some(self.tenants[i].to_string()),
            Scope::Global | Scope::User => None,
        }
    }

    fn user_of(&self, scope: Scope) -> Option<Uuid> {
        matches!(scope, Scope::User).then_some(self.user)
    }
}

fn survivors() -> Vec<String> {
    SURVIVORS.iter().map(|id| (*id).to_owned()).collect()
}

#[tokio::test]
async fn only_athlete_protein_rows_outside_the_range_are_removed() {
    let database = create_test_db_url().await.unwrap();
    #[cfg(feature = "postgresql")]
    if database.url.starts_with("postgres") {
        removes_out_of_range_rows_on_postgres(&database.url).await;
        return;
    }
    removes_out_of_range_rows_on_sqlite(&database.url).await;
}

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

/// A database migrated up to, not including, the migration under test, and
/// that migration's SQL. One connection: every pooled connection to an
/// in-memory database is its own empty database. Foreign keys off: the
/// planted rows name tenants and a user the test does not build.
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
        if migration.version == RANGE_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the protein-range migration is in migrations/");
}

async fn removes_out_of_range_rows_on_sqlite(url: &str) {
    let (pool, sql) = sqlite_before_migration(url).await;
    let world = World::new();
    for row in &ROWS {
        sqlx::query(INSERT_SQL)
            .bind(row.id)
            .bind(row.category)
            .bind(row.key)
            .bind(row.value)
            .bind(world.tenant_of(row.scope))
            .bind(world.user_of(row.scope).map(|u| u.to_string()))
            .bind(world.user.to_string())
            .bind("2026-09-01T00:00:00Z")
            .execute(&pool)
            .await
            .unwrap();
    }

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();

    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM admin_config_overrides ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(ids, survivors());

    // Idempotent: a second run finds nothing more to remove.
    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    let again: Vec<String> =
        sqlx::query_scalar("SELECT id FROM admin_config_overrides ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(again, survivors());
}

// ---------------------------------------------------------------------------
// PostgreSQL
// ---------------------------------------------------------------------------

/// A database migrated up to, not including, the migration under test, and
/// that migration's SQL. The factory hands out a clone of the migrated
/// template; start again from an empty schema.
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
        if migration.version == RANGE_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the protein-range migration is in migrations_pg/");
}

/// `created_by` and `user_id` carry real foreign keys here, so the admin
/// exists before any row names them; `tenant_id` is free text on this lane.
#[cfg(feature = "postgresql")]
async fn removes_out_of_range_rows_on_postgres(url: &str) {
    let (pool, sql) = postgres_before_migration(url).await;
    let world = World::new();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, 'x')")
        .bind(world.user)
        .bind(format!("{}@protein.test", world.user))
        .execute(&pool)
        .await
        .unwrap();
    for row in &ROWS {
        sqlx::query(&INSERT_SQL.replace("$8, $8", "now(), now()"))
            .bind(row.id)
            .bind(row.category)
            .bind(row.key)
            .bind(row.value)
            .bind(world.tenant_of(row.scope))
            .bind(world.user_of(row.scope))
            .bind(world.user)
            .execute(&pool)
            .await
            .unwrap();
    }

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();

    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM admin_config_overrides ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(ids, survivors());

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    let again: Vec<String> =
        sqlx::query_scalar("SELECT id FROM admin_config_overrides ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(again, survivors());
}
