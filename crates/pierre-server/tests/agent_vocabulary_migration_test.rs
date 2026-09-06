// ABOUTME: The two agent-vocabulary migrations, proven on a real database rather than read from the SQL
// ABOUTME: Catalogue display text becomes agent; a generated persona's opening sentence loses its role noun
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! ADR-026 splits the two senses the word "coach" carried: the persona an
//! athlete talks to is an agent, a coach is a human. Two migrations carry that
//! into rows that already exist — the `tool_catalog` display text an operator
//! reads, and the opening sentence of every persona the old
//! `coach_generation` mandate produced.
//!
//! Both are rewrites of live data, so they are proven by replaying each lane's
//! migrations up to the cutover, planting the shape the mandate produced, and
//! reading every row back. The `SQLite` lane rewrites with `instr`/`substr`
//! because that build links no `regexp` function and the `PostgreSQL` lane
//! with `regexp_replace`; the two must agree row for row, which is why both
//! lanes assert the same table. Each migration is then applied a second time:
//! a rewrite that is not idempotent corrupts on a re-run.

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
#[cfg(feature = "postgresql")]
use uuid::Uuid;

/// The display-text migration; everything before it seeds the catalogue.
const DISPLAY_TEXT_MIGRATION: i64 = 20_260_906_000_001;

/// The system-prompt migration; the rows it rewrites are planted before it.
const SYSTEM_PROMPT_MIGRATION: i64 = 20_260_906_000_002;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
#[cfg(feature = "postgresql")]
static PG_MIGRATOR: Migrator = sqlx::migrate!("../../migrations_pg");

/// (`tool_name`, `category`, `display_name`, `description`) the operator reads
/// after the migration. `tool_name` is the MCP wire name every override row
/// points at and `category` is one of the values the CHECK constraint admits —
/// both are asserted here because the migration must leave them alone.
const CATALOGUE: [(&str, &str, &str, &str); 8] = [
    (
        "list_coaches",
        "coaches",
        "List Agents",
        "List available AI agents for personalized training guidance",
    ),
    (
        "create_coach",
        "coaches",
        "Create Agent",
        "Create a custom AI agent with personalized training guidance",
    ),
    (
        "admin_list_system_coaches",
        "admin",
        "List System Agents",
        "List all system agents in the tenant (admin only)",
    ),
    (
        "admin_list_coach_assignments",
        "admin",
        "List Agent Assignments",
        "List all assignments for a system agent (admin only)",
    ),
    (
        "coach_note_add",
        "coaches",
        "Add Agent Note",
        "Persist a private agent note about the user",
    ),
    (
        "recall_user_memory",
        "coaches",
        "Recall User Memory",
        "Retrieve stored facts the agent has remembered about the user",
    ),
    (
        "browse_coach_store",
        "coaches",
        "Browse Agent Store",
        "Browse the catalogue of published agents anyone can install",
    ),
    (
        "install_coach_from_store",
        "coaches",
        "Install Agent from Store",
        "Install a published Agent Store agent into the athlete's own library",
    ),
];

/// The one catalogue row that keeps the word: a playbook is the coaching
/// *activity*, not the persona, so it reads the same in every locale.
const PLAYBOOKS: (&str, &str, &str) = (
    "list_coaching_playbooks",
    "List Coaching Playbooks",
    "List the coaching playbooks learned for this athlete",
);

/// (id, planted `system_prompt`, `system_prompt` after the migration).
///
/// The first three are what the old mandate produced. The rest are the rows
/// the rewrite must leave alone: one already rewritten, one about the
/// athlete's *human* coach, one where the phrase is not in the opening
/// sentence, one that never named a role, and one whose opening sentence
/// already carries "specialist in".
const PROMPTS: [(&str, &str, &str); 8] = [
    (
        "marathon",
        "You are a marathon coach specializing in negative-split pacing. Build weekly blocks.",
        "You are a marathon specialist in negative-split pacing. Build weekly blocks.",
    ),
    (
        "strength",
        "You are a strength and conditioning coach specializing in in-season maintenance.\n\nCite the numbers you used.",
        "You are a strength and conditioning specialist in in-season maintenance.\n\nCite the numbers you used.",
    ),
    (
        "twice",
        "You are a trail coach specializing in vert. A road coach specializing in speed is different.",
        "You are a trail specialist in vert. A road coach specializing in speed is different.",
    ),
    (
        "already",
        "You are a marathon specialist in negative-split pacing. Build weekly blocks.",
        "You are a marathon specialist in negative-split pacing. Build weekly blocks.",
    ),
    (
        "human",
        "You are Dravr. The athlete's human coach reviews these plans weekly.",
        "You are Dravr. The athlete's human coach reviews these plans weekly.",
    ),
    (
        "later-sentence",
        "You are Dravr. Answer like a triathlon coach specializing in brick workouts.",
        "You are Dravr. Answer like a triathlon coach specializing in brick workouts.",
    ),
    (
        "neutral",
        "You are Dravr, an expert in strength training. Keep replies short.",
        "You are Dravr, an expert in strength training. Keep replies short.",
    ),
    (
        "guarded",
        "You are a nutrition specialist in fuelling and a coach specializing in race day.",
        "You are a nutrition specialist in fuelling and a coach specializing in race day.",
    ),
];

