// ABOUTME: Integration tests for messaging channel linking (OAuth/deep-link account verification)
// ABOUTME: Validates link state lifecycle, channel link CRUD, code consumption, and tenant isolation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(
    clippy::wildcard_in_or_patterns,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::module_name_repetitions,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::uninlined_format_args,
    clippy::redundant_closure_for_method_calls
)]

use chrono::{Duration, Utc};
use pierre_database::{
    backends::{factory::Database, CreateChannelLinkParams, CreateLinkStateParams},
    DatabaseProvider,
};
#[cfg(feature = "postgresql")]
use pierre_mcp_server::config::environment::PostgresPoolConfig;
use pierre_mcp_server::models::{Tenant, TenantId, User};
use uuid::Uuid;

/// Create an in-memory `SQLite` database for testing
async fn create_test_db() -> Database {
    let encryption_key = b"test_encryption_key_32_bytes_long".to_vec();

    #[cfg(feature = "postgresql")]
    let db = Database::new(
        "sqlite::memory:",
        encryption_key,
        &PostgresPoolConfig::default(),
    )
    .await
    .expect("Failed to create test database");

    #[cfg(not(feature = "postgresql"))]
    let db = Database::new("sqlite::memory:", encryption_key)
        .await
        .expect("Failed to create test database");

    db.migrate().await.expect("Failed to run migrations");
    db
}

/// Seed a real user and tenant so FK constraints on `messaging_*` tables are satisfied.
/// Returns `(user_uuid, tenant_id)`; stringify `user_uuid` when passing to messaging params.
async fn seed_user(db: &Database) -> (Uuid, TenantId) {
    let email = format!("user-{}@test.local", Uuid::new_v4());
    let user = User::new(
        email,
        "hash_not_verified_in_tests".to_owned(),
        Some("Test User".to_owned()),
    );
    let user_id = user.id;
    db.repositories().users.create(&user).await.unwrap();

    let tenant_id = TenantId::new();
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

/// Seed only a tenant (with an owner user). Returns the `tenant_id`. Use when a test
/// needs only a valid `tenant_id` (e.g. negative-path tests that never touch `user_id`).
async fn seed_tenant(db: &Database) -> TenantId {
    let (_, tenant_id) = seed_user(db).await;
    tenant_id
}

// ════════════════════════════════════════════════════════════════
// Link State Tests
// ════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_create_and_consume_link_state() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let code = Uuid::new_v4().to_string();
    let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();

    let params = CreateLinkStateParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: Some(&user_id),
        channel_type: "telegram",
        code: &code,
        method: "deep_link",
        channel_user_id: None,
        sender_name: None,
        expires_at: &expires_at,
    };
    db.repositories()
        .messaging
        .create_link_state(&params)
        .await
        .unwrap();

    // Consume should succeed
    let state = db
        .repositories()
        .messaging
        .consume_link_state(&code, tenant_id)
        .await
        .unwrap();
    assert_eq!(state["user_id"].as_str().unwrap(), user_id);
    assert_eq!(state["channel_type"].as_str().unwrap(), "telegram");
    assert_eq!(state["method"].as_str().unwrap(), "deep_link");
}

#[tokio::test]
async fn test_consume_link_state_already_used() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let code = Uuid::new_v4().to_string();
    let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();

    let params = CreateLinkStateParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: Some(&user_id),
        channel_type: "whatsapp",
        code: &code,
        method: "deep_link",
        channel_user_id: None,
        sender_name: None,
        expires_at: &expires_at,
    };
    db.repositories()
        .messaging
        .create_link_state(&params)
        .await
        .unwrap();

    // First consumption succeeds
    db.repositories()
        .messaging
        .consume_link_state(&code, tenant_id)
        .await
        .unwrap();

    // Second consumption should fail (already used)
    let err = db
        .repositories()
        .messaging
        .consume_link_state(&code, tenant_id)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("already been used"),
        "Expected 'already used' error, got: {err}"
    );
}

