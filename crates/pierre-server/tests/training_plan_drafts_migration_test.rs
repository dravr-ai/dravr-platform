// ABOUTME: The training-plan drafts migration: three statuses, three draft columns, one open draft per coach and athlete
// ABOUTME: Replays each lane's migrations up to it, plants plans of every status, and checks every column, default and index survives

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! On `SQLite` the status CHECK cannot change in place, so 20261016120000
//! rebuilds `training_plans`: a new table, every row copied, the old one
//! dropped and the new one renamed over it, and every index recreated. A
//! rebuild that dropped a column added since July, a default, or the
//! one-season index would be silent — nothing fails, a plan just loses its
//! flavour or an athlete gains a second season — so each lane proves the rows
//! and the rules survive on a real database. On Postgres the CHECK is dropped
//! and re-added, and the migration also proves it runs twice.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::str::FromStr;

use pierre_test_support::db::create_test_db_url;
use sqlx::error::DatabaseError;
use sqlx::migrate::Migrator;
#[cfg(feature = "postgresql")]
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;
use uuid::Uuid;

/// The migration under test; everything before it builds the prior shape.
const DRAFTS_MIGRATION: i64 = 20_261_016_120_000;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
#[cfg(feature = "postgresql")]
static PG_MIGRATOR: Migrator = sqlx::migrate!("../../migrations_pg");

/// Plant one outline with every column the table held before the migration.
const PLAN_SQL: &str = "INSERT INTO training_plans (id, tenant_id, user_id, author_agent_id, \
     goal_fact_id, goal_race_json, races_json, strategy, phases_json, status, supersedes_id, \
     source_conversation_id, created_at, updated_at, flavour_json, season_start, season_end, \
     first_party_only) \
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)";

/// Every pre-migration column, in [`PlanRow`]'s order, then the draft ones.
const SNAPSHOT_SQL: &str = "SELECT id, tenant_id, user_id, author_agent_id, goal_fact_id, \
     goal_race_json, races_json, strategy, phases_json, status, supersedes_id, \
     source_conversation_id, created_at, updated_at, flavour_json, season_start, season_end, \
     first_party_only, drafted_by_user_id, draft_base_plan_id, outline_drafted \
     FROM training_plans WHERE strategy <> 'bare' ORDER BY id";

/// A row naming only the columns without a default, plus a status and the
/// coach, so each check runs against the schema's own defaults and rules.
const BARE_SQL: &str = "INSERT INTO training_plans (id, tenant_id, user_id, goal_race_json, \
     strategy, phases_json, created_at, updated_at, status, drafted_by_user_id) \
     VALUES ($1, $2, $3, '{}', 'bare', '[]', 1, 1, $4, $5)";

/// What a bare row of the athlete's season reads back as.
const DEFAULTS_SQL: &str = "SELECT author_agent_id, races_json, first_party_only, \
     drafted_by_user_id, draft_base_plan_id, outline_drafted \
     FROM training_plans WHERE strategy = 'bare' AND status = 'active' AND user_id = $1";

/// One stored outline: every column it had before the migration, then the
/// three the migration adds.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PlanRow {
    id: String,
    tenant_id: String,
    user_id: String,
    author_agent_id: String,
    goal_fact_id: Option<String>,
    goal_race_json: String,
    races_json: String,
    strategy: String,
    phases_json: String,
    status: String,
    supersedes_id: Option<String>,
    source_conversation_id: Option<String>,
    created_at: i64,
    updated_at: i64,
    flavour_json: Option<String>,
    season_start: Option<String>,
    season_end: Option<String>,
    first_party_only: bool,
    drafted_by_user_id: Option<String>,
    draft_base_plan_id: Option<String>,
    outline_drafted: bool,
}

/// The athletes, and what they hold before the migration.
struct World {
    tenant: String,
    /// Holds a live season and three outlines of history.
    first: String,
    /// Holds a live season stamped first-party-only.
    second: String,
    /// Holds nothing.
    third: String,
}

impl World {
    fn new() -> Self {
        Self {
            tenant: Uuid::new_v4().to_string(),
            first: Uuid::new_v4().to_string(),
            second: Uuid::new_v4().to_string(),
            third: Uuid::new_v4().to_string(),
        }
    }

