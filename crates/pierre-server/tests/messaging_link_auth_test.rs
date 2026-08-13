// ABOUTME: HTTP-level integration tests for webhook-initiated channel link auth flow
// ABOUTME: Tests GET /messaging/link/:code and POST /messaging/link/auth endpoints
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

mod common;
mod helpers;

use chrono::{Duration, Utc};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::{Tenant, TenantId, User};
use pierre_database::backends::{CreateLinkStateParams, MessagingRepository};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::messaging::MessagingRoutes;
use tokio::task::spawn_blocking;
use uuid::Uuid;

/// Seed a tenant by creating an owner user and the tenant record. Returns the
/// `(owner_user_id, tenant_id)`. The tenant must exist before any `messaging_*`
/// row referencing its id is inserted (FK constraint).
async fn seed_tenant_with_owner(resources: &ServerContext) -> (Uuid, TenantId) {
    let email = format!("owner-{}@test.local", Uuid::new_v4());
    let user = User::new(email, "hash".to_owned(), Some("Tenant Owner".to_owned()));
    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: "Link Auth Test Tenant".to_owned(),
        slug: format!("link-auth-test-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    resources
        .common
        .repos
        .tenants
        .create(&tenant)
        .await
        .unwrap();
    (user_id, tenant_id)
}

/// Create a webhook-initiated link state (no `user_id`) and return the code
async fn create_channel_initiated_link_state(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel_type: &str,
    channel_user_id: &str,
    sender_name: Option<&str>,
    expired: bool,
) -> String {
    let code = Uuid::new_v4().to_string();
    let expires_at = if expired {
        (Utc::now() - Duration::minutes(1)).to_rfc3339()
    } else {
        (Utc::now() + Duration::minutes(10)).to_rfc3339()
    };

    let params = CreateLinkStateParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: None,
        channel_type,
        code: &code,
        method: "channel_initiated",
        channel_user_id: Some(channel_user_id),
        sender_name,
        expires_at: &expires_at,
    };
    db.create_link_state(&params).await.unwrap();
    code
}

/// Assign an already-created user to an existing tenant by updating the
/// user's `tenant_id` column. The tenant must already exist (see
/// `seed_tenant_with_owner`).
async fn add_user_to_tenant(resources: &ServerContext, user_id: Uuid, tenant_id: TenantId) {
    resources
        .common
        .repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();
}

/// Create a test user with bcrypt-hashed password and return the `user_id`
async fn create_test_user_with_password(
    resources: &ServerContext,
    email: &str,
    password: &str,
) -> Uuid {
    let password_owned = password.to_owned();
    let password_hash =
        spawn_blocking(move || bcrypt::hash(&password_owned, bcrypt::DEFAULT_COST).unwrap())
            .await
            .unwrap();

    let user = User::new(
        email.to_owned(),
        password_hash,
        Some("Test User".to_owned()),
    );
    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();
    user_id
}

// ════════════════════════════════════════════════════════════════
// GET /messaging/link/:code — Link Page Rendering
// ════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_link_page_renders_for_valid_code() {
    let resources = common::create_test_server_resources().await.unwrap();
    let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
    let (_owner_id, tenant_id) = seed_tenant_with_owner(&resources).await;

    let code = create_channel_initiated_link_state(
        db,
        tenant_id,
        "telegram",
        "tg-user-99",
        Some("Alice"),
        false,
    )
    .await;

    let app = MessagingRoutes::routes(resources);
    let resp = AxumTestRequest::get(&format!("/messaging/link/{code}"))
        .send(app)
        .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        body.contains("telegram"),
        "Page should mention channel type"
    );
    assert!(body.contains("Alice"), "Page should greet the sender");
    assert!(
        body.contains(&code),
        "Page should include the code in a hidden field"
    );
}

#[tokio::test]
async fn test_link_page_expired_code() {
    let resources = common::create_test_server_resources().await.unwrap();
    let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
    let (_owner_id, tenant_id) = seed_tenant_with_owner(&resources).await;

    let code = create_channel_initiated_link_state(
        db,
        tenant_id,
        "telegram",
        "tg-user-expired",
        None,
        true, // expired
    )
    .await;

    let app = MessagingRoutes::routes(resources);
    let resp = AxumTestRequest::get(&format!("/messaging/link/{code}"))
        .send(app)
        .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        body.contains("expired") || body.contains("invalid"),
        "Page should show error for expired code"
    );
}

