// ABOUTME: The 26 agent tool names move in the catalogue while every tenant and user override follows
// ABOUTME: Foreign keys are ON, so the order the migration writes in is what the assertions actually test
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! ADR-026 splits the two senses the word "coach" carried, and 20260907000002
//! carries that into the key itself: the `tool_name` of the 26 catalogue rows
//! for tools that operate on the AI persona.
//!
//! The rows that matter are not the catalogue's. `tenant_tool_overrides` and
//! `user_tool_overrides` reference `tool_catalog(tool_name)`
//! `ON DELETE CASCADE` with no `ON UPDATE`, so a catalogue row deleted ahead of
//! its children takes every operator's decision with it, and
//! `guardian::tenant_tool_enabled` reads the resulting absence as "no override
//! applies" — a tool someone turned off comes back on. A test that read only
//! the catalogue would pass while that happened, so this one plants a tenant
//! override and a user override for all 26 tools and reads them back by
//! `is_enabled` and `reason`: the same rows, under the new name.
//!
//! Foreign keys are enabled on the `SQLite` lane. Without them the ordering
//! the migration is built around is unenforced and the lane proves nothing
//! about `PostgreSQL`, where it is enforced natively.
//!
//! Each lane then applies the migration a second time. The old names are gone
//! by then, so a second run must find nothing to do rather than duplicate or
//! fail.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::str::FromStr;

use pierre_database::database::test_utils::create_test_db_url;
use sqlx::migrate::Migrator;
#[cfg(feature = "postgresql")]
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;
#[cfg(feature = "postgresql")]
use uuid::Uuid;

/// The tool-name migration; everything before it seeds the catalogue and the
/// tables the overrides live in.
const TOOL_NAME_MIGRATION: i64 = 20_260_907_000_002;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
#[cfg(feature = "postgresql")]
static PG_MIGRATOR: Migrator = sqlx::migrate!("../../migrations_pg");

/// Every `(old, new)` pair the migration carries across, in the order it writes
/// them. The old name is what the catalogue and both override tables hold
/// before the migration; the new one is the MCP wire name afterwards.
const RENAMES: [(&str, &str); 26] = [
    ("list_coaches", "list_agents"),
    ("create_coach", "create_agent"),
    ("get_coach", "get_agent"),
    ("update_coach", "update_agent"),
    ("delete_coach", "delete_agent"),
    ("toggle_coach_favorite", "toggle_agent_favorite"),
    ("search_coaches", "search_agents"),
    ("activate_coach", "activate_agent"),
    ("deactivate_coach", "deactivate_agent"),
    ("get_active_coach", "get_active_agent"),
    ("hide_coach", "hide_agent"),
    ("show_coach", "show_agent"),
    ("list_hidden_coaches", "list_hidden_agents"),
    ("admin_list_system_coaches", "admin_list_system_agents"),
    ("admin_create_system_coach", "admin_create_system_agent"),
    ("admin_get_system_coach", "admin_get_system_agent"),
    ("admin_update_system_coach", "admin_update_system_agent"),
    ("admin_delete_system_coach", "admin_delete_system_agent"),
    ("admin_assign_coach", "admin_assign_agent"),
    ("admin_unassign_coach", "admin_unassign_agent"),
    (
        "admin_list_coach_assignments",
        "admin_list_agent_assignments",
    ),
    ("browse_coach_store", "browse_agent_store"),
    ("search_coach_store", "search_agent_store"),
    ("install_coach_from_store", "install_agent_from_store"),
    ("coach_note_add", "agent_note_add"),
    ("coach_followup_schedule", "agent_followup_schedule"),
];

/// The catalogue row that keeps its name: a playbook is a learned pattern about
/// the activity of coaching, not the persona. It is planted with overrides of
/// its own so a migration that swept the table by pattern would fail here.
const KEEPS_ITS_NAME: &str = "list_coaching_playbooks";

/// What an override row says, so the assertions read the row that was planted
/// rather than one the migration could have recreated with defaults.
fn reason_for(tool_name: &str) -> String {
    format!("operator turned off {tool_name}")
}

/// The catalogue columns that must survive the move unchanged. `id` is the
/// primary key and takes the next free value, so it is deliberately absent.
#[derive(Debug, PartialEq, Eq)]
struct Catalogued {
    display_name: String,
    description: String,
    category: String,
    min_plan: String,
    enabled_by_default: String,
    created_at: String,
}

#[tokio::test]
async fn the_agent_tool_names_move_and_every_override_moves_with_them() {
    let database = create_test_db_url().await.unwrap();
    #[cfg(feature = "postgresql")]
    if database.url.starts_with("postgres") {
        rename_on_postgres(&database.url).await;
        return;
    }
    rename_on_sqlite(&database.url).await;
}

