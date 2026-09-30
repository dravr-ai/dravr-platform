// ABOUTME: The one-season migration: one active training plan per athlete and tenant, the rest set aside as abandoned
// ABOUTME: Replays each lane's migrations up to it, plants athletes holding several plans, and checks which one each keeps

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Before 20260929180000 a plan was keyed per (tenant, user, agent), so an
//! athlete who used several agents could hold several active plans. The
//! migration keeps one per (tenant, user) — the one the athlete's home screen
//! and `/plan` showed: the selected agent's, else the agent-agnostic one,
//! else the most recently touched, where a later week counts — and sets the
//! others to `abandoned` with their weeks intact. It is proven on a real
//! database of each lane, because a ranking that picks the wrong season is
//! silent: nothing fails, the athlete just sees another plan.
//!
//! Each lane has its own migration set, so the factory's URL decides which
//! one is replayed.

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

/// The migration under test; everything before it builds the legacy shape.
const ONE_SEASON_MIGRATION: i64 = 20_260_929_180_000;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
#[cfg(feature = "postgresql")]
static PG_MIGRATOR: Migrator = sqlx::migrate!("../../migrations_pg");

/// Plant one legacy outline: `agent_slug` is the pre-migration column.
const PLAN_SQL: &str = "INSERT INTO training_plans (id, tenant_id, user_id, agent_slug, \
     goal_race_json, strategy, phases_json, status, created_at, updated_at) \
     VALUES ($1, $2, $3, $4, '{\"name\":\"R\",\"date\":\"2026-12-01\",\"discipline\":\"run\",\"priority\":\"A\"}', \
     $5, '[]', $6, $7, $7)";

/// Plant one legacy week.
const WEEK_SQL: &str = "INSERT INTO training_plan_weeks (id, tenant_id, user_id, plan_id, \
     week_start, days_json, status, created_at, updated_at) \
     VALUES ($1, $2, $3, $4, $5, '[]', $6, $7, $7)";

/// One legacy outline to plant.
struct Plan {
    id: &'static str,
    tenant: usize,
    athlete: usize,
    agent: &'static str,
    status: &'static str,
    created_at: i64,
}

/// One legacy week to plant.
struct Week {
    id: &'static str,
    plan: &'static str,
    start: &'static str,
    status: &'static str,
    created_at: i64,
}

/// The legacy world: two tenants, four athletes, and who selected which agent.
///
/// - Athlete 0 selected `agent-sel`; a newer plan by `agent-other` must lose.
/// - Athlete 1 selected `agent-idle`, which holds no plan; the agnostic plan
///   must beat a newer one by `agent-x`.
/// - Athlete 2 selected nobody; the plan with the later week wins over a plan
///   whose outline is newer.
/// - Athlete 3 holds one plan, untouched. Athlete 0 in the second tenant
///   holds one plan too, untouched: a season is per tenant.
struct World {
    tenants: [Uuid; 2],
    athletes: [Uuid; 4],
}