#[tokio::test]
async fn test_consume_link_state_expired() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let code = Uuid::new_v4().to_string();
    // Already expired
    let expires_at = (Utc::now() - Duration::minutes(1)).to_rfc3339();

    let params = CreateLinkStateParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: Some(&user_id),
        channel_type: "slack",
        code: &code,
        method: "oauth",
        channel_user_id: None,
        sender_name: None,
        expires_at: &expires_at,
    };
    db.repositories()
        .messaging
        .create_link_state(&params)
        .await
        .unwrap();

    // Should fail (expired)
    let err = db
        .repositories()
        .messaging
        .consume_link_state(&code, tenant_id)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("expired"),
        "Expected 'expired' error, got: {err}"
    );
}

#[tokio::test]
async fn test_consume_link_state_nonexistent() {
    let db = create_test_db().await;
    let tenant_id = seed_tenant(&db).await;

    let err = db
        .repositories()
        .messaging
        .consume_link_state("nonexistent-code", tenant_id)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("expired"),
        "Expected 'expired' error for missing code, got: {err}"
    );
}

// ════════════════════════════════════════════════════════════════
// Channel Link Tests
// ════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_create_and_get_channel_link() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();

    let params = CreateChannelLinkParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: &user_id,
        channel_type: "telegram",
        channel_user_id: "tg-user-42",
        display_name: Some("Alice"),
    };
    db.repositories()
        .messaging
        .create_channel_link(&params)
        .await
        .unwrap();

    // Get by channel identity
    let link = db
        .repositories()
        .messaging
        .get_channel_link(tenant_id, "telegram", "tg-user-42")
        .await
        .unwrap()
        .expect("Channel link should exist");

    assert_eq!(link["user_id"].as_str().unwrap(), user_id);
    assert_eq!(link["channel_type"].as_str().unwrap(), "telegram");
    assert_eq!(link["channel_user_id"].as_str().unwrap(), "tg-user-42");
    assert_eq!(link["display_name"].as_str().unwrap(), "Alice");
}

#[tokio::test]
async fn test_channel_link_not_found() {
    let db = create_test_db().await;
    let tenant_id = seed_tenant(&db).await;

    let link = db
        .repositories()
        .messaging
        .get_channel_link(tenant_id, "telegram", "nonexistent")
        .await
        .unwrap();
    assert!(link.is_none());
}

#[tokio::test]
async fn test_channel_link_duplicate_rejected() {
    let db = create_test_db().await;
    let (user_uuid_a, tenant_id) = seed_user(&db).await;
    let user_id_a = user_uuid_a.to_string();
    // Second user in same tenant for the duplicate attempt.
    let user_uuid_b = {
        let email = format!("user-{}@test.local", Uuid::new_v4());
        let u = User::new(email, "hash".to_owned(), Some("Other".to_owned()));
        let id = u.id;
        db.repositories().users.create(&u).await.unwrap();
        id
    };
    let user_id_b = user_uuid_b.to_string();

    let params = CreateChannelLinkParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: &user_id_a,
        channel_type: "slack",
        channel_user_id: "slack-user-1",
        display_name: None,
    };
    db.repositories()
        .messaging
        .create_channel_link(&params)
        .await
        .unwrap();

    // Duplicate should fail
    let params2 = CreateChannelLinkParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: &user_id_b,
        channel_type: "slack",
        channel_user_id: "slack-user-1",
        display_name: None,
    };
    let err = db
        .repositories()
        .messaging
        .create_channel_link(&params2)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("already linked"),
        "Expected 'already linked' error, got: {err}"
    );
}

#[tokio::test]
async fn test_list_user_channel_links() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();

    // Link two channels for the same user
    let params1 = CreateChannelLinkParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: &user_id,
        channel_type: "telegram",
        channel_user_id: "tg-400",
        display_name: Some("Bob TG"),
    };
    db.repositories()
        .messaging
        .create_channel_link(&params1)
        .await
        .unwrap();

    let params2 = CreateChannelLinkParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: &user_id,
        channel_type: "whatsapp",
        channel_user_id: "wa-400",
        display_name: Some("Bob WA"),
    };
    db.repositories()
        .messaging
        .create_channel_link(&params2)
        .await
        .unwrap();

    let links = db
        .repositories()
        .messaging
        .list_user_channel_links(tenant_id, &user_id)
        .await
        .unwrap();
    assert_eq!(links.len(), 2);
}