/// One connection: every pooled connection to an in-memory database is its own
/// empty database. Foreign keys on: the cascade this migration is ordered
/// around only exists when they are enforced.
async fn sqlite_pool(url: &str) -> sqlx::SqlitePool {
    let options = SqliteConnectOptions::from_str(url)
        .unwrap()
        .foreign_keys(true);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap()
}

async fn rename_on_sqlite(url: &str) {
    let pool = sqlite_pool(url).await;
    let mut applied = None;
    let mut before = Vec::new();
    let mut catalogue_size = 0_i64;
    for migration in MIGRATOR.iter() {
        if migration.version == TOOL_NAME_MIGRATION {
            plant_sqlite(&pool).await;
            before = catalogue_sqlite(&pool).await;
            catalogue_size = count_sqlite(&pool, "SELECT COUNT(*) FROM tool_catalog").await;
            sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
            applied = Some(migration.sql.to_string());
            break;
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    let sql = applied.expect("the tool-name migration is in migrations/");

    assert_renamed_sqlite(&pool, &before, catalogue_size).await;
    // The old names are gone, so the INSERT selects nothing and the two
    // UPDATEs and the DELETE match nothing.
    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    assert_renamed_sqlite(&pool, &before, catalogue_size).await;
}

/// A tenant and a user, then one override of each kind for all 26 tools plus
/// the one that keeps its name. Foreign keys are on, so the owners are real.
async fn plant_sqlite(pool: &sqlx::SqlitePool) {
    sqlx::query(
        "INSERT INTO tenants (id, name, slug, created_at, updated_at)
         VALUES ('t-rename', 'Agent Tool Names', 'agent-tool-names', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, created_at, last_active)
         VALUES ('u-rename', 'agent-tool-names@test.invalid', 'x', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(pool)
    .await
    .unwrap();

    for tool_name in RENAMES.iter().map(|(old, _)| *old).chain([KEEPS_ITS_NAME]) {
        sqlx::query(
            "INSERT INTO tenant_tool_overrides (id, tenant_id, tool_name, is_enabled, reason)
             VALUES ($1, 't-rename', $2, 0, $3)",
        )
        .bind(format!("o-{tool_name}"))
        .bind(tool_name)
        .bind(reason_for(tool_name))
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO user_tool_overrides (user_id, tool_name, is_enabled, reason)
             VALUES ('u-rename', $1, 0, $2)",
        )
        .bind(tool_name)
        .bind(reason_for(tool_name))
        .execute(pool)
        .await
        .unwrap();
    }
}

/// The catalogue columns of every old name, read before the migration runs.
async fn catalogue_sqlite(pool: &sqlx::SqlitePool) -> Vec<Catalogued> {
    let mut rows = Vec::new();
    for (old, _) in RENAMES {
        let row = sqlx::query(
            "SELECT display_name, description, category, min_plan, is_enabled_by_default, created_at
             FROM tool_catalog WHERE tool_name = $1",
        )
        .bind(old)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("`{old}` is catalogued before the migration: {e}"));
        rows.push(Catalogued {
            display_name: row.get("display_name"),
            description: row.get("description"),
            category: row.get("category"),
            min_plan: row.get("min_plan"),
            enabled_by_default: row.get::<i64, _>("is_enabled_by_default").to_string(),
            created_at: row.get("created_at"),
        });
    }
    rows
}

async fn count_sqlite(pool: &sqlx::SqlitePool, sql: &str) -> i64 {
    sqlx::query(sql).fetch_one(pool).await.unwrap().get(0)
}

async fn assert_renamed_sqlite(
    pool: &sqlx::SqlitePool,
    before: &[Catalogued],
    catalogue_size: i64,
) {
    for ((old, new), was) in RENAMES.iter().zip(before) {
        let gone: i64 = sqlx::query("SELECT COUNT(*) FROM tool_catalog WHERE tool_name = $1")
            .bind(old)
            .fetch_one(pool)
            .await
            .unwrap()
            .get(0);
        assert_eq!(gone, 0, "`{old}` is still catalogued");

        let row = sqlx::query(
            "SELECT display_name, description, category, min_plan, is_enabled_by_default, created_at
             FROM tool_catalog WHERE tool_name = $1",
        )
        .bind(new)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("`{new}` is catalogued after the migration: {e}"));
        let now = Catalogued {
            display_name: row.get("display_name"),
            description: row.get("description"),
            category: row.get("category"),
            min_plan: row.get("min_plan"),
            enabled_by_default: row.get::<i64, _>("is_enabled_by_default").to_string(),
            created_at: row.get("created_at"),
        };
        assert_eq!(&now, was, "`{new}` did not carry `{old}`'s columns across");

        // The point of the migration: the operator's decision survived the
        // move, as the same row, and now names the new tool.
        let tenant = sqlx::query(
            "SELECT is_enabled, reason FROM tenant_tool_overrides
             WHERE tenant_id = 't-rename' AND tool_name = $1",
        )
        .bind(new)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("the tenant override for `{old}` survived as `{new}`: {e}"));
        assert_eq!(tenant.get::<i64, _>("is_enabled"), 0, "{new}");
        assert_eq!(tenant.get::<String, _>("reason"), reason_for(old), "{new}");

        let user = sqlx::query(
            "SELECT is_enabled, reason FROM user_tool_overrides
             WHERE user_id = 'u-rename' AND tool_name = $1",
        )
        .bind(new)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("the user override for `{old}` survived as `{new}`: {e}"));
        assert_eq!(user.get::<i64, _>("is_enabled"), 0, "{new}");
        assert_eq!(user.get::<String, _>("reason"), reason_for(old), "{new}");
    }

    // Nothing was orphaned or duplicated: one catalogue row per name, and the
    // overrides planted for all 27 tools are all still there.
    assert_eq!(
        count_sqlite(pool, "SELECT COUNT(*) FROM tool_catalog").await,
        catalogue_size,
        "the catalogue changed size"
    );
    assert_eq!(
        count_sqlite(pool, "SELECT COUNT(*) FROM tenant_tool_overrides").await,
        27,
        "a tenant override was cascaded away"
    );
    assert_eq!(
        count_sqlite(pool, "SELECT COUNT(*) FROM user_tool_overrides").await,
        27,
        "a user override was cascaded away"
    );

    // The playbook tool and its overrides are untouched.
    let kept: i64 = sqlx::query("SELECT COUNT(*) FROM tool_catalog WHERE tool_name = $1")
        .bind(KEEPS_ITS_NAME)
        .fetch_one(pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(kept, 1, "`{KEEPS_ITS_NAME}` keeps its name");
    let kept_overrides: i64 =
        sqlx::query("SELECT COUNT(*) FROM tenant_tool_overrides WHERE tool_name = $1")
            .bind(KEEPS_ITS_NAME)
            .fetch_one(pool)
            .await
            .unwrap()
            .get(0);
    assert_eq!(kept_overrides, 1, "`{KEEPS_ITS_NAME}` keeps its override");
}