#[tokio::test]
async fn the_catalogue_display_text_speaks_the_agent_vocabulary() {
    let database = create_test_db_url().await.unwrap();
    #[cfg(feature = "postgresql")]
    if database.url.starts_with("postgres") {
        catalogue_on_postgres(&database.url).await;
        return;
    }
    catalogue_on_sqlite(&database.url).await;
}

#[tokio::test]
async fn a_generated_personas_opening_sentence_loses_its_role_noun() {
    let database = create_test_db_url().await.unwrap();
    #[cfg(feature = "postgresql")]
    if database.url.starts_with("postgres") {
        prompts_on_postgres(&database.url).await;
        return;
    }
    prompts_on_sqlite(&database.url).await;
}

/// One connection: every pooled connection to an in-memory database is its own
/// empty database. Foreign keys off: the planted personas name no real user.
async fn sqlite_pool(url: &str) -> sqlx::SqlitePool {
    let options = SqliteConnectOptions::from_str(url)
        .unwrap()
        .foreign_keys(false);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap()
}

async fn catalogue_on_sqlite(url: &str) {
    let pool = sqlite_pool(url).await;
    let mut applied = None;
    for migration in MIGRATOR.iter() {
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
        if migration.version == DISPLAY_TEXT_MIGRATION {
            applied = Some(migration.sql.to_string());
            break;
        }
    }
    let sql = applied.expect("the display-text migration is in migrations/");

    assert_catalogue(&pool).await;
    // A second run recomputes the same constants: the statements key on
    // `tool_name`, which the rewrite never touches.
    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    assert_catalogue(&pool).await;
}