#[tokio::test]
async fn test_list_user_channel_links_empty() {
    let db = create_test_db().await;
    let tenant_id = seed_tenant(&db).await;

    let links = db
        .repositories()
        .messaging
        .list_user_channel_links(tenant_id, "nonexistent-user")
        .await
        .unwrap();
    assert!(links.is_empty());
}

#[tokio::test]
async fn test_delete_channel_link() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();

    let params = CreateChannelLinkParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: &user_id,
        channel_type: "discord",
        channel_user_id: "discord-500",
        display_name: None,
    };
    db.repositories()
        .messaging
        .create_channel_link(&params)
        .await
        .unwrap();

    // Delete should succeed
    let deleted = db
        .repositories()
        .messaging
        .delete_channel_link(tenant_id, &user_id, "discord")
        .await
        .unwrap();
    assert!(deleted);

    // Verify it's gone
    let link = db
        .repositories()
        .messaging
        .get_channel_link(tenant_id, "discord", "discord-500")
        .await
        .unwrap();
    assert!(link.is_none());
}

#[tokio::test]
async fn test_delete_channel_link_not_found() {
    let db = create_test_db().await;
    let tenant_id = seed_tenant(&db).await;

    let deleted = db
        .repositories()
        .messaging
        .delete_channel_link(tenant_id, "user-600", "telegram")
        .await
        .unwrap();
    assert!(!deleted);
}

// ════════════════════════════════════════════════════════════════
// Tenant Isolation Tests
// ════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_channel_link_tenant_isolation() {
    let db = create_test_db().await;
    let (user_uuid_a, tenant_a) = seed_user(&db).await;
    let (_user_uuid_b, tenant_b) = seed_user(&db).await;
    let user_id_a = user_uuid_a.to_string();

    // Create a link for tenant A
    let params = CreateChannelLinkParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id: tenant_a,
        user_id: &user_id_a,
        channel_type: "telegram",
        channel_user_id: "tg-iso",
        display_name: None,
    };
    db.repositories()
        .messaging
        .create_channel_link(&params)
        .await
        .unwrap();

    // Tenant A can see it
    let link = db
        .repositories()
        .messaging
        .get_channel_link(tenant_a, "telegram", "tg-iso")
        .await
        .unwrap();
    assert!(link.is_some());

    // Tenant B cannot see it
    let link = db
        .repositories()
        .messaging
        .get_channel_link(tenant_b, "telegram", "tg-iso")
        .await
        .unwrap();
    assert!(link.is_none());
}

#[tokio::test]
async fn test_channel_link_different_tenants_same_channel_user() {
    let db = create_test_db().await;
    let (user_uuid_a, tenant_a) = seed_user(&db).await;
    let (user_uuid_b, tenant_b) = seed_user(&db).await;
    let user_id_a = user_uuid_a.to_string();
    let user_id_b = user_uuid_b.to_string();

    // Same channel_user_id can link to different tenants
    let params_a = CreateChannelLinkParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id: tenant_a,
        user_id: &user_id_a,
        channel_type: "slack",
        channel_user_id: "shared-slack-id",
        display_name: None,
    };
    db.repositories()
        .messaging
        .create_channel_link(&params_a)
        .await
        .unwrap();

    let params_b = CreateChannelLinkParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id: tenant_b,
        user_id: &user_id_b,
        channel_type: "slack",
        channel_user_id: "shared-slack-id",
        display_name: None,
    };
    db.repositories()
        .messaging
        .create_channel_link(&params_b)
        .await
        .unwrap();

    // Each tenant sees their own link
    let link_a = db
        .repositories()
        .messaging
        .get_channel_link(tenant_a, "slack", "shared-slack-id")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(link_a["user_id"].as_str().unwrap(), user_id_a);

    let link_b = db
        .repositories()
        .messaging
        .get_channel_link(tenant_b, "slack", "shared-slack-id")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(link_b["user_id"].as_str().unwrap(), user_id_b);
}

