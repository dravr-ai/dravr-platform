// ABOUTME: carnet#827 — the migration that moves the coach role off coaching_persona and resets the style it forced
// ABOUTME: Runs the shipped backfill statements against a pre-split coach and athlete: the coach keeps the role, both read casual

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use pierre_core::models::{CoachingPersona, TenantId};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use uuid::Uuid;

use crate::common::{create_test_database, create_test_user_with_plan};

/// The shipped migration, per backend — read as a file because the migration
/// is the data under test, and its statements are executed, not matched. Only
/// its data statements run here: the column it adds already exists on a test
/// database built from every migration.
const SQLITE_MIGRATION_SQL: &str =
    include_str!("../../../migrations/20261007150000_user_coaches_others.sql");
#[cfg(feature = "postgresql")]
const POSTGRES_MIGRATION_SQL: &str =
    include_str!("../../../migrations_pg/20261007150000_user_coaches_others.sql");

/// The migration's `UPDATE` statements, in the order a deploy runs them.
fn backfill_statements(sql: &str) -> Vec<String> {
    let code: String = sql
        .lines()
        .map(|line| line.split_once("--").map_or(line, |(code, _)| code))
        .collect::<Vec<_>>()
        .join("\n");
    code.split(';')
        .map(|stmt| stmt.trim().to_owned())
        .filter(|stmt| stmt.starts_with("UPDATE"))
        .collect()
}

async fn run_backfill(db: &Database) {
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            let statements = backfill_statements(SQLITE_MIGRATION_SQL);
            assert_eq!(statements.len(), 2, "role backfill then style reset");
            for stmt in statements {
                sqlx::raw_sql(&stmt).execute(sqlite.pool()).await.unwrap();
            }
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => {
            let statements = backfill_statements(POSTGRES_MIGRATION_SQL);
            assert_eq!(statements.len(), 2, "role backfill then style reset");
            for stmt in statements {
                sqlx::raw_sql(&stmt).execute(postgres.pool()).await.unwrap();
            }
        }
    }
}

/// Put a user in the pre-split shape: the role answer stored as the style.
async fn set_pre_split(db: &Database, user_id: Uuid, persona: CoachingPersona) {
    let users = &db.repositories().users;
    users.set_coaching_persona(user_id, persona).await.unwrap();
    users.set_coaches_others(user_id, false).await.unwrap();
}

async fn role_and_style(db: &Database, user_id: Uuid, tenant: TenantId) -> (bool, CoachingPersona) {
    let users = &db.repositories().users;
    let user = users
        .get(user_id, tenant)
        .await
        .unwrap()
        .expect("user exists");
    (
        users.coaches_others(user_id).await.unwrap(),
        user.coaching_persona,
    )
}

#[tokio::test]
async fn a_pre_split_coach_keeps_the_role_and_gets_the_casual_voice() {
    let db = create_test_database().await.unwrap();
    let (coach_id, _, coach_tenant) =
        create_test_user_with_plan(&db, "role-split-coach@example.com", "starter")
            .await
            .unwrap();
    let (athlete_id, _, athlete_tenant) =
        create_test_user_with_plan(&db, "role-split-athlete@example.com", "starter")
            .await
            .unwrap();
    set_pre_split(&db, coach_id, CoachingPersona::Coach).await;
    set_pre_split(&db, athlete_id, CoachingPersona::PowerAthlete).await;

    run_backfill(&db).await;

    assert_eq!(
        role_and_style(&db, coach_id, coach_tenant).await,
        (true, CoachingPersona::Casual),
        "the coach answer becomes the role, and the style it forced resets to casual"
    );
    assert_eq!(
        role_and_style(&db, athlete_id, athlete_tenant).await,
        (false, CoachingPersona::PowerAthlete),
        "a style the athlete chose is untouched and grants no role"
    );

    // Idempotent: a second run changes nothing.
    run_backfill(&db).await;
    assert_eq!(
        role_and_style(&db, coach_id, coach_tenant).await,
        (true, CoachingPersona::Casual)
    );
}
