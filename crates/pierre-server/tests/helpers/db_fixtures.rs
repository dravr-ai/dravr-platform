// ABOUTME: Shared test DB fixtures — factory-opened database + user/tenant seeding for FK-satisfying tests
// ABOUTME: Included via `#[path] mod db_fixtures;` so the messaging/backfill/coverage tests reuse one copy
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, dead_code)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::similar_names,
    clippy::uninlined_format_args
)]

use chrono::Utc;
use pierre_core::models::{Tenant, TenantId, User};
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db_with_key;
use uuid::Uuid;

/// Create a migrated test database on whichever backend `DATABASE_URL`
/// selects — the factory's private `PostgreSQL` clone, or in-memory `SQLite`.
pub async fn create_test_db() -> Database {
    let encryption_key = b"test_encryption_key_32_bytes_long".to_vec();
    create_test_db_with_key(encryption_key)
        .await
        .expect("Failed to create test database")
}

/// Seed a real user and tenant so FK constraints on `messaging_*` tables are
/// satisfied. Returns `(user_uuid, tenant_id)`.
pub async fn seed_user(db: &Database) -> (Uuid, TenantId) {
    let email = format!("user-{}@test.local", Uuid::new_v4());
    let user = User::new(
        email,
        "hash_not_verified_in_tests".to_owned(),
        Some("Test User".to_owned()),
    );
    let user_id = user.id;
    db.repositories().users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    let now = Utc::now();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Test Tenant {tenant_id}"),
        slug: tenant_id.to_string(),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: now,
        updated_at: now,
    };
    db.repositories().tenants.create(&tenant).await.unwrap();

    (user_id, tenant_id)
}

/// Seed only a tenant (with a throwaway owner user). Returns the `tenant_id`.
/// Use when a test needs only a valid `tenant_id` (e.g. negative-path tests
/// that never touch `user_id`).
pub async fn seed_tenant(db: &Database) -> TenantId {
    let (_, tenant_id) = seed_user(db).await;
    tenant_id
}