#[tokio::test]
async fn test_delete_channel_link_tenant_isolation() {
    let db = create_test_db().await;
    let (user_uuid_a, tenant_a) = seed_user(&db).await;
    let (_user_uuid_b, tenant_b) = seed_user(&db).await;
    let user_id_a = user_uuid_a.to_string();

    let params = CreateChannelLinkParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id: tenant_a,
        user_id: &user_id_a,
        channel_type: "messenger",
        channel_user_id: "msn-del-iso",
        display_name: None,
    };
    db.repositories()
        .messaging
        .create_channel_link(&params)
        .await
        .unwrap();

    // Tenant B trying to delete tenant A's link should fail
    let deleted = db
        .repositories()
        .messaging
        .delete_channel_link(tenant_b, &user_id_a, "messenger")
        .await
        .unwrap();
    assert!(!deleted);

    // Tenant A's link should still exist
    let link = db
        .repositories()
        .messaging
        .get_channel_link(tenant_a, "messenger", "msn-del-iso")
        .await
        .unwrap();
    assert!(link.is_some());
}

// ════════════════════════════════════════════════════════════════
// End-to-end Link State → Channel Link Flow
// ════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_full_linking_flow() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let seeded_user_id = user_uuid.to_string();
    let code = Uuid::new_v4().to_string();
    let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();

    // Step 1: Create link state
    let state_params = CreateLinkStateParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: Some(&seeded_user_id),
        channel_type: "telegram",
        code: &code,
        method: "deep_link",
        channel_user_id: None,
        sender_name: None,
        expires_at: &expires_at,
    };
    db.repositories()
        .messaging
        .create_link_state(&state_params)
        .await
        .unwrap();

    // Step 2: Consume the code
    let consumed = db
        .repositories()
        .messaging
        .consume_link_state(&code, tenant_id)
        .await
        .unwrap();
    let user_id = consumed["user_id"].as_str().unwrap();
    assert_eq!(user_id, seeded_user_id);

    // Step 3: Create the channel link
    let link_params = CreateChannelLinkParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id,
        channel_type: "telegram",
        channel_user_id: "tg-e2e-user",
        display_name: Some("E2E User"),
    };
    db.repositories()
        .messaging
        .create_channel_link(&link_params)
        .await
        .unwrap();

    // Step 4: Verify the link exists
    let link = db
        .repositories()
        .messaging
        .get_channel_link(tenant_id, "telegram", "tg-e2e-user")
        .await
        .unwrap()
        .expect("Link should exist after full flow");
    assert_eq!(link["user_id"].as_str().unwrap(), seeded_user_id);

    // Step 5: List user's links
    let links = db
        .repositories()
        .messaging
        .list_user_channel_links(tenant_id, &seeded_user_id)
        .await
        .unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["channel_type"].as_str().unwrap(), "telegram");

    // Step 6: Unlink
    let deleted = db
        .repositories()
        .messaging
        .delete_channel_link(tenant_id, &seeded_user_id, "telegram")
        .await
        .unwrap();
    assert!(deleted);

    // Step 7: Verify gone
    let links = db
        .repositories()
        .messaging
        .list_user_channel_links(tenant_id, &seeded_user_id)
        .await
        .unwrap();
    assert!(links.is_empty());
}

// ════════════════════════════════════════════════════════════════
// Webhook-Initiated Link State Tests (nullable user_id)
// ════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_create_link_state_without_user_id() {
    let db = create_test_db().await;
    let tenant_id = seed_tenant(&db).await;
    let code = Uuid::new_v4().to_string();
    let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();

    let params = CreateLinkStateParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: None,
        channel_type: "telegram",
        code: &code,
        method: "channel_initiated",
        channel_user_id: Some("tg-sender-42"),
        sender_name: Some("Alice"),
        expires_at: &expires_at,
    };
    db.repositories()
        .messaging
        .create_link_state(&params)
        .await
        .unwrap();

    // Should be readable via get_link_state
    let state = db
        .repositories()
        .messaging
        .get_link_state(&code)
        .await
        .unwrap();
    assert!(state.is_some(), "Link state should be readable");
    let state = state.unwrap();
    assert!(state["user_id"].is_null(), "user_id should be null");
    assert_eq!(state["channel_type"].as_str().unwrap(), "telegram");
    assert_eq!(state["channel_user_id"].as_str().unwrap(), "tg-sender-42");
    assert_eq!(state["sender_name"].as_str().unwrap(), "Alice");
}