#[tokio::test]
async fn test_link_page_nonexistent_code() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = MessagingRoutes::routes(resources);

    let resp = AxumTestRequest::get("/messaging/link/nonexistent-code-12345")
        .send(app)
        .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        body.contains("expired") || body.contains("invalid"),
        "Page should show error for nonexistent code"
    );
}

// ════════════════════════════════════════════════════════════════
// POST /messaging/link/auth — Login Flow
// ════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_link_auth_login_success() {
    let resources = common::create_test_server_resources().await.unwrap();
    let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
    let (_owner_id, tenant_id) = seed_tenant_with_owner(&resources).await;

    let code = create_channel_initiated_link_state(
        db,
        tenant_id,
        "whatsapp",
        "wa-user-1",
        Some("Bob"),
        false,
    )
    .await;

    // Create a user and add them to the link state's tenant
    let user_id =
        create_test_user_with_password(&resources, "bob@example.com", "SecurePass123!").await;
    add_user_to_tenant(&resources, user_id, tenant_id).await;

    let app = MessagingRoutes::routes(resources);
    let form_data = [
        ("code", code.as_str()),
        ("email", "bob@example.com"),
        ("password", "SecurePass123!"),
        ("action", "login"),
    ];

    let resp = AxumTestRequest::post("/messaging/link/auth")
        .form(&form_data)
        .send(app)
        .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        body.contains("Linked") || body.contains("linked") || body.contains("success"),
        "Should show success page, got: {}",
        &body[..body.len().min(500)]
    );
}

#[tokio::test]
async fn test_link_auth_wrong_password() {
    let resources = common::create_test_server_resources().await.unwrap();
    let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
    let (_owner_id, tenant_id) = seed_tenant_with_owner(&resources).await;

    let code = create_channel_initiated_link_state(
        db,
        tenant_id,
        "telegram",
        "tg-user-wrong-pw",
        None,
        false,
    )
    .await;

    // Create a user with known password
    create_test_user_with_password(&resources, "wrongpw@example.com", "CorrectPassword123!").await;

    let app = MessagingRoutes::routes(resources);
    let form_data = [
        ("code", code.as_str()),
        ("email", "wrongpw@example.com"),
        ("password", "WrongPassword999!"),
        ("action", "login"),
    ];

    let resp = AxumTestRequest::post("/messaging/link/auth")
        .form(&form_data)
        .send(app)
        .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        body.contains("Invalid email or password"),
        "Should show login error, got: {}",
        &body[..body.len().min(500)]
    );
}

// ════════════════════════════════════════════════════════════════
// POST /messaging/link/auth — Registration Flow
// ════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_link_auth_register_success() {
    let resources = common::create_test_server_resources().await.unwrap();
    let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
    let (_owner_id, tenant_id) = seed_tenant_with_owner(&resources).await;

    let code = create_channel_initiated_link_state(
        db,
        tenant_id,
        "discord",
        "discord-user-1",
        Some("Charlie"),
        false,
    )
    .await;

    let app = MessagingRoutes::routes(resources);
    let form_data = [
        ("code", code.as_str()),
        ("email", "charlie@example.com"),
        ("password", "NewUser123!"),
        ("action", "register"),
        ("display_name", "Charlie"),
    ];

    let resp = AxumTestRequest::post("/messaging/link/auth")
        .form(&form_data)
        .send(app)
        .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        body.contains("Linked") || body.contains("linked") || body.contains("success"),
        "Should show success page after registration, got: {}",
        &body[..body.len().min(500)]
    );
}

// ════════════════════════════════════════════════════════════════
// POST /messaging/link/auth — Error Cases
// ════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_link_auth_expired_code() {
    let resources = common::create_test_server_resources().await.unwrap();
    let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
    let (_owner_id, tenant_id) = seed_tenant_with_owner(&resources).await;

    let code = create_channel_initiated_link_state(
        db,
        tenant_id,
        "telegram",
        "tg-expired-auth",
        None,
        true, // expired
    )
    .await;

    // Create a user to try logging in as
    create_test_user_with_password(&resources, "expired@example.com", "Pass123!").await;

    let app = MessagingRoutes::routes(resources);
    let form_data = [
        ("code", code.as_str()),
        ("email", "expired@example.com"),
        ("password", "Pass123!"),
        ("action", "login"),
    ];

    let resp = AxumTestRequest::post("/messaging/link/auth")
        .form(&form_data)
        .send(app)
        .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        body.contains("expired") || body.contains("invalid"),
        "Should show error for expired code, got: {}",
        &body[..body.len().min(500)]
    );
}