impl World {
    fn new() -> Self {
        Self {
            tenants: [Uuid::new_v4(), Uuid::new_v4()],
            athletes: [
                Uuid::new_v4(),
                Uuid::new_v4(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            ],
        }
    }

    /// `(tenant, athlete, selected agent)` rows for `tenant_users`.
    fn selections(&self) -> Vec<(Uuid, Uuid, Option<&'static str>)> {
        vec![
            (self.tenants[0], self.athletes[0], Some("agent-sel")),
            (self.tenants[0], self.athletes[1], Some("agent-idle")),
            (self.tenants[0], self.athletes[2], None),
            (self.tenants[0], self.athletes[3], Some("agent-d")),
            (self.tenants[1], self.athletes[0], Some("agent-sel")),
        ]
    }
}

const PLANS: [Plan; 10] = [
    Plan {
        id: "a-selected",
        tenant: 0,
        athlete: 0,
        agent: "agent-sel",
        status: "active",
        created_at: 100,
    },
    Plan {
        id: "a-newer",
        tenant: 0,
        athlete: 0,
        agent: "agent-other",
        status: "active",
        created_at: 500,
    },
    Plan {
        id: "b-agnostic",
        tenant: 0,
        athlete: 1,
        agent: "",
        status: "active",
        created_at: 100,
    },
    Plan {
        id: "b-newer",
        tenant: 0,
        athlete: 1,
        agent: "agent-x",
        status: "active",
        created_at: 900,
    },
    Plan {
        id: "c-late-week",
        tenant: 0,
        athlete: 2,
        agent: "agent-y",
        status: "active",
        created_at: 100,
    },
    Plan {
        id: "c-new-outline",
        tenant: 0,
        athlete: 2,
        agent: "agent-z",
        status: "active",
        created_at: 500,
    },
    Plan {
        id: "c-superseded",
        tenant: 0,
        athlete: 2,
        agent: "agent-y",
        status: "superseded",
        created_at: 50,
    },
    Plan {
        id: "d-only",
        tenant: 0,
        athlete: 3,
        agent: "agent-d",
        status: "active",
        created_at: 100,
    },
    Plan {
        id: "a-other-tenant",
        tenant: 1,
        athlete: 0,
        agent: "agent-other",
        status: "active",
        created_at: 700,
    },
    Plan {
        id: "a-old-superseded",
        tenant: 0,
        athlete: 0,
        agent: "agent-sel",
        status: "superseded",
        created_at: 10,
    },
];

const WEEKS: [Week; 5] = [
    Week {
        id: "w-a-newer",
        plan: "a-newer",
        start: "2026-10-05",
        status: "active",
        created_at: 510,
    },
    Week {
        id: "w-b-agnostic",
        plan: "b-agnostic",
        start: "2026-10-05",
        status: "active",
        created_at: 110,
    },
    Week {
        id: "w-c-late",
        plan: "c-late-week",
        start: "2026-10-12",
        status: "active",
        created_at: 1000,
    },
    Week {
        id: "w-c-superseded",
        plan: "c-superseded",
        start: "2026-09-28",
        status: "superseded",
        created_at: 60,
    },
    Week {
        id: "w-orphan",
        plan: "no-such-plan",
        start: "2026-09-28",
        status: "active",
        created_at: 60,
    },
];

/// Every outline's status and author after the migration, by id.
const EXPECTED_PLANS: [(&str, &str, &str); 10] = [
    ("a-newer", "abandoned", "agent-other"),
    ("a-old-superseded", "superseded", "agent-sel"),
    ("a-other-tenant", "active", "agent-other"),
    ("a-selected", "active", "agent-sel"),
    ("b-agnostic", "active", ""),
    ("b-newer", "abandoned", "agent-x"),
    ("c-late-week", "active", "agent-y"),
    ("c-new-outline", "abandoned", "agent-z"),
    ("c-superseded", "superseded", "agent-y"),
    ("d-only", "active", "agent-d"),
];

/// Every week's status and backfilled author after the migration, by id.
const EXPECTED_WEEKS: [(&str, &str, &str, &str); 5] = [
    ("w-a-newer", "a-newer", "active", "agent-other"),
    ("w-b-agnostic", "b-agnostic", "active", ""),
    ("w-c-late", "c-late-week", "active", "agent-y"),
    ("w-c-superseded", "c-superseded", "superseded", "agent-y"),
    ("w-orphan", "no-such-plan", "active", ""),
];

fn owned(rows: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
    rows.iter()
        .map(|(a, b, c)| ((*a).to_owned(), (*b).to_owned(), (*c).to_owned()))
        .collect()
}

fn owned_weeks(rows: &[(&str, &str, &str, &str)]) -> Vec<(String, String, String, String)> {
    rows.iter()
        .map(|(a, b, c, d)| {
            (
                (*a).to_owned(),
                (*b).to_owned(),
                (*c).to_owned(),
                (*d).to_owned(),
            )
        })
        .collect()
}

#[tokio::test]
async fn each_athlete_keeps_the_season_their_home_screen_showed() {
    let database = create_test_db_url().await.unwrap();
    #[cfg(feature = "postgresql")]
    if database.url.starts_with("postgres") {
        keeps_one_season_on_postgres(&database.url).await;
        return;
    }
    keeps_one_season_on_sqlite(&database.url).await;
}

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

/// A database migrated up to, not including, the migration under test, and
/// that migration's SQL. One connection: every pooled connection to an
/// in-memory database is its own empty database. Foreign keys off: the
/// planted selections name agents and users the test does not build.
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
        if migration.version == ONE_SEASON_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the one-season migration is in migrations/");
}