#[cfg(feature = "postgresql")]
async fn postgres_pool(url: &str) -> sqlx::PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .unwrap();
    // The factory hands out a clone of the migrated template; the rename only
    // happens as the migration runs, so start again from an empty schema.
    sqlx::raw_sql("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
        .execute(&pool)
        .await
        .unwrap();
    pool
}

#[cfg(feature = "postgresql")]
async fn rename_on_postgres(url: &str) {
    let pool = postgres_pool(url).await;
    let tenant = Uuid::new_v4();
    let user = Uuid::new_v4();
    let mut applied = None;
    let mut before = Vec::new();
    let mut catalogue_size = 0_i64;
    for migration in PG_MIGRATOR.iter() {
        if migration.version == TOOL_NAME_MIGRATION {
            plant_postgres(&pool, tenant, user).await;
            before = catalogue_postgres(&pool).await;
            catalogue_size = count_postgres(&pool, "SELECT COUNT(*) FROM tool_catalog").await;
            sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
            applied = Some(migration.sql.to_string());
            break;
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    let sql = applied.expect("the tool-name migration is in migrations_pg/");

    assert_renamed_postgres(&pool, &before, catalogue_size, tenant, user).await;
    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    assert_renamed_postgres(&pool, &before, catalogue_size, tenant, user).await;
}

#[cfg(feature = "postgresql")]
async fn plant_postgres(pool: &sqlx::PgPool, tenant: Uuid, user: Uuid) {
    sqlx::query("INSERT INTO tenants (id, name, slug) VALUES ($1, $2, $3)")
        .bind(tenant)
        .bind("Agent Tool Names")
        .bind(format!("agent-tool-names-{tenant}"))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
        .bind(user)
        .bind(format!("agent-tool-names-{user}@test.invalid"))
        .bind("x")
        .execute(pool)
        .await
        .unwrap();

    for tool_name in RENAMES.iter().map(|(old, _)| *old).chain([KEEPS_ITS_NAME]) {
        sqlx::query(
            "INSERT INTO tenant_tool_overrides (tenant_id, tool_name, is_enabled, reason)
             VALUES ($1, $2, FALSE, $3)",
        )
        .bind(tenant)
        .bind(tool_name)
        .bind(reason_for(tool_name))
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO user_tool_overrides (user_id, tool_name, is_enabled, reason)
             VALUES ($1, $2, FALSE, $3)",
        )
        .bind(user)
        .bind(tool_name)
        .bind(reason_for(tool_name))
        .execute(pool)
        .await
        .unwrap();
    }
}

#[cfg(feature = "postgresql")]
async fn catalogue_postgres(pool: &sqlx::PgPool) -> Vec<Catalogued> {
    let mut rows = Vec::new();
    for (old, _) in RENAMES {
        let row = sqlx::query(
            "SELECT display_name, description, category, min_plan,
                    is_enabled_by_default::text AS enabled, created_at::text AS created
             FROM tool_catalog WHERE tool_name = $1",
        )
        .bind(old)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("`{old}` is catalogued before the migration: {e}"));
        rows.push(Catalogued {
            display_name: row.get("display_name"),
            description: row.get("description"),
            category: row.get("category"),
            min_plan: row.get("min_plan"),
            enabled_by_default: row.get("enabled"),
            created_at: row.get("created"),
        });
    }
    rows
}

