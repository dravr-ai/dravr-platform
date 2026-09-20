// ABOUTME: Pins what the seeder repository writes against whichever backend DATABASE_URL names
// ABOUTME: Each test is a value the two backends once disagreed on, asserted through the app's own readers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The seeder repository had no direct test on either backend: `demo_data`
//! swallowed insert errors with `.is_ok()`, and the shape test read source
//! text. Every test here seeds one row through `SeederRepository` and reads
//! it back the way the application does, so a value that lands differently
//! on `SQLite` and `PostgreSQL` fails on the wrong side. `create_test_db`
//! opens whichever `DATABASE_URL` names, so the same assertions cover both.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;

use chrono::{DateTime, Duration, Timelike, Utc};
use pierre_core::models::mobility::{
    ActivityMuscleMapping, DifficultyLevel, StretchingCategory, StretchingExercise, YogaCategory,
    YogaPose, YogaPoseType,
};
use pierre_core::models::{ApiKey, ApiKeyTier, User};
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
use pierre_database::repositories::SeedTable;
use pierre_database::seed_models::{
    SeedA2AClient, SeedA2AUsage, SeedApiKey, SeedApiKeyUsage, SeedDemoUser,
};
use pierre_database::RepositoryRegistry;
use sqlx::Row;
use uuid::Uuid;