async fn plant_sqlite(pool: &sqlx::SqlitePool, world: &World) {
    for (tenant, athlete, agent) in world.selections() {
        sqlx::query(
            "INSERT INTO tenant_users (id, tenant_id, user_id, role, invited_at, selected_agent_id) \
             VALUES ($1, $2, $3, 'member', '2026-01-01T00:00:00Z', $4)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(tenant.to_string())
        .bind(athlete.to_string())
        .bind(agent)
        .execute(pool)
        .await
        .unwrap();
    }
    for plan in &PLANS {
        sqlx::query(PLAN_SQL)
            .bind(plan.id)
            .bind(world.tenants[plan.tenant].to_string())
            .bind(world.athletes[plan.athlete].to_string())
            .bind(plan.agent)
            .bind(format!("strategy of {}", plan.id))
            .bind(plan.status)
            .bind(plan.created_at)
            .execute(pool)
            .await
            .unwrap();
    }
    for week in &WEEKS {
        let owner = PLANS.iter().find(|p| p.id == week.plan);
        let (tenant, athlete) = owner.map_or((0, 0), |p| (p.tenant, p.athlete));
        sqlx::query(WEEK_SQL)
            .bind(week.id)
            .bind(world.tenants[tenant].to_string())
            .bind(world.athletes[athlete].to_string())
            .bind(week.plan)
            .bind(week.start)
            .bind(week.status)
            .bind(week.created_at)
            .execute(pool)
            .await
            .unwrap();
    }
}

async fn keeps_one_season_on_sqlite(url: &str) {
    let (pool, sql) = sqlite_before_migration(url).await;
    let world = World::new();
    plant_sqlite(&pool, &world).await;

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();

    let plans: Vec<(String, String, String)> =
        sqlx::query("SELECT id, status, author_agent_id FROM training_plans ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap()
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2)))
            .collect();
    assert_eq!(plans, owned(&EXPECTED_PLANS));

    let weeks: Vec<(String, String, String, String)> = sqlx::query(
        "SELECT id, plan_id, status, author_agent_id FROM training_plan_weeks ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
    .collect();
    assert_eq!(weeks, owned_weeks(&EXPECTED_WEEKS));

    // An abandoned plan's own content is what it was: nothing but the
    // status and the stamp moved.
    let strategy: String =
        sqlx::query_scalar("SELECT strategy FROM training_plans WHERE id = 'a-newer'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(strategy, "strategy of a-newer");

    // The one-season index refuses a second active plan for the athlete.
    let after = PLAN_SQL.replace("agent_slug", "author_agent_id");
    let second = sqlx::query(&after)
        .bind("a-second-season")
        .bind(world.tenants[0].to_string())
        .bind(world.athletes[0].to_string())
        .bind("agent-other")
        .bind("a second season")
        .bind("active")
        .bind(2000_i64)
        .execute(&pool)
        .await;
    let Err(refused) = second else {
        panic!("a second active plan for the athlete must be refused");
    };
    assert!(
        refused
            .as_database_error()
            .is_some_and(DatabaseError::is_unique_violation),
        "{refused}"
    );
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
        if migration.version == ONE_SEASON_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the one-season migration is in migrations_pg/");
}

/// The selections carry real foreign keys here: users, tenants and the agents
/// they select exist before `tenant_users` can name them.
#[cfg(feature = "postgresql")]
async fn plant_postgres(pool: &sqlx::PgPool, world: &World) {
    for athlete in world.athletes {
        sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, 'x')")
            .bind(athlete)
            .bind(format!("{athlete}@season.test"))
            .execute(pool)
            .await
            .unwrap();
    }
    for tenant in world.tenants {
        sqlx::query("INSERT INTO tenants (id, name, slug) VALUES ($1, 'Season', $2)")
            .bind(tenant)
            .bind(format!("season-{tenant}"))
            .execute(pool)
            .await
            .unwrap();
    }
    let mut seeded: Vec<(Uuid, &str)> = Vec::new();
    for (tenant, athlete, agent) in world.selections() {
        if let Some(agent) = agent {
            if !seeded.iter().any(|(t, a)| *t == tenant && *a == agent) {
                sqlx::query(
                    "INSERT INTO agents (id, user_id, tenant_id, title, system_prompt, created_at, updated_at) \
                     VALUES ($1, $2, $3, $1, 'prompt', now(), now())",
                )
                .bind(format!("{agent}-{tenant}"))
                .bind(athlete)
                .bind(tenant)
                .execute(pool)
                .await
                .unwrap();
                seeded.push((tenant, agent));
            }
        }
        sqlx::query(
            "INSERT INTO tenant_users (tenant_id, user_id, invited_at, selected_agent_id) \
             VALUES ($1, $2, now(), $3)",
        )
        .bind(tenant)
        .bind(athlete)
        .bind(agent.map(|agent| format!("{agent}-{tenant}")))
        .execute(pool)
        .await
        .unwrap();
    }
    // Agent ids are per tenant here, so each plan names its agent the way a
    // chat turn stored it: the id the selection points at.
    for plan in &PLANS {
        let tenant = world.tenants[plan.tenant];
        let agent = if plan.agent.is_empty() {
            String::new()
        } else {
            format!("{}-{tenant}", plan.agent)
        };
        sqlx::query(PLAN_SQL)
            .bind(plan.id)
            .bind(tenant.to_string())
            .bind(world.athletes[plan.athlete].to_string())
            .bind(agent)
            .bind(format!("strategy of {}", plan.id))
            .bind(plan.status)
            .bind(plan.created_at)
            .execute(pool)
            .await
            .unwrap();
    }
    for week in &WEEKS {
        let owner = PLANS.iter().find(|p| p.id == week.plan);
        let (tenant, athlete) = owner.map_or((0, 0), |p| (p.tenant, p.athlete));
        sqlx::query(WEEK_SQL)
            .bind(week.id)
            .bind(world.tenants[tenant].to_string())
            .bind(world.athletes[athlete].to_string())
            .bind(week.plan)
            .bind(week.start)
            .bind(week.status)
            .bind(week.created_at)
            .execute(pool)
            .await
            .unwrap();
    }
}