#[tokio::test]
async fn test_get_link_state_valid() {
    let db = create_test_db().await;
    let tenant_id = seed_tenant(&db).await;
    let code = Uuid::new_v4().to_string();
    let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();

    let params = CreateLinkStateParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: None,
        channel_type: "slack",
        code: &code,
        method: "channel_initiated",
        channel_user_id: Some("slack-u1"),
        sender_name: None,
        expires_at: &expires_at,
    };
    db.repositories()
        .messaging
        .create_link_state(&params)
        .await
        .unwrap();

    let state = db
        .repositories()
        .messaging
        .get_link_state(&code)
        .await
        .unwrap();
    assert!(state.is_some());
}

#[tokio::test]
async fn test_get_link_state_expired() {
    let db = create_test_db().await;
    let tenant_id = seed_tenant(&db).await;
    let code = Uuid::new_v4().to_string();
    let expires_at = (Utc::now() - Duration::minutes(1)).to_rfc3339();

    let params = CreateLinkStateParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: None,
        channel_type: "slack",
        code: &code,
        method: "channel_initiated",
        channel_user_id: None,
        sender_name: None,
        expires_at: &expires_at,
    };
    db.repositories()
        .messaging
        .create_link_state(&params)
        .await
        .unwrap();

    let state = db
        .repositories()
        .messaging
        .get_link_state(&code)
        .await
        .unwrap();
    assert!(state.is_none(), "Expired link state should not be returned");
}

#[tokio::test]
async fn test_get_link_state_used() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let code = Uuid::new_v4().to_string();
    let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();

    let params = CreateLinkStateParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: Some(&user_id),
        channel_type: "telegram",
        code: &code,
        method: "deep_link",
        channel_user_id: None,
        sender_name: None,
        expires_at: &expires_at,
    };
    db.repositories()
        .messaging
        .create_link_state(&params)
        .await
        .unwrap();

    // Consume it
    db.repositories()
        .messaging
        .consume_link_state(&code, tenant_id)
        .await
        .unwrap();

    // get_link_state should return None (used)
    let state = db
        .repositories()
        .messaging
        .get_link_state(&code)
        .await
        .unwrap();
    assert!(state.is_none(), "Used link state should not be returned");
}

#[tokio::test]
async fn test_complete_link_state_success() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let code = Uuid::new_v4().to_string();
    let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();

    let params = CreateLinkStateParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: None,
        channel_type: "telegram",
        code: &code,
        method: "channel_initiated",
        channel_user_id: Some("tg-42"),
        sender_name: Some("Bob"),
        expires_at: &expires_at,
    };
    db.repositories()
        .messaging
        .create_link_state(&params)
        .await
        .unwrap();

    // Complete should succeed and set user_id
    let result = db
        .repositories()
        .messaging
        .complete_link_state(&code, &user_id)
        .await
        .unwrap();
    assert_eq!(result["user_id"].as_str().unwrap(), user_id);
    assert_eq!(result["channel_user_id"].as_str().unwrap(), "tg-42");
    assert_eq!(result["sender_name"].as_str().unwrap(), "Bob");

    // Should no longer be gettable (it's now used)
    let state = db
        .repositories()
        .messaging
        .get_link_state(&code)
        .await
        .unwrap();
    assert!(state.is_none(), "Completed link state should be consumed");
}

