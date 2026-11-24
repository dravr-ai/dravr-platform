// ABOUTME: The retirement of the eight admin config categories that hold no key, and of the 22 unread keys' overrides
// ABOUTME: Replays each lane's migrations up to it, plants overrides, checks what survives; the catalog shows no empty section

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Twenty-two admin catalog keys were removed because nothing read them.
//! Five categories held only those keys, three more never had one, and all
//! eight rows were seeded by migrations, so the admin console rendered them
//! as empty sections. Migration 20261124100000 deletes those category rows and every
//! stored override of a removed key or filed under a retired category,
//! leaving the other categories and the overrides of live keys as stored.
//! Proven on a real database of each lane, and on the catalog the service
//! builds after every migration has run.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::str::FromStr;

use anyhow::Result;
use pierre_mcp_server::config::admin::service::AdminConfigService;
use pierre_runtime_context::ConfigLookupScope;
use pierre_test_support::db::{create_test_db, create_test_db_url};
use sqlx::migrate::Migrator;
#[cfg(feature = "postgresql")]
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use uuid::Uuid;

/// The migration under test; everything before it builds the tables.
const RETIRE_MIGRATION: i64 = 20_261_124_100_000;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
#[cfg(feature = "postgresql")]
static PG_MIGRATOR: Migrator = sqlx::migrate!("../../migrations_pg");

/// The categories the migration retires.
const RETIRED_CATEGORIES: [&str; 8] = [
    "cache_ttl",
    "effort_thresholds",
    "feature_flags",
    "heart_rate_zones",
    "monitoring",
    "recommendation_engine",
    "sleep_recovery",
    "weather_analysis",
];

/// Categories that lost a key but still define others, so they stay.
const KEPT_CATEGORIES: [&str; 2] = ["algorithms", "provider_garmin"];

/// Key-holding categories only the SQLite lane seeded; the PostgreSQL
/// migration seeds them, so the console lists them on both lanes.
const SEEDED_ON_POSTGRES: [&str; 4] = [
    "llm_provider",
    "provider_strava",
    "provider_garmin",
    "mcp_network",
];

/// One planted override: id, category, key.
struct Row {
    id: &'static str,
    category: &'static str,
    key: &'static str,
}

const ROWS: [Row; 12] = [
    // An override under each retired category, of one of its removed keys.
    Row {
        id: "feature-flag",
        category: "feature_flags",
        key: "feature.auto_approval_enabled",
    },
    Row {
        id: "monitoring",
        category: "monitoring",
        key: "monitoring.latency_warn_ms",
    },
    Row {
        id: "recommendation",
        category: "recommendation_engine",
        key: "recommendation.max_per_category",
    },
    Row {
        id: "sleep",
        category: "sleep_recovery",
        key: "sleep.adult_min_hours",
    },
    Row {
        id: "weather",
        category: "weather_analysis",
        key: "weather.ideal_max_celsius",
    },
    // Keys no catalog ever defined, filed under retired categories.
    Row {
        id: "heart-rate-stray",
        category: "heart_rate_zones",
        key: "heart_rate_zones.max_hr",
    },
    Row {
        id: "sleep-stray",
        category: "sleep_recovery",
        key: "sleep_recovery.sleep_debt_hours_threshold",
    },
    // Removed keys whose categories still define other keys.
    Row {
        id: "algorithm-tss",
        category: "algorithms",
        key: "algorithm.tss",
    },
    Row {
        id: "garmin-hourly",
        category: "provider_garmin",
        key: "provider.garmin_rate_limit_hourly",
    },
    // Live keys of the kept categories, and of an untouched one.
    Row {
        id: "algorithm-live",
        category: "algorithms",
        key: "algorithm.maxhr",
    },
    Row {
        id: "garmin-live",
        category: "provider_garmin",
        key: "provider.garmin_max_activities_per_request",
    },
    Row {
        id: "nutrition-live",
        category: "nutrition",
        key: "nutrition.protein_athlete_g_per_kg",
    },
];

/// The rows that survive, by id.
const SURVIVORS: [&str; 3] = ["algorithm-live", "garmin-live", "nutrition-live"];

/// Categories the PostgreSQL lane never seeded: its admin schema migration
/// seeds the first fifteen categories only, so these rows exist on SQLite
/// alone and the migration finds nothing of theirs to delete there.
#[cfg(feature = "postgresql")]
const PG_UNSEEDED: [&str; 3] = ["cache_ttl", "monitoring", "provider_garmin"];

const INSERT_SQL: &str = "INSERT INTO admin_config_overrides (id, category, config_key, \
     config_value, data_type, tenant_id, user_id, created_by, created_at, updated_at, reason) \
     VALUES ($1, $2, $3, '1', 'integer', NULL, NULL, $4, $5, $5, 'planted')";

const CATEGORY_SQL: &str = "SELECT COUNT(*) FROM admin_config_categories WHERE name = $1";

fn survivors() -> Vec<String> {
    SURVIVORS.iter().map(|id| (*id).to_owned()).collect()
}