async fn prompts_on_sqlite(url: &str) {
    let pool = sqlite_pool(url).await;
    let mut applied = None;
    for migration in MIGRATOR.iter() {
        if migration.version == SYSTEM_PROMPT_MIGRATION {
            for (id, planted, _) in PROMPTS {
                sqlx::query(
                    "INSERT INTO coaches (id, user_id, tenant_id, title, system_prompt, created_at, updated_at)
                     VALUES ($1, 'u', 't', $2, $3, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                )
                .bind(id)
                .bind(id)
                .bind(planted)
                .execute(&pool)
                .await
                .unwrap();
            }
            sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
            applied = Some(migration.sql.to_string());
            break;
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    let sql = applied.expect("the system-prompt migration is in migrations/");

    assert_prompts(&pool).await;
    // Once rewritten the opening sentence carries "specialist in", which is one
    // of the guards, so a second run matches nothing.
    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    assert_prompts(&pool).await;
}

async fn assert_catalogue(pool: &sqlx::SqlitePool) {
    for (tool_name, category, display_name, description) in CATALOGUE {
        let row = sqlx::query(
            "SELECT display_name, description, category FROM tool_catalog WHERE tool_name = $1",
        )
        .bind(tool_name)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(row.get::<String, _>("display_name"), display_name);
        assert_eq!(row.get::<String, _>("description"), description);
        assert_eq!(row.get::<String, _>("category"), category);
    }

    let (tool_name, display_name, description) = PLAYBOOKS;
    let row =
        sqlx::query("SELECT display_name, description FROM tool_catalog WHERE tool_name = $1")
            .bind(tool_name)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(row.get::<String, _>("display_name"), display_name);
    assert_eq!(row.get::<String, _>("description"), description);

    // Nothing else in the catalogue still names the persona with the old word.
    let rows = sqlx::query("SELECT tool_name, display_name, description FROM tool_catalog")
        .fetch_all(pool)
        .await
        .unwrap();
    assert!(
        rows.len() > 100,
        "the whole catalogue is seeded: {}",
        rows.len()
    );
    for row in &rows {
        let name = row.get::<String, _>("tool_name");
        if name == PLAYBOOKS.0 {
            continue;
        }
        let text = format!(
            "{} {}",
            row.get::<String, _>("display_name"),
            row.get::<String, _>("description")
        );
        assert!(
            !text.to_lowercase().contains("coach"),
            "`{name}` still spells the persona coach: {text}"
        );
    }
}

async fn assert_prompts(pool: &sqlx::SqlitePool) {
    for (id, _, expected) in PROMPTS {
        let row = sqlx::query("SELECT system_prompt FROM coaches WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("system_prompt"), expected, "{id}");
    }
}

#[cfg(feature = "postgresql")]
async fn postgres_pool(url: &str) -> sqlx::PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .unwrap();
    // The factory hands out a clone of the migrated template; the rewrite only
    // happens as the migration runs, so start again from an empty schema.
    sqlx::raw_sql("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
        .execute(&pool)
        .await
        .unwrap();
    pool
}

#[cfg(feature = "postgresql")]
async fn catalogue_on_postgres(url: &str) {
    let pool = postgres_pool(url).await;
    let mut applied = None;
    for migration in PG_MIGRATOR.iter() {
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
        if migration.version == DISPLAY_TEXT_MIGRATION {
            applied = Some(migration.sql.to_string());
            break;
        }
    }
    let sql = applied.expect("the display-text migration is in migrations_pg/");

    assert_catalogue_pg(&pool).await;
    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    assert_catalogue_pg(&pool).await;
}

#[cfg(feature = "postgresql")]
async fn prompts_on_postgres(url: &str) {
    let pool = postgres_pool(url).await;
    let planted_at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let tenant = Uuid::new_v4();
    let user = Uuid::new_v4();
    let mut applied = None;
    for migration in PG_MIGRATOR.iter() {
        if migration.version == SYSTEM_PROMPT_MIGRATION {
            // `coaches` carries real foreign keys here, so the owner exists.
            sqlx::query("INSERT INTO tenants (id, name, slug) VALUES ($1, $2, $3)")
                .bind(tenant)
                .bind("Agent Vocabulary")
                .bind(format!("agent-vocabulary-{tenant}"))
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
                .bind(user)
                .bind(format!("agent-vocabulary-{user}@test.invalid"))
                .bind("x")
                .execute(&pool)
                .await
                .unwrap();
            for (id, planted, _) in PROMPTS {
                sqlx::query(
                    "INSERT INTO coaches (id, user_id, tenant_id, title, system_prompt, created_at, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $6)",
                )
                .bind(id)
                .bind(user)
                .bind(tenant)
                .bind(id)
                .bind(planted)
                .bind(planted_at)
                .execute(&pool)
                .await
                .unwrap();
            }
            sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
            applied = Some(migration.sql.to_string());
            break;
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    let sql = applied.expect("the system-prompt migration is in migrations_pg/");

    assert_prompts_pg(&pool).await;
    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    assert_prompts_pg(&pool).await;
}

#[cfg(feature = "postgresql")]
async fn assert_catalogue_pg(pool: &sqlx::PgPool) {
    for (tool_name, category, display_name, description) in CATALOGUE {
        let row = sqlx::query(
            "SELECT display_name, description, category FROM tool_catalog WHERE tool_name = $1",
        )
        .bind(tool_name)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(row.get::<String, _>("display_name"), display_name);
        assert_eq!(row.get::<String, _>("description"), description);
        assert_eq!(row.get::<String, _>("category"), category);
    }

    let (tool_name, display_name, description) = PLAYBOOKS;
    let row =
        sqlx::query("SELECT display_name, description FROM tool_catalog WHERE tool_name = $1")
            .bind(tool_name)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(row.get::<String, _>("display_name"), display_name);
    assert_eq!(row.get::<String, _>("description"), description);

    let rows = sqlx::query("SELECT tool_name, display_name, description FROM tool_catalog")
        .fetch_all(pool)
        .await
        .unwrap();
    assert!(
        rows.len() > 100,
        "the whole catalogue is seeded: {}",
        rows.len()
    );
    for row in &rows {
        let name = row.get::<String, _>("tool_name");
        if name == PLAYBOOKS.0 {
            continue;
        }
        let text = format!(
            "{} {}",
            row.get::<String, _>("display_name"),
            row.get::<String, _>("description")
        );
        assert!(
            !text.to_lowercase().contains("coach"),
            "`{name}` still spells the persona coach: {text}"
        );
    }
}

#[cfg(feature = "postgresql")]
async fn assert_prompts_pg(pool: &sqlx::PgPool) {
    for (id, _, expected) in PROMPTS {
        let row = sqlx::query("SELECT system_prompt FROM coaches WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("system_prompt"), expected, "{id}");
    }
}