/// A distinct user per call, so one test's rows cannot satisfy another's
/// assertions; the seeded tables carry a foreign key to `users`.
async fn fresh_user(repos: &RepositoryRegistry) -> Uuid {
    let user = User::new(
        format!("seed-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Seeder Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

/// One text column of one row, read straight from the table on whichever
/// backend the test database is: for the columns no repository reader
/// exposes.
async fn text_cell(db: &Database, sql: &str, key: &str) -> String {
    match db {
        Database::SQLite(sqlite) => sqlx::query(sql)
            .bind(key)
            .fetch_one(sqlite.pool())
            .await
            .unwrap()
            .get(0),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(pg) => sqlx::query(sql)
            .bind(key)
            .fetch_one(pg.pool())
            .await
            .unwrap()
            .get(0),
    }
}

fn seed_api_key(user_id: Uuid, tier: &str, rate_limit: Option<i32>) -> SeedApiKey {
    let id = Uuid::new_v4();
    SeedApiKey {
        id,
        user_id,
        name: format!("seed-key-{id}"),
        description: "seeded for the repository test".to_owned(),
        key_hash: format!("{:064x}", id.as_u128()),
        key_prefix: format!("pk_{}", &id.simple().to_string()[..8]),
        tier: tier.to_owned(),
        rate_limit,
        expires_at: None,
        created_at: Utc::now(),
    }
}

fn seed_a2a_client(user_id: Uuid, capabilities: &str) -> SeedA2AClient {
    let id = Uuid::new_v4();
    SeedA2AClient {
        id,
        user_id,
        name: format!("seed-client-{id}"),
        description: "seeded for the repository test".to_owned(),
        public_key: format!("pk_a2a_{}", id.simple()),
        client_secret: format!("{:064x}", id.as_u128()),
        permissions: r#"["read", "write"]"#.to_owned(),
        capabilities: capabilities.to_owned(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

/// `api_key_usage.tool_name` was renamed to `endpoint` on `SQLite` in
/// 20260606000001; the `SQLite` seeder kept the old name, so every usage
/// insert failed and `demo_data` swallowed it — a `SQLite` demo database had
/// no API-key usage at all. Both usage tables must take a seeded row and
/// store the tool under `endpoint`.
#[tokio::test]
async fn usage_rows_land_under_endpoint_on_both_tables() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(&repos).await;

    let key = seed_api_key(user_id, "professional", Some(1000));
    repos.seeder.seed_insert_api_key(&key).await.unwrap();
    let keys_before = repos
        .seeder
        .seed_count_table(SeedTable::ApiKeyUsage)
        .await
        .unwrap();
    repos
        .seeder
        .seed_insert_api_key_usage(&SeedApiKeyUsage {
            id: Uuid::new_v4(),
            api_key_id: key.id,
            timestamp: Utc::now(),
            tool_name: "get_activities".to_owned(),
            status_code: 200,
            response_time_ms: 42,
        })
        .await
        .expect("an API-key usage row must insert on this backend");
    assert_eq!(
        repos
            .seeder
            .seed_count_table(SeedTable::ApiKeyUsage)
            .await
            .unwrap(),
        keys_before + 1,
        "one API-key usage row lands"
    );
    assert_eq!(
        text_cell(
            &db,
            "SELECT endpoint FROM api_key_usage WHERE api_key_id = $1",
            &key.id.to_string(),
        )
        .await,
        "get_activities",
        "the tool name is stored under endpoint"
    );

    let client = seed_a2a_client(user_id, r#"["chat"]"#);
    repos.seeder.seed_insert_a2a_client(&client).await.unwrap();
    let clients_before = repos
        .seeder
        .seed_count_table(SeedTable::A2AUsage)
        .await
        .unwrap();
    repos
        .seeder
        .seed_insert_a2a_usage(&SeedA2AUsage {
            id: Uuid::new_v4(),
            client_id: client.id,
            timestamp: Utc::now(),
            tool_name: "get_athlete".to_owned(),
            status_code: 201,
            response_time_ms: 7,
        })
        .await
        .expect("an A2A usage row must insert on this backend");
    assert_eq!(
        repos
            .seeder
            .seed_count_table(SeedTable::A2AUsage)
            .await
            .unwrap(),
        clients_before + 1,
        "one A2A usage row lands"
    );
    assert_eq!(
        text_cell(
            &db,
            "SELECT endpoint FROM a2a_usage WHERE client_id = $1",
            &client.id.to_string(),
        )
        .await,
        "get_athlete",
        "the tool name is stored under endpoint"
    );
}

fn seed_demo_user(status: &str) -> SeedDemoUser {
    let id = Uuid::new_v4();
    SeedDemoUser {
        id,
        email: format!("demo-{id}@example.com"),
        display_name: "Demo Tester".to_owned(),
        password_hash: "argon2-hash-placeholder".to_owned(),
        tier: "starter".to_owned(),
        status: status.to_owned(),
        is_admin: false,
        locale: "en".to_owned(),
        created_at: Utc::now(),
    }
}

/// `demo_data` seeds one suspended account. `SQLite` derived `is_active`
/// from the status; Postgres wrote `true` for every user, so the suspended
/// demo account was active there. The status decides on both.
#[tokio::test]
async fn demo_user_is_active_follows_its_status() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    for (status, expected) in [("active", true), ("pending", true), ("suspended", false)] {
        let user = seed_demo_user(status);
        repos.seeder.seed_insert_demo_user(&user).await.unwrap();
        let stored = repos
            .users
            .get_by_email(&user.email)
            .await
            .unwrap()
            .expect("seeded user reads back");
        assert_eq!(
            stored.is_active, expected,
            "a {status} demo user reads back is_active = {expected}"
        );
    }
}

/// An enterprise key seeded with `rate_limit: None` is an unlimited key —
/// every `None` in `demo_data` sits on an enterprise tier. `SQLite` stored
/// NULL, which its reader maps to unlimited; Postgres stored 1000, a limited
/// enterprise key, while its own `ApiKeyRepository::create` stores `i32::MAX`
/// for unlimited. The seeded key must read back exactly as a key the
/// application created as unlimited on the same backend.
#[tokio::test]
async fn enterprise_key_seeded_without_a_limit_reads_as_unlimited() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(&repos).await;

    let seeded = seed_api_key(user_id, "enterprise", None);
    repos.seeder.seed_insert_api_key(&seeded).await.unwrap();

    let created = ApiKey {
        id: Uuid::new_v4().to_string(),
        user_id,
        name: "created unlimited".to_owned(),
        key_prefix: "pk_created".to_owned(),
        key_hash: format!("{:064x}", Uuid::new_v4().as_u128()),
        description: None,
        tier: ApiKeyTier::Enterprise,
        rate_limit_requests: u32::MAX,
        rate_limit_window_seconds: 3600,
        is_active: true,
        last_used_at: None,
        expires_at: None,
        created_at: Utc::now(),
    };
    repos.api_keys.create(&created).await.unwrap();

    let keys = repos.api_keys.get_for_user(user_id).await.unwrap();
    let find = |id: &str| {
        keys.iter()
            .find(|k| k.id == id)
            .unwrap_or_else(|| panic!("key {id} reads back"))
    };
    let seeded_limit = find(&seeded.id.to_string()).rate_limit_requests;
    let created_limit = find(&created.id).rate_limit_requests;
    assert_eq!(
        seeded_limit, created_limit,
        "the seeded enterprise key must be as unlimited as one the application created"
    );
}

/// The seed's capabilities are a JSON list. `SQLite` stored that text in
/// its TEXT column; Postgres bound an empty array to its TEXT[] column and
/// dropped the list, so every demo A2A client had no capabilities there.
/// The client must read back with the seeded capabilities on both.
#[tokio::test]
async fn a2a_client_keeps_its_seeded_capabilities() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(&repos).await;

    let client = seed_a2a_client(user_id, r#"["chat", "analyze"]"#);
    repos.seeder.seed_insert_a2a_client(&client).await.unwrap();

    let stored = repos
        .a2a
        .get_client(&client.id.to_string())
        .await
        .unwrap()
        .expect("seeded A2A client reads back");
    assert_eq!(
        stored.capabilities,
        vec!["chat".to_owned(), "analyze".to_owned()],
        "the seeded capability list is what the client carries"
    );
    assert_eq!(stored.name, client.name);
    assert!(stored.is_active, "a seeded client is active");
}

/// The per-day quota of a seeded A2A client. `SQLite` wrote 3600 — the
/// API-key window-seconds constant, pasted into the wrong column — while
/// Postgres wrote 10000, the column default on both backends. No reader
/// exposes the column, so it is read straight from the table.
#[tokio::test]
async fn a2a_client_seeds_the_default_daily_quota() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(&repos).await;

    let client = seed_a2a_client(user_id, "[]");
    repos.seeder.seed_insert_a2a_client(&client).await.unwrap();

    let per_day = text_cell(
        &db,
        "SELECT CAST(rate_limit_per_day AS TEXT) FROM a2a_clients WHERE client_id = $1",
        &client.id.to_string(),
    )
    .await;
    assert_eq!(
        per_day, "10000",
        "a seeded client carries the default daily quota"
    );
}

/// A whole-second instant: Postgres keeps microseconds and `SQLite` keeps
/// what `to_rfc3339` wrote, so a value with no sub-second part round-trips
/// identically on both.
fn whole_second(offset: Duration) -> DateTime<Utc> {
    (Utc::now() + offset).with_nanosecond(0).unwrap()
}

fn stretching_exercise(id: &str, name: &str, at: DateTime<Utc>) -> StretchingExercise {
    StretchingExercise {
        id: id.to_owned(),
        name: name.to_owned(),
        description: "seeded for the repository test".to_owned(),
        category: StretchingCategory::Static,
        difficulty: DifficultyLevel::Beginner,
        primary_muscles: vec!["hamstrings".to_owned()],
        secondary_muscles: vec![],
        duration_seconds: 30,
        repetitions: None,
        sets: 1,
        recommended_for_activities: vec!["run".to_owned()],
        contraindications: vec![],
        instructions: vec!["reach".to_owned()],
        cues: vec![],
        image_url: None,
        video_url: None,
        created_at: at,
        updated_at: at,
    }
}

fn yoga_pose(id: &str, name: &str, at: DateTime<Utc>) -> YogaPose {
    YogaPose {
        id: id.to_owned(),
        english_name: name.to_owned(),
        sanskrit_name: None,
        description: "seeded for the repository test".to_owned(),
        benefits: vec!["calm".to_owned()],
        category: YogaCategory::Seated,
        difficulty: DifficultyLevel::Beginner,
        pose_type: YogaPoseType::Relaxation,
        primary_muscles: vec!["hips".to_owned()],
        secondary_muscles: vec![],
        chakras: vec![],
        hold_duration_seconds: 45,
        breath_guidance: None,
        recommended_for_activities: vec![],
        recommended_for_recovery: vec![],
        contraindications: vec![],
        instructions: vec!["sit".to_owned()],
        modifications: vec![],
        progressions: vec![],
        cues: vec![],
        warmup_poses: vec![],
        followup_poses: vec![],
        image_url: None,
        video_url: None,
        created_at: at,
        updated_at: at,
    }
}

fn activity_mapping(
    id: &str,
    activity_type: &str,
    stretch: &str,
    at: DateTime<Utc>,
) -> ActivityMuscleMapping {
    ActivityMuscleMapping {
        id: id.to_owned(),
        activity_type: activity_type.to_owned(),
        primary_muscles: HashMap::from([("quadriceps".to_owned(), 90)]),
        secondary_muscles: HashMap::new(),
        recommended_stretch_categories: vec![stretch.to_owned()],
        recommended_yoga_categories: vec![],
        created_at: at,
        updated_at: at,
    }
}

/// Re-seeding a reference row refreshes its content and `updated_at` and
/// keeps its `created_at`. `SQLite` used INSERT OR REPLACE, which deletes
/// the row and inserts the new one — `created_at` moved on every seed run —
/// while Postgres updated in place. All three mobility reference tables
/// must behave as Postgres did.
#[tokio::test]
async fn reseeding_reference_rows_keeps_created_at() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let first = whole_second(Duration::zero());
    let second = whole_second(Duration::hours(1));
    let suffix = Uuid::new_v4().simple().to_string();

    let stretch_id = format!("stretch-{suffix}");
    repos
        .seeder
        .seed_upsert_stretching_exercise(&stretching_exercise(&stretch_id, "Hamstring", first))
        .await
        .unwrap();
    repos
        .seeder
        .seed_upsert_stretching_exercise(&stretching_exercise(
            &stretch_id,
            "Hamstring (standing)",
            second,
        ))
        .await
        .unwrap();
    let stretch = repos
        .mobility
        .get_stretching_exercise(&stretch_id)
        .await
        .unwrap()
        .expect("seeded exercise reads back");
    assert_eq!(
        stretch.name, "Hamstring (standing)",
        "the re-seed refreshes content"
    );
    assert_eq!(stretch.updated_at, second, "the re-seed moves updated_at");
    assert_eq!(stretch.created_at, first, "the re-seed keeps created_at");

    let pose_id = format!("pose-{suffix}");
    repos
        .seeder
        .seed_upsert_yoga_pose(&yoga_pose(&pose_id, "Easy Pose", first))
        .await
        .unwrap();
    repos
        .seeder
        .seed_upsert_yoga_pose(&yoga_pose(&pose_id, "Easy Pose (Sukhasana)", second))
        .await
        .unwrap();
    let pose = repos
        .mobility
        .get_yoga_pose(&pose_id)
        .await
        .unwrap()
        .expect("seeded pose reads back");
    assert_eq!(pose.english_name, "Easy Pose (Sukhasana)");
    assert_eq!(pose.updated_at, second);
    assert_eq!(pose.created_at, first, "the re-seed keeps created_at");

    let activity_type = format!("activity-{suffix}");
    let mapping_id = format!("mapping-{suffix}");
    repos
        .seeder
        .seed_upsert_activity_mapping(&activity_mapping(
            &mapping_id,
            &activity_type,
            "static",
            first,
        ))
        .await
        .unwrap();
    repos
        .seeder
        .seed_upsert_activity_mapping(&activity_mapping(
            &mapping_id,
            &activity_type,
            "dynamic",
            second,
        ))
        .await
        .unwrap();
    let mapping = repos
        .mobility
        .get_activity_muscle_mapping(&activity_type)
        .await
        .unwrap()
        .expect("seeded mapping reads back");
    assert_eq!(
        mapping.recommended_stretch_categories,
        vec!["dynamic".to_owned()]
    );
    assert_eq!(mapping.updated_at, second);
    assert_eq!(mapping.created_at, first, "the re-seed keeps created_at");
}
