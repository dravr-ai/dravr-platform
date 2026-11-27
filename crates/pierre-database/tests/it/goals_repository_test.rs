// ABOUTME: Covers recording and reading a goal against whichever backend DATABASE_URL names
// ABOUTME: A goal's row id binds as the column's own type, so set_goal records on PostgreSQL as on SQLite
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `goals.id` is a uuid on PostgreSQL (the deployed database held it so, and a
//! migration brings every built database to it) and TEXT on `SQLite`. A goal id
//! bound as a bare string failed every `set_goal` on the deployed database with
//! `column "id" is of type uuid but expression is of type text` (2026-11-26).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_core::models::User;
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use serde_json::json;
use uuid::Uuid;

/// A distinct user per call; the table's foreign key needs the row to exist.
async fn fresh_user(repos: &RepositoryRegistry) -> Uuid {
    let user = User::new(
        format!("goals-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Goals Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

#[tokio::test]
async fn a_recorded_goal_reads_back_carrying_its_own_id() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(repos).await;

    let goal_id = repos
        .profiles
        .create_goal(
            user_id,
            json!({ "goal_type": "frequency", "target_value": 4.0, "sport_type": "run" }),
        )
        .await
        .expect("bug: a goal must record on every backend");

    assert!(
        Uuid::parse_str(&goal_id).is_ok(),
        "the row id is a uuid: {goal_id}"
    );
    let goals = repos.profiles.get_goals(user_id).await.unwrap();
    assert_eq!(goals.len(), 1);
    assert_eq!(goals[0]["goal_id"], goal_id.as_str());
    assert_eq!(goals[0]["goal_type"], "frequency");
}