#[tokio::test]
async fn retired_categories_and_removed_keys_overrides_are_deleted() {
    let database = create_test_db_url().await.unwrap();
    #[cfg(feature = "postgresql")]
    if database.url.starts_with("postgres") {
        retires_on_postgres(&database.url).await;
        return;
    }
    retires_on_sqlite(&database.url).await;
}

#[tokio::test]
async fn catalog_lists_no_empty_section_after_migrating() -> Result<()> {
    let db = create_test_db().await?;
    let svc = AdminConfigService::for_database(&db).await?;
    let catalog = svc.get_catalog(ConfigLookupScope::global()).await?;
    let empty: Vec<&str> = catalog
        .categories
        .iter()
        .filter(|c| c.parameters.is_empty())
        .map(|c| c.name.as_str())
        .collect();
    assert!(
        empty.is_empty(),
        "bug: the admin console would render these categories as empty sections: {empty:?}"
    );
    for retired in RETIRED_CATEGORIES {
        assert!(
            catalog.categories.iter().all(|c| c.name != retired),
            "bug: the retired {retired} category is still listed"
        );
    }
    for kept in KEPT_CATEGORIES.iter().chain(&SEEDED_ON_POSTGRES) {
        assert!(
            catalog.categories.iter().any(|c| c.name == *kept),
            "bug: {kept} still defines keys and must be listed on every lane"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

/// A database migrated up to, not including, the migration under test, and
/// that migration's SQL. One connection: every pooled connection to an
/// in-memory database is its own empty database. Foreign keys off: the
/// planted rows name an admin the test does not build.
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
        if migration.version == RETIRE_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the category-retirement migration is in migrations/");
}

async fn sqlite_category_count(pool: &sqlx::SqlitePool, name: &str) -> i64 {
    sqlx::query_scalar(CATEGORY_SQL)
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn retires_on_sqlite(url: &str) {
    let (pool, sql) = sqlite_before_migration(url).await;
    for name in RETIRED_CATEGORIES.iter().chain(&KEPT_CATEGORIES) {
        assert_eq!(
            sqlite_category_count(&pool, name).await,
            1,
            "{name} is seeded before the migration"
        );
    }
    let admin = Uuid::new_v4().to_string();
    for row in &ROWS {
        sqlx::query(INSERT_SQL)
            .bind(row.id)
            .bind(row.category)
            .bind(row.key)
            .bind(&admin)
            .bind("2026-11-01T00:00:00Z")
            .execute(&pool)
            .await
            .unwrap();
    }

    // Twice: the second run finds nothing more to remove.
    for _ in 0..2 {
        sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
        let ids: Vec<String> =
            sqlx::query_scalar("SELECT id FROM admin_config_overrides ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(ids, survivors());
        for name in RETIRED_CATEGORIES {
            assert_eq!(
                sqlite_category_count(&pool, name).await,
                0,
                "{name} retired"
            );
        }
        for name in KEPT_CATEGORIES {
            assert_eq!(sqlite_category_count(&pool, name).await, 1, "{name} kept");
        }
    }
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
        if migration.version == RETIRE_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the category-retirement migration is in migrations_pg/");
}

#[cfg(feature = "postgresql")]
async fn postgres_category_count(pool: &sqlx::PgPool, name: &str) -> i64 {
    sqlx::query_scalar(CATEGORY_SQL)
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// `created_by` carries a real foreign key here, so the admin exists before
/// any row names it.
#[cfg(feature = "postgresql")]
async fn retires_on_postgres(url: &str) {
    let (pool, sql) = postgres_before_migration(url).await;
    for name in RETIRED_CATEGORIES.iter().chain(&KEPT_CATEGORIES) {
        let seeded = i64::from(!PG_UNSEEDED.contains(name));
        assert_eq!(
            postgres_category_count(&pool, name).await,
            seeded,
            "{name} is seeded before the migration on this lane, or never"
        );
    }
    let admin = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, 'x')")
        .bind(admin)
        .bind(format!("{admin}@retire-categories.test"))
        .execute(&pool)
        .await
        .unwrap();
    for row in &ROWS {
        sqlx::query(&INSERT_SQL.replace("$5, $5", "now(), now()"))
            .bind(row.id)
            .bind(row.category)
            .bind(row.key)
            .bind(admin)
            .execute(&pool)
            .await
            .unwrap();
    }

    for _ in 0..2 {
        sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
        let ids: Vec<String> =
            sqlx::query_scalar("SELECT id FROM admin_config_overrides ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(ids, survivors());
        for name in RETIRED_CATEGORIES {
            assert_eq!(
                postgres_category_count(&pool, name).await,
                0,
                "{name} retired"
            );
        }
        for name in KEPT_CATEGORIES.iter().chain(&SEEDED_ON_POSTGRES) {
            assert_eq!(
                postgres_category_count(&pool, name).await,
                1,
                "{name} is listed after the migration"
            );
        }
    }
}