#[tokio::test]
async fn test_link_auth_double_submit() {
    let resources = common::create_test_server_resources().await.unwrap();
    let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
    let (_owner_id, tenant_id) = seed_tenant_with_owner(&resources).await;

    let code = create_channel_initiated_link_state(
        db,
        tenant_id,
        "slack",
        "slack-user-double",
        None,
        false,
    )
    .await;

    // Create a user and add them to the link state's tenant
    let user_id =
        create_test_user_with_password(&resources, "double@example.com", "Pass123!").await;
    add_user_to_tenant(&resources, user_id, tenant_id).await;

    // First submission should succeed
    let app = MessagingRoutes::routes(resources.clone());
    let form_data = [
        ("code", code.as_str()),
        ("email", "double@example.com"),
        ("password", "Pass123!"),
        ("action", "login"),
    ];

    let resp = AxumTestRequest::post("/messaging/link/auth")
        .form(&form_data)
        .send(app)
        .await;
    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        body.contains("Linked") || body.contains("linked") || body.contains("success"),
        "First submit should succeed, got: {}",
        &body[..body.len().min(500)]
    );

    // Second submission with same code should fail gracefully
    let app2 = MessagingRoutes::routes(resources);
    let form_data2 = [
        ("code", code.as_str()),
        ("email", "double@example.com"),
        ("password", "Pass123!"),
        ("action", "login"),
    ];

    let resp2 = AxumTestRequest::post("/messaging/link/auth")
        .form(&form_data2)
        .send(app2)
        .await;
    assert_eq!(resp2.status(), 200);
    let body2 = resp2.text();
    assert!(
        body2.contains("expired")
            || body2.contains("invalid")
            || body2.contains("already been used"),
        "Second submit should show error, got: {}",
        &body2[..body2.len().min(500)]
    );
}

#[tokio::test]
async fn test_link_auth_register_duplicate_email() {
    let resources = common::create_test_server_resources().await.unwrap();
    let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
    let (_owner_id, tenant_id) = seed_tenant_with_owner(&resources).await;

    let code =
        create_channel_initiated_link_state(db, tenant_id, "telegram", "tg-dup-email", None, false)
            .await;

    // Create existing user
    create_test_user_with_password(&resources, "existing@example.com", "Pass123!").await;

    let app = MessagingRoutes::routes(resources);
    let form_data = [
        ("code", code.as_str()),
        ("email", "existing@example.com"),
        ("password", "DifferentPass123!"),
        ("action", "register"),
        ("display_name", "Duplicate"),
    ];

    let resp = AxumTestRequest::post("/messaging/link/auth")
        .form(&form_data)
        .send(app)
        .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        body.contains("already exists"),
        "Should show duplicate email error, got: {}",
        &body[..body.len().min(500)]
    );
}

// ════════════════════════════════════════════════════════════════
// POST /messaging/link/auth — Cross-Tenant Isolation
// ════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_link_auth_cross_tenant_rejected() {
    let resources = common::create_test_server_resources().await.unwrap();
    let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
    let (_owner_a, tenant_a) = seed_tenant_with_owner(&resources).await;
    let (_owner_b, tenant_b) = seed_tenant_with_owner(&resources).await;

    // Link state belongs to tenant A
    let code = create_channel_initiated_link_state(
        db,
        tenant_a,
        "telegram",
        "tg-cross-tenant",
        Some("CrossTest"),
        false,
    )
    .await;

    // Create user who belongs to tenant B (not tenant A)
    let user_id =
        create_test_user_with_password(&resources, "tenant_b_user@example.com", "Pass123!").await;
    add_user_to_tenant(&resources, user_id, tenant_b).await;

    let app = MessagingRoutes::routes(resources);
    let form_data = [
        ("code", code.as_str()),
        ("email", "tenant_b_user@example.com"),
        ("password", "Pass123!"),
        ("action", "login"),
    ];

    let resp = AxumTestRequest::post("/messaging/link/auth")
        .form(&form_data)
        .send(app)
        .await;

    assert_eq!(resp.status(), 200);
    let body = resp.text();
    assert!(
        body.contains("does not belong to this organization"),
        "Should reject cross-tenant link, got: {}",
        &body[..body.len().min(500)]
    );
}