    /// Four outlines of the first athlete — the live season, the one it
    /// replaced, one set aside and one completed — and the second athlete's
    /// season. Every optional column is set on some row and empty on another.
    fn plans(&self) -> Vec<PlanRow> {
        let row = |id: &str, athlete: &str, status: &str, at: i64| {
            PlanRow {
            id: id.to_owned(),
            tenant_id: self.tenant.clone(),
            user_id: athlete.to_owned(),
            author_agent_id: format!("agent-of-{id}"),
            goal_fact_id: Some(format!("fact-{id}")),
            goal_race_json: format!(
                "{{\"name\":\"Race {id}\",\"date\":\"2026-12-01\",\"discipline\":\"run\",\"priority\":\"A\"}}"
            ),
            races_json: format!(
                "[{{\"name\":\"Tune-up {id}\",\"date\":\"2026-10-01\",\"discipline\":\"run\",\"priority\":\"B\"}}]"
            ),
            strategy: format!("strategy of {id}"),
            phases_json: "[]".to_owned(),
            status: status.to_owned(),
            supersedes_id: None,
            source_conversation_id: Some(format!("conv-{id}")),
            created_at: at,
            updated_at: at + 5,
            flavour_json: Some(format!("{{\"id\":\"flavour-{id}\"}}")),
            season_start: Some("2026-07-06".to_owned()),
            season_end: Some("2026-12-06".to_owned()),
            first_party_only: false,
            drafted_by_user_id: None,
            draft_base_plan_id: None,
            outline_drafted: false,
        }
        };
        let mut rows = vec![
            row("a-done", &self.first, "completed", 50),
            row("a-live", &self.first, "active", 300),
            row("a-replaced", &self.first, "superseded", 200),
            row("a-set-aside", &self.first, "abandoned", 100),
            row("b-live", &self.second, "active", 400),
        ];
        rows[1].supersedes_id = Some("a-replaced".to_owned());
        rows[3].goal_fact_id = None;
        rows[3].flavour_json = None;
        rows[3].season_start = None;
        rows[3].season_end = None;
        rows[3].source_conversation_id = None;
        rows[4].first_party_only = true;
        rows[4].author_agent_id = String::new();
        rows
    }
}

/// Plant every row of `$world` on `$pool`.
macro_rules! plant {
    ($pool:expr, $world:expr) => {
        for row in $world.plans() {
            sqlx::query(PLAN_SQL)
                .bind(&row.id)
                .bind(&row.tenant_id)
                .bind(&row.user_id)
                .bind(&row.author_agent_id)
                .bind(&row.goal_fact_id)
                .bind(&row.goal_race_json)
                .bind(&row.races_json)
                .bind(&row.strategy)
                .bind(&row.phases_json)
                .bind(&row.status)
                .bind(&row.supersedes_id)
                .bind(&row.source_conversation_id)
                .bind(row.created_at)
                .bind(row.updated_at)
                .bind(&row.flavour_json)
                .bind(&row.season_start)
                .bind(&row.season_end)
                .bind(row.first_party_only)
                .execute($pool)
                .await
                .unwrap();
        }
    };
}

/// Every planted outline read back from `$pool`, by id.
macro_rules! snapshot {
    ($pool:expr) => {
        sqlx::query(SNAPSHOT_SQL)
            .fetch_all($pool)
            .await
            .unwrap()
            .iter()
            .map(|r| PlanRow {
                id: r.get("id"),
                tenant_id: r.get("tenant_id"),
                user_id: r.get("user_id"),
                author_agent_id: r.get("author_agent_id"),
                goal_fact_id: r.get("goal_fact_id"),
                goal_race_json: r.get("goal_race_json"),
                races_json: r.get("races_json"),
                strategy: r.get("strategy"),
                phases_json: r.get("phases_json"),
                status: r.get("status"),
                supersedes_id: r.get("supersedes_id"),
                source_conversation_id: r.get("source_conversation_id"),
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
                flavour_json: r.get("flavour_json"),
                season_start: r.get("season_start"),
                season_end: r.get("season_end"),
                first_party_only: r.get("first_party_only"),
                drafted_by_user_id: r.get("drafted_by_user_id"),
                draft_base_plan_id: r.get("draft_base_plan_id"),
                outline_drafted: r.get("outline_drafted"),
            })
            .collect::<Vec<PlanRow>>()
    };
}

/// Insert a bare row for `$athlete` on `$pool`, as `Result<(), sqlx::Error>`.
macro_rules! bare {
    ($pool:expr, $world:expr, $athlete:expr, $status:expr, $coach:expr) => {
        sqlx::query(BARE_SQL)
            .bind(Uuid::new_v4().to_string())
            .bind(&$world.tenant)
            .bind($athlete)
            .bind($status)
            .bind($coach)
            .execute($pool)
            .await
            .map(|_| ())
    };
}

