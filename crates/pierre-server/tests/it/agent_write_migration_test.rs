// ABOUTME: The agent-written records migration: the store, clinical review logging, and one live child per (parent, scope)
// ABOUTME: Replays each lane's migrations up to it, plants a parent with a step-up child, runs it, and checks the new index and table

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1544 slice 4c (plan revision 21 §21.13). 20261127010000 recreates
//! `idx_oauth_client_grants_live_child` on `(parent_grant_id, scope)`, so a
//! parent holds one live step-up child and one live agent-write child at
//! once, never two of a kind. It creates `agent_written_records`, whose rows
//! go with their user, and adds `clinical_records.review_logged_at`.
//!
//! Each lane has its own migration set, so the factory's URL decides which
//! one is replayed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::str::FromStr;

use pierre_test_support::db::create_test_db_url;
use sqlx::migrate::Migrator;
#[cfg(feature = "postgresql")]
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

/// The migration under test; everything before it builds the old shape.
const AGENT_WRITE_MIGRATION: i64 = 20_261_127_010_000;

// The shipped SQL is the data under test, replayed up to and through the
// migration, since a test database from the factory is already past it.
static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
#[cfg(feature = "postgresql")]
static PG_MIGRATOR: Migrator = sqlx::migrate!("../../migrations_pg");

const USER: &str = "6d1f0c0e-2b49-4c56-9d0a-3f1b8b0c4a11";

/// A live parent grant and its live step-up child, planted before the
/// migration.
const PLANT_GRANTS: &str = "
INSERT INTO oauth_client_grants (id, user_id, tenant_id, client_id, scope, granted_at)
    VALUES ('parent', '6d1f0c0e-2b49-4c56-9d0a-3f1b8b0c4a11', 'tenant-1', 'client-1',
            'fitness:read', CURRENT_TIMESTAMP);
INSERT INTO oauth_client_grants (id, user_id, tenant_id, client_id, scope, granted_at,
        parent_grant_id)
    VALUES ('step-up', '6d1f0c0e-2b49-4c56-9d0a-3f1b8b0c4a11', 'tenant-1', 'client-1',
            'fitness:read pdpp:preset:one_off_question', CURRENT_TIMESTAMP, 'parent');
";

/// A live agent-write child `id` of the planted parent.
const WRITE_CHILD: &str = "
INSERT INTO oauth_client_grants (id, user_id, tenant_id, client_id, scope, granted_at,
        parent_grant_id)
    VALUES ($1, '6d1f0c0e-2b49-4c56-9d0a-3f1b8b0c4a11', 'tenant-1', 'client-1',
            'pdpp:agent_write', CURRENT_TIMESTAMP, 'parent')";

/// A second live step-up child of the planted parent.
const SECOND_STEP_UP: &str = "
INSERT INTO oauth_client_grants (id, user_id, tenant_id, client_id, scope, granted_at,
        parent_grant_id)
    VALUES ('step-up-2', '6d1f0c0e-2b49-4c56-9d0a-3f1b8b0c4a11', 'tenant-1', 'client-1',
            'fitness:read pdpp:preset:one_off_question', CURRENT_TIMESTAMP, 'parent')";

/// One pending proposal of the planted user.
const PROPOSAL: &str = "
INSERT INTO agent_written_records (id, tenant_id, user_id, client_grant_id, client_id, stream,
        tier, fields, provenance, review_state, created_at)
    VALUES ('record-1', 'tenant-1', '6d1f0c0e-2b49-4c56-9d0a-3f1b8b0c4a11', 'write-1',
            'client-1', 'labs.summary', 'T2', '{}', '{}', 'pending', CURRENT_TIMESTAMP)";

#[tokio::test]
async fn a_parent_holds_one_child_of_each_kind_and_proposals_go_with_their_user() {
    let database = create_test_db_url().await.unwrap();
    #[cfg(feature = "postgresql")]
    if database.url.starts_with("postgres") {
        on_postgres(&database.url).await;
        return;
    }
    on_sqlite(&database.url).await;
}

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