#[tokio::test]
async fn test_complete_link_state_already_used() {
    let db = create_test_db().await;
    let (user_uuid_1, tenant_id) = seed_user(&db).await;
    let user_id_1 = user_uuid_1.to_string();
    let (user_uuid_2, _) = seed_user(&db).await;
    let user_id_2 = user_uuid_2.to_string();
    let code = Uuid::new_v4().to_string();
    let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();

    let params = CreateLinkStateParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: None,
        channel_type: "telegram",
        code: &code,
        method: "channel_initiated",
        channel_user_id: Some("tg-43"),
        sender_name: None,
        expires_at: &expires_at,
    };
    db.repositories()
        .messaging
        .create_link_state(&params)
        .await
        .unwrap();

    // Complete first time
    db.repositories()
        .messaging
        .complete_link_state(&code, &user_id_1)
        .await
        .unwrap();

    // Second attempt should fail
    let err = db
        .repositories()
        .messaging
        .complete_link_state(&code, &user_id_2)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("already been used"),
        "Expected 'already used' error, got: {err}"
    );
}

#[tokio::test]
async fn test_complete_link_state_expired() {
    let db = create_test_db().await;
    let (user_uuid, tenant_id) = seed_user(&db).await;
    let user_id = user_uuid.to_string();
    let code = Uuid::new_v4().to_string();
    let expires_at = (Utc::now() - Duration::minutes(1)).to_rfc3339();

    let params = CreateLinkStateParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: None,
        channel_type: "telegram",
        code: &code,
        method: "channel_initiated",
        channel_user_id: None,
        sender_name: None,
        expires_at: &expires_at,
    };
    db.repositories()
        .messaging
        .create_link_state(&params)
        .await
        .unwrap();

    let err = db
        .repositories()
        .messaging
        .complete_link_state(&code, &user_id)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("expired"),
        "Expected 'expired' error, got: {err}"
    );
}

#[tokio::test]
async fn test_complete_link_state_already_has_user() {
    let db = create_test_db().await;
    let (existing_uuid, tenant_id) = seed_user(&db).await;
    let existing_user = existing_uuid.to_string();
    let (another_uuid, _) = seed_user(&db).await;
    let another_user = another_uuid.to_string();
    let code = Uuid::new_v4().to_string();
    let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();

    // Create a link state WITH a user_id (web-initiated)
    let params = CreateLinkStateParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: Some(&existing_user),
        channel_type: "telegram",
        code: &code,
        method: "deep_link",
        channel_user_id: None,
        sender_name: None,
        expires_at: &expires_at,
    };
    db.repositories()
        .messaging
        .create_link_state(&params)
        .await
        .unwrap();

    // complete_link_state should fail because user_id is already set
    let err = db
        .repositories()
        .messaging
        .complete_link_state(&code, &another_user)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("cannot be completed"),
        "Expected 'not completable' error, got: {err}"
    );
}

// ════════════════════════════════════════════════════════════════
// Tenant-Scoped consume_link_state Tests
// ════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_consume_link_state_requires_tenant_id() {
    let db = create_test_db().await;
    let (user_uuid_a, tenant_a) = seed_user(&db).await;
    let tenant_b = seed_tenant(&db).await;
    let user_id_a = user_uuid_a.to_string();
    let code = Uuid::new_v4().to_string();
    let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();

    // Create link state for tenant A
    let params = CreateLinkStateParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id: tenant_a,
        user_id: Some(&user_id_a),
        channel_type: "telegram",
        code: &code,
        method: "deep_link",
        channel_user_id: None,
        sender_name: None,
        expires_at: &expires_at,
    };
    db.repositories()
        .messaging
        .create_link_state(&params)
        .await
        .unwrap();

    // Consuming with tenant B should fail (cross-tenant)
    let err = db
        .repositories()
        .messaging
        .consume_link_state(&code, tenant_b)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("expired")
            || err.to_string().contains("not found")
            || err.to_string().contains("invalid"),
        "Cross-tenant consume should be rejected, got: {err}"
    );

    // Consuming with tenant A (correct tenant) should succeed
    let state = db
        .repositories()
        .messaging
        .consume_link_state(&code, tenant_a)
        .await
        .unwrap();
    assert_eq!(state["user_id"].as_str().unwrap(), user_id_a);
    assert_eq!(state["channel_type"].as_str().unwrap(), "telegram");
}