/// An author as planted on this lane, without its per-tenant suffix.
#[cfg(feature = "postgresql")]
fn unsuffix(author: &str, tenant: &str) -> String {
    author
        .strip_suffix(&format!("-{tenant}"))
        .unwrap_or(author)
        .to_owned()
}

#[cfg(feature = "postgresql")]
async fn keeps_one_season_on_postgres(url: &str) {
    let (pool, sql) = postgres_before_migration(url).await;
    let world = World::new();
    plant_postgres(&pool, &world).await;

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();

    let plans: Vec<(String, String, String)> = sqlx::query(
        "SELECT id, status, author_agent_id, tenant_id FROM training_plans ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .iter()
    .map(|r| {
        let tenant: String = r.get(3);
        (
            r.get(0),
            r.get(1),
            unsuffix(&r.get::<String, _>(2), &tenant),
        )
    })
    .collect();
    assert_eq!(plans, owned(&EXPECTED_PLANS));

    let weeks: Vec<(String, String, String, String)> = sqlx::query(
        "SELECT id, plan_id, status, author_agent_id, tenant_id FROM training_plan_weeks ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .iter()
    .map(|r| {
        let tenant: String = r.get(4);
        (r.get(0), r.get(1), r.get(2), unsuffix(&r.get::<String, _>(3), &tenant))
    })
    .collect();
    assert_eq!(weeks, owned_weeks(&EXPECTED_WEEKS));

    let strategy: String =
        sqlx::query_scalar("SELECT strategy FROM training_plans WHERE id = 'a-newer'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(strategy, "strategy of a-newer");

    let after = PLAN_SQL.replace("agent_slug", "author_agent_id");
    let second = sqlx::query(&after)
        .bind("a-second-season")
        .bind(world.tenants[0].to_string())
        .bind(world.athletes[0].to_string())
        .bind("agent-other")
        .bind("a second season")
        .bind("active")
        .bind(2000_i64)
        .execute(&pool)
        .await;
    let Err(refused) = second else {
        panic!("a second active plan for the athlete must be refused");
    };
    assert!(
        refused
            .as_database_error()
            .is_some_and(DatabaseError::is_unique_violation),
        "{refused}"
    );

    // The DDL survives a second pass on this lane — the rename is guarded
    // and the rest is IF [NOT] EXISTS — and with one season per athlete
    // already in place it sets no further plan aside.
    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    let active: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM training_plans WHERE status = 'active'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(active, 5);
}