async fn on_sqlite(url: &str) {
    let options = SqliteConnectOptions::from_str(url)
        .unwrap()
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let mut sql = None;
    for migration in MIGRATOR.iter() {
        if migration.version == AGENT_WRITE_MIGRATION {
            sql = Some(migration.sql.to_string());
            break;
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    let sql = sql.expect("the agent-write migration is in migrations/");
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, created_at, last_active)
         VALUES ($1, $2, 'x', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
        .bind(USER)
        .bind("writer@agent-write.test")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(PLANT_GRANTS).execute(&pool).await.unwrap();
    assert!(
        sqlx::query(WRITE_CHILD)
            .bind("write-before")
            .execute(&pool)
            .await
            .is_err(),
        "the old index holds one live child per parent, whatever its kind"
    );

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();

    sqlx::query(WRITE_CHILD)
        .bind("write-1")
        .execute(&pool)
        .await
        .expect("bug: a write child was refused beside the step-up child");
    assert!(
        sqlx::query(WRITE_CHILD)
            .bind("write-2")
            .execute(&pool)
            .await
            .is_err(),
        "bug: a parent took a second live write child"
    );
    assert!(
        sqlx::raw_sql(SECOND_STEP_UP).execute(&pool).await.is_err(),
        "bug: a parent took a second live step-up child"
    );
    sqlx::raw_sql("UPDATE clinical_records SET review_logged_at = NULL")
        .execute(&pool)
        .await
        .expect("bug: clinical_records has no review_logged_at");
    sqlx::raw_sql(PROPOSAL).execute(&pool).await.unwrap();
    assert!(
        sqlx::raw_sql(&PROPOSAL.replace("'T2'", "'T3'"))
            .execute(&pool)
            .await
            .is_err(),
        "bug: a T3 proposal was stored"
    );
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(USER)
        .execute(&pool)
        .await
        .unwrap();
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_written_records")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0, "bug: a deleted user's proposals stayed");
}

// ---------------------------------------------------------------------------
// PostgreSQL
// ---------------------------------------------------------------------------

#[cfg(feature = "postgresql")]
async fn on_postgres(url: &str) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .unwrap();
    sqlx::raw_sql("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
        .execute(&pool)
        .await
        .unwrap();
    let mut sql = None;
    for migration in PG_MIGRATOR.iter() {
        if migration.version == AGENT_WRITE_MIGRATION {
            sql = Some(migration.sql.to_string());
            break;
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    let sql = sql.expect("the agent-write migration is in migrations_pg/");
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1::uuid, $2, 'x')")
        .bind(USER)
        .bind("writer@agent-write.test")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(PLANT_GRANTS).execute(&pool).await.unwrap();
    assert!(
        sqlx::query(WRITE_CHILD)
            .bind("write-before")
            .execute(&pool)
            .await
            .is_err(),
        "the old index holds one live child per parent, whatever its kind"
    );

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();

    sqlx::query(WRITE_CHILD)
        .bind("write-1")
        .execute(&pool)
        .await
        .expect("bug: a write child was refused beside the step-up child");
    assert!(
        sqlx::query(WRITE_CHILD)
            .bind("write-2")
            .execute(&pool)
            .await
            .is_err(),
        "bug: a parent took a second live write child"
    );
    assert!(
        sqlx::raw_sql(SECOND_STEP_UP).execute(&pool).await.is_err(),
        "bug: a parent took a second live step-up child"
    );
    sqlx::raw_sql("UPDATE clinical_records SET review_logged_at = NULL")
        .execute(&pool)
        .await
        .expect("bug: clinical_records has no review_logged_at");
    sqlx::raw_sql(PROPOSAL).execute(&pool).await.unwrap();
    assert!(
        sqlx::raw_sql(&PROPOSAL.replace("'T2'", "'T3'"))
            .execute(&pool)
            .await
            .is_err(),
        "bug: a T3 proposal was stored"
    );
    sqlx::query("DELETE FROM users WHERE id = $1::uuid")
        .bind(USER)
        .execute(&pool)
        .await
        .unwrap();
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_written_records")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0, "bug: a deleted user's proposals stayed");
}
