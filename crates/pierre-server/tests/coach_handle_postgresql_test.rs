// ABOUTME: PostgreSQL-lane tests for the coach catalogue handle
// ABOUTME: Exercises the UUID/BOOLEAN bind paths of approval, install, and lookup-by-handle

//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `PostgreSQL` coach-handle tests.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(feature = "postgresql")]

use chrono::Utc;
use pierre_core::models::coaches::{
    CoachCategory, CoachHandle, CoachVisibility, CreateSystemCoachRequest,
};
use pierre_core::models::{CoachingPersona, TenantId, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::Database;
use uuid::Uuid;

mod common;

async fn seed_pg_user(db: &Database) -> Uuid {
    let user_id = Uuid::new_v4();
    let user = User {
        id: user_id,
        email: format!("handle-{user_id}@test.local"),
        display_name: Some("Coach Handle Test".to_owned()),
        password_hash: "hash_not_verified".to_owned(),
        tier: UserTier::Starter,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: Some(Utc::now()),
        created_at: Utc::now(),
        last_active: Utc::now(),
        strava_token: None,
        fitbit_token: None,
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
        theme: None,
    };
    db.repositories().users.create(&user).await.unwrap();
    user_id
}

async fn publish_coach(db: &Database, author_id: Uuid, tenant_id: TenantId, title: &str) -> Uuid {
    let repos = db.repositories();
    let coach = repos
        .coaches
        .create_system_coach(
            author_id,
            tenant_id,
            &CreateSystemCoachRequest {
                title: title.to_owned(),
                description: Some(format!("Description for {title}")),
                system_prompt: format!("You are the {title}."),
                category: CoachCategory::Training,
                tags: vec!["test".to_owned()],
                visibility: CoachVisibility::Tenant,
                sample_prompts: vec![],
            },
        )
        .await
        .unwrap();
    assert_eq!(coach.handle, None);

    let id = coach.id.to_string();
    repos
        .store_listings
        .submit_for_review(&id, author_id, tenant_id)
        .await
        .unwrap();
    let approved = repos
        .store_listings
        .approve_coach(&id, tenant_id, Some(author_id))
        .await
        .unwrap();
    assert!(
        approved.coach.handle.is_some(),
        "approval must assign a handle on the PG lane"
    );
    coach.id
}

#[tokio::test]
async fn test_pg_handle_is_assigned_copied_and_resolved_for_installers_only() {
    let isolated = match common::IsolatedPostgresDb::new().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not available: {e}");
            return;
        }
    };
    let db = isolated.get_database().await.unwrap();
    let repos = db.repositories();

    let author_id = seed_pg_user(&db).await;
    let author_tenant = TenantId::generate();
    let athlete_id = seed_pg_user(&db).await;
    let athlete_tenant = TenantId::generate();
    let bystander_id = seed_pg_user(&db).await;

    // Two coaches with the same title get distinct catalogue handles.
    let first = publish_coach(&db, author_id, author_tenant, "Tempo Coach").await;
    let second = publish_coach(&db, author_id, author_tenant, "Tempo Coach").await;
    let first_listed = repos
        .store_listings
        .get_published_coach(&first.to_string())
        .await
        .unwrap()
        .unwrap();
    let second_listed = repos
        .store_listings
        .get_published_coach(&second.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first_listed.coach.handle.as_deref(), Some("tempo-coach"));
    assert_eq!(second_listed.coach.handle.as_deref(), Some("tempo-coach-2"));

    // The unique index refuses a second origin row with an owned handle.
    let pool = db.postgres_pool().expect("PG lane");
    let clash = sqlx::query(
        "INSERT INTO coaches (id, user_id, tenant_id, title, system_prompt, slug, created_at, updated_at) \
         VALUES ($1, $2, $3, 'Clash', 'prompt', 'tempo-coach', NOW(), NOW())",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(author_id)
    .bind(author_tenant.to_string())
    .execute(pool)
    .await;
    let err = clash.expect_err("duplicate origin handle must violate idx_coaches_handle");
    assert!(
        err.to_string().contains("idx_coaches_handle"),
        "expected idx_coaches_handle violation, got: {err}"
    );

    // Cross-tenant install copies the handle onto the athlete's row.
    let handle = CoachHandle::parse("tempo-coach").unwrap();
    let installed = repos
        .store_listings
        .install_from_store(&first.to_string(), athlete_id, athlete_tenant)
        .await
        .unwrap();
    assert_eq!(installed.handle.as_deref(), Some("tempo-coach"));
    assert_eq!(installed.forked_from, Some(first));

    let resolved = repos
        .coaches
        .find_installed_by_handle(&handle, athlete_id, athlete_tenant)
        .await
        .unwrap()
        .expect("installed coach resolves by handle on PG");
    assert_eq!(resolved.id, installed.id);
    assert_eq!(resolved.title, "Tempo Coach");

    let none = repos
        .coaches
        .find_installed_by_handle(&handle, bystander_id, TenantId::generate())
        .await
        .unwrap();
    assert!(none.is_none(), "a non-installed coach must not resolve");

    repos
        .store_listings
        .uninstall_coach(&installed.id.to_string(), athlete_id, athlete_tenant)
        .await
        .unwrap();
    assert!(repos
        .coaches
        .find_installed_by_handle(&handle, athlete_id, athlete_tenant)
        .await
        .unwrap()
        .is_none());
}