/// The rules the migration leaves in force on `$pool`: one active season per
/// athlete, a draft names its coach, one open draft per coach and athlete,
/// a closed status vocabulary — and a bare row takes the table's defaults.
macro_rules! the_rules_hold {
    ($pool:expr, $world:expr) => {{
        let world = &$world;
        let none: Option<&str> = None;
        let coach = |id: &'static str| Some(id);

        // A second active season for an athlete who has one is refused; an
        // athlete with none lays one.
        assert_unique(bare!($pool, world, &world.first, "active", none));
        bare!($pool, world, &world.third, "active", none).unwrap();

        // A draft that names no coach is refused; one that does is kept.
        assert_check(bare!($pool, world, &world.first, "draft", none));
        bare!($pool, world, &world.first, "draft", coach("coach-one")).unwrap();
        // One open draft per coach and athlete; another coach opens their
        // own, and closed drafts do not count.
        assert_unique(bare!(
            $pool,
            world,
            &world.first,
            "draft",
            coach("coach-one")
        ));
        bare!($pool, world, &world.first, "draft", coach("coach-two")).unwrap();
        bare!($pool, world, &world.first, "published", coach("coach-one")).unwrap();
        bare!($pool, world, &world.first, "discarded", coach("coach-one")).unwrap();
        // The same coach drafts another athlete's plan on its own row.
        bare!($pool, world, &world.second, "draft", coach("coach-one")).unwrap();
        // The vocabulary is closed.
        assert_check(bare!($pool, world, &world.first, "bogus", none));

        let defaults: (String, String, bool, Option<String>, Option<String>, bool) =
            sqlx::query(DEFAULTS_SQL)
                .bind(&world.third)
                .fetch_one($pool)
                .await
                .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4), r.get(5)))
                .unwrap();
        assert_eq!(
            defaults,
            (String::new(), "[]".to_owned(), false, None, None, false)
        );
    }};
}

fn assert_unique(result: Result<(), sqlx::Error>) {
    let Err(e) = result else {
        panic!("the insert must be refused by a unique index");
    };
    assert!(
        e.as_database_error()
            .is_some_and(DatabaseError::is_unique_violation),
        "{e}"
    );
}

fn assert_check(result: Result<(), sqlx::Error>) {
    let Err(e) = result else {
        panic!("the insert must be refused by a CHECK");
    };
    assert!(
        e.as_database_error()
            .is_some_and(DatabaseError::is_check_violation),
        "{e}"
    );
}

#[tokio::test]
async fn every_plan_column_default_and_index_survives_the_drafts_migration() {
    let database = create_test_db_url().await.unwrap();
    #[cfg(feature = "postgresql")]
    if database.url.starts_with("postgres") {
        survives_on_postgres(&database.url).await;
        return;
    }
    survives_on_sqlite(&database.url).await;
}

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

/// A database migrated up to, not including, the migration under test, and
/// that migration's SQL. One connection: every pooled connection to an
/// in-memory database is its own empty database.
async fn sqlite_before_migration(url: &str) -> (sqlx::SqlitePool, String) {
    let options = SqliteConnectOptions::from_str(url).unwrap();
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    for migration in MIGRATOR.iter() {
        if migration.version == DRAFTS_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the drafts migration is in migrations/");
}

async fn survives_on_sqlite(url: &str) {
    let (pool, sql) = sqlite_before_migration(url).await;
    let world = World::new();
    plant!(&pool, world);

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();

    assert_eq!(snapshot!(&pool), world.plans(), "every row, every column");

    let indexes: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'training_plans' \
         AND name LIKE 'idx_%' ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        indexes,
        vec![
            "idx_training_plans_lookup",
            "idx_training_plans_one_draft",
            "idx_training_plans_one_season",
        ]
    );

    the_rules_hold!(&pool, world);
}

// ---------------------------------------------------------------------------
// PostgreSQL
// ---------------------------------------------------------------------------

/// A database migrated up to, not including, the migration under test, and
/// that migration's SQL. The factory hands out a clone of the migrated
/// template; start again from an empty schema so the prior shape exists.
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
        if migration.version == DRAFTS_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the drafts migration is in migrations_pg/");
}

#[cfg(feature = "postgresql")]
async fn survives_on_postgres(url: &str) {
    let (pool, sql) = postgres_before_migration(url).await;
    let world = World::new();
    plant!(&pool, world);

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    // Every statement is guarded, so a second pass changes nothing.
    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();

    assert_eq!(snapshot!(&pool), world.plans(), "every row, every column");

    let indexes: Vec<String> = sqlx::query_scalar(
        "SELECT indexname::text FROM pg_indexes WHERE tablename = 'training_plans' \
         AND indexname LIKE 'idx_%' ORDER BY indexname",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        indexes,
        vec![
            "idx_training_plans_lookup",
            "idx_training_plans_one_draft",
            "idx_training_plans_one_season",
        ]
    );

    the_rules_hold!(&pool, world);
}