// ════════════════════════════════════════════════════════════════
// GET /api/messaging/link/callback/:channel — Channel Mismatch
// ════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_link_callback_channel_mismatch_rejected() {
    let resources = common::create_test_server_resources().await.unwrap();
    let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
    let (owner_id, tenant_id) = seed_tenant_with_owner(&resources).await;
    let owner_id_str = owner_id.to_string();

    // Create a link state for "slack"
    let code = Uuid::new_v4().to_string();
    let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();
    let params = CreateLinkStateParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: Some(&owner_id_str),
        channel_type: "slack",
        code: &code,
        method: "deep_link",
        channel_user_id: None,
        sender_name: None,
        expires_at: &expires_at,
    };
    db.create_link_state(&params).await.unwrap();

    // Call callback with "telegram" channel — should reject due to mismatch
    let app = MessagingRoutes::routes(resources);
    let uri = format!(
        "/api/messaging/link/callback/telegram?state={}&channel_user_id=tg-123",
        code
    );
    let resp = AxumTestRequest::get(&uri).send(app).await;

    // Should return 400 with channel mismatch error
    assert!(
        resp.status() == 400 || resp.status() == 422,
        "Channel mismatch should be rejected, got status: {}",
        resp.status()
    );
}

// ════════════════════════════════════════════════════════════════
// GET /api/messaging/link/callback/:channel — OAuth code vs link code
// ════════════════════════════════════════════════════════════════

/// An OAuth callback carries two codes that are not interchangeable, and the
/// exchange must resolve the tenant from ours.
///
/// `state` is the link code *we* issued and the key the link-state row is
/// stored under. `code` is the provider's authorization code, meaningful only
/// to the provider. Looking the link state up by the provider's code cannot
/// match — it is not a key we ever issued — so that swap rejects every OAuth
/// link as "invalid or expired" no matter how the channel is configured.
///
/// This shipped, and no unit test caught it: the exchange's own tests drive the
/// HTTP round trip directly, so they never crossed the handler-to-exchange seam
/// where the two codes get threaded. Only a live Slack link surfaced it.
///
/// The channel is deliberately left unconfigured, which keeps the test offline:
/// a correct implementation resolves the tenant, then fails at the config
/// lookup. What it must never do is reject the link code we just created.
#[tokio::test]
async fn callback_resolves_the_link_state_by_state_not_by_the_provider_code() {
    let resources = common::create_test_server_resources().await.unwrap();
    let db: &dyn MessagingRepository = &*resources.common.repos.messaging;
    let (owner_id, tenant_id) = seed_tenant_with_owner(&resources).await;
    let owner_id_str = owner_id.to_string();

    let link_code = Uuid::new_v4().to_string();
    let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();
    let params = CreateLinkStateParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id,
        user_id: Some(&owner_id_str),
        channel_type: "slack",
        code: &link_code,
        method: "oauth",
        channel_user_id: None,
        sender_name: None,
        expires_at: &expires_at,
    };
    db.create_link_state(&params).await.unwrap();

    // A provider code that is deliberately NOT a link code we issued. With the
    // arguments swapped this is what gets looked up, and nothing matches it.
    let app = MessagingRoutes::routes(resources);
    let uri =
        format!("/api/messaging/link/callback/slack?code=provider-auth-code-xyz&state={link_code}");
    let resp = AxumTestRequest::get(&uri).send(app).await;
    let status = resp.status();
    let body = resp.text();

    assert!(
        !body.contains("invalid or expired"),
        "the link code was created seconds ago, unused and unexpired — reporting it \
         invalid means the tenant was resolved from the provider's code instead of \
         `state`. status={status}, body={}",
        &body[..body.len().min(500)]
    );
    assert_ne!(
        status,
        400,
        "a valid link code must not be rejected as bad input; the callback should get \
         past state resolution and fail later on the unconfigured channel. body={}",
        &body[..body.len().min(500)]
    );
}