#[cfg(feature = "postgresql")]
async fn count_postgres(pool: &sqlx::PgPool, sql: &str) -> i64 {
    sqlx::query(sql).fetch_one(pool).await.unwrap().get(0)
}

#[cfg(feature = "postgresql")]
async fn assert_renamed_postgres(
    pool: &sqlx::PgPool,
    before: &[Catalogued],
    catalogue_size: i64,
    tenant: Uuid,
    user: Uuid,
) {
    for ((old, new), was) in RENAMES.iter().zip(before) {
        let gone: i64 = sqlx::query("SELECT COUNT(*) FROM tool_catalog WHERE tool_name = $1")
            .bind(old)
            .fetch_one(pool)
            .await
            .unwrap()
            .get(0);
        assert_eq!(gone, 0, "`{old}` is still catalogued");

        let row = sqlx::query(
            "SELECT display_name, description, category, min_plan,
                    is_enabled_by_default::text AS enabled, created_at::text AS created
             FROM tool_catalog WHERE tool_name = $1",
        )
        .bind(new)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("`{new}` is catalogued after the migration: {e}"));
        let now = Catalogued {
            display_name: row.get("display_name"),
            description: row.get("description"),
            category: row.get("category"),
            min_plan: row.get("min_plan"),
            enabled_by_default: row.get("enabled"),
            created_at: row.get("created"),
        };
        assert_eq!(&now, was, "`{new}` did not carry `{old}`'s columns across");

        let tenant_row = sqlx::query(
            "SELECT is_enabled, reason FROM tenant_tool_overrides
             WHERE tenant_id = $1 AND tool_name = $2",
        )
        .bind(tenant)
        .bind(new)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("the tenant override for `{old}` survived as `{new}`: {e}"));
        assert!(!tenant_row.get::<bool, _>("is_enabled"), "{new}");
        assert_eq!(
            tenant_row.get::<String, _>("reason"),
            reason_for(old),
            "{new}"
        );

        let user_row = sqlx::query(
            "SELECT is_enabled, reason FROM user_tool_overrides
             WHERE user_id = $1 AND tool_name = $2",
        )
        .bind(user)
        .bind(new)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("the user override for `{old}` survived as `{new}`: {e}"));
        assert!(!user_row.get::<bool, _>("is_enabled"), "{new}");
        assert_eq!(
            user_row.get::<String, _>("reason"),
            reason_for(old),
            "{new}"
        );
    }

    assert_eq!(
        count_postgres(pool, "SELECT COUNT(*) FROM tool_catalog").await,
        catalogue_size,
        "the catalogue changed size"
    );
    assert_eq!(
        count_postgres(pool, "SELECT COUNT(*) FROM tenant_tool_overrides").await,
        27,
        "a tenant override was cascaded away"
    );
    assert_eq!(
        count_postgres(pool, "SELECT COUNT(*) FROM user_tool_overrides").await,
        27,
        "a user override was cascaded away"
    );

    let kept: i64 = sqlx::query("SELECT COUNT(*) FROM tool_catalog WHERE tool_name = $1")
        .bind(KEEPS_ITS_NAME)
        .fetch_one(pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(kept, 1, "`{KEEPS_ITS_NAME}` keeps its name");
    let kept_overrides: i64 =
        sqlx::query("SELECT COUNT(*) FROM tenant_tool_overrides WHERE tool_name = $1")
            .bind(KEEPS_ITS_NAME)
            .fetch_one(pool)
            .await
            .unwrap()
            .get(0);
    assert_eq!(kept_overrides, 1, "`{KEEPS_ITS_NAME}` keeps its override");
}
