// ABOUTME: carnet#1391 — every admin write through the console or pierre-cli is one row of the audit trail
// ABOUTME: Actor, route, target and outcome recorded; refusals too; reads write nothing; a deletion leaves no trace

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! On 2026-11-04 an operator removed three pre-approved emails from the
//! console and believed he had deleted users. Nothing the server kept could
//! say which: its log lines named neither the admin nor the address, and the
//! `audit_events` table had no writer. These tests drive the app the
//! composition root serves (`ProviderToolRouter::build_http_app`), so the
//! recorder is exercised where it is mounted, and assert on the rows it wrote.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use anyhow::Result;
use pierre_core::admin::models::{CreateAdminTokenRequest, DEVICE_CLI_SERVICE_PREFIX};
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_database::repositories::{
    subject_hash, AuditEventFilter, AuditEventRecord, AuditOutcome,
};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use secrecy::ExposeSecret;
use serde_json::{json, Value};
use serial_test::serial;
use uuid::Uuid;

use crate::common::{create_test_server_resources, generate_test_token};
use crate::helpers::axum_test::AxumTestRequest;

/// A signed-in admin: the bearer header and the account id.
struct Admin {
    auth: String,
    user_id: Uuid,
}

/// An active admin with `role`, owning a tenant, signed in.
async fn admin(resources: &Arc<ServerContext>, email: &str, role: UserRole) -> Result<Admin> {
    let mut user = User::new(
        email.to_owned(),
        bcrypt::hash("password123", 4)?,
        Some("Audit Admin".to_owned()),
    );
    user.is_admin = true;
    user.role = role;
    user.user_status = UserStatus::Active;
    let user_id = user.id;
    resources.common.repos.users.create(&user).await?;
    let tenant_id = TenantId::generate();
    resources
        .common
        .repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: format!("Tenant for {email}"),
            slug: format!("tenant-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .await?;
    resources
        .common
        .repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await?;
    let token = generate_test_token(resources, &user).await;
    Ok(Admin {
        auth: format!("Bearer {token}"),
        user_id,
    })
}

/// A plain active athlete with no tenant.
async fn athlete(resources: &Arc<ServerContext>, email: &str) -> Result<Uuid> {
    let mut user = User::new(email.to_owned(), bcrypt::hash("password123", 4)?, None);
    user.user_status = UserStatus::Active;
    let user_id = user.id;
    resources.common.repos.users.create(&user).await?;
    Ok(user_id)
}

/// A super-admin `pierre-cli` device-login token acting as `operator`.
async fn cli_token(resources: &Arc<ServerContext>, operator: &Admin) -> Result<String> {
    let generated = resources
        .common
        .repos
        .admin
        .create_token(
            &CreateAdminTokenRequest {
                service_name: format!("{DEVICE_CLI_SERVICE_PREFIX}ops@example.com"),
                service_description: None,
                permissions: None,
                expires_in_days: Some(1),
                is_super_admin: true,
                tenant_id: None,
                operator_user_id: Some(operator.user_id),
            },
            resources.auth.admin_jwt_secret.as_ref(),
            &resources.auth.jwks_manager,
        )
        .await?;
    Ok(format!("Bearer {}", generated.jwt_token.expose_secret()))
}

/// Every row of the trail, newest first.
async fn trail(resources: &Arc<ServerContext>) -> Result<Vec<AuditEventRecord>> {
    Ok(resources
        .common
        .repos
        .admin_audit
        .list(&AuditEventFilter::default(), 200)
        .await?)
}

#[tokio::test]
#[serial]
async fn a_console_write_is_recorded_with_its_actor_route_and_target() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let app = ProviderToolRouter::build_http_app(&resources);
    let operator = admin(&resources, "audit-console@example.com", UserRole::Admin).await?;
    let target = "invited-athlete@example.com";

    let allowed = AxumTestRequest::post("/api/admin/pre-approved-emails")
        .header("Authorization", &operator.auth)
        .json(&json!({ "email": target }))
        .send(app.clone())
        .await;
    assert_eq!(allowed.status(), 200);
    let listed = AxumTestRequest::get("/api/admin/pre-approved-emails")
        .header("Authorization", &operator.auth)
        .send(app.clone())
        .await;
    assert_eq!(listed.status(), 200);
    let removed = AxumTestRequest::delete(&format!(
        "/api/admin/pre-approved-emails/{}",
        urlencoding::encode(target)
    ))
    .header("Authorization", &operator.auth)
    .send(app.clone())
    .await;
    assert_eq!(removed.status(), 200);

    let rows = trail(&resources).await?;
    assert_eq!(
        rows.len(),
        2,
        "two writes, two rows; the read wrote none: {rows:?}"
    );
    let removal = &rows[0];
    assert_eq!(removal.action, "pre_approval.remove");
    assert_eq!(
        removal.metadata["route"], "/api/admin/pre-approved-emails/{email}",
        "the route template, never the raw URL"
    );
    assert!(
        removal.summary.contains("no account deleted"),
        "the trail says what the action did not do: {}",
        removal.summary
    );
    assert_eq!(removal.outcome, "success");
    assert_eq!(removal.source, "console");
    assert_eq!(
        removal.actor_user_id.as_deref(),
        Some(operator.user_id.to_string().as_str())
    );
    assert_eq!(
        removal.actor_email.as_deref(),
        Some("audit-console@example.com")
    );
    assert_eq!(removal.metadata["params"]["email"], target, "{removal:?}");
    assert_eq!(removal.metadata["status"], 200);
    assert!(
        removal.metadata.get("token_id").is_none(),
        "a console session's token id is the actor's own id, already on the row"
    );
    assert_eq!(rows[1].action, "pre_approval.allow");
    assert_eq!(
        rows[1].metadata["target"]["email"], target,
        "the body's address is named"
    );

    // An action key holding `_` filters by its own text, not as a wildcard.
    let filtered = AxumTestRequest::get("/api/admin/audit-events?action=pre_approval.")
        .header("Authorization", &operator.auth)
        .send(app.clone())
        .await;
    assert_eq!(filtered.status(), 200, "{}", filtered.body_text());
    let body: Value = filtered.json();
    let actions: Vec<&str> = body["data"]["events"]
        .as_array()
        .expect("events")
        .iter()
        .filter_map(|e| e["action"].as_str())
        .collect();
    assert_eq!(
        actions,
        ["pre_approval.remove", "pre_approval.allow"],
        "the prefix `pre_approval.` selects both pre-approval rows: {body}"
    );
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_sign_up_settings_change_records_what_it_was_and_what_it_became() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let app = ProviderToolRouter::build_http_app(&resources);
    let operator = admin(&resources, "audit-settings@example.com", UserRole::Admin).await?;

    let response = AxumTestRequest::put("/api/admin/settings/signup")
        .header("Authorization", &operator.auth)
        .json(&json!({ "daily_limit": 1000 }))
        .send(app.clone())
        .await;
    assert_eq!(response.status(), 200);

    let rows = trail(&resources).await?;
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(row.action, "settings.signup.update");
    assert_eq!(row.metadata["after"]["daily_limit"], 1000);
    assert_ne!(
        row.metadata["before"]["daily_limit"], row.metadata["after"]["daily_limit"],
        "the row must say what the limit was: {row:?}"
    );
    assert!(
        row.summary.contains("daily limit") && row.summary.contains("1000"),
        "the sentence names the change: {}",
        row.summary
    );
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_refused_write_is_recorded_and_a_stranger_writes_nothing() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let app = ProviderToolRouter::build_http_app(&resources);
    let operator = admin(&resources, "audit-plain@example.com", UserRole::Admin).await?;
    let coach = athlete(&resources, "audit-coach@example.com").await?;

    // Super-admin only: a plain admin is refused.
    let refused = AxumTestRequest::post(&format!("/api/admin/users/{coach}/manages-roster"))
        .header("Authorization", &operator.auth)
        .json(&json!({ "manages_roster": true }))
        .send(app.clone())
        .await;
    assert_eq!(refused.status(), 403);

    // No credential at all is not an operator's action.
    let stranger = AxumTestRequest::post(&format!("/api/admin/suspend-user/{coach}"))
        .json(&json!({ "reason": "probe" }))
        .send(app.clone())
        .await;
    assert_eq!(stranger.status(), 401);

    let rows = trail(&resources).await?;
    assert_eq!(rows.len(), 1, "only the refusal: {rows:?}");
    assert_eq!(rows[0].outcome, "refused");
    assert_eq!(rows[0].metadata["status"], 403);
    assert_eq!(rows[0].metadata["params"]["user_id"], coach.to_string());

    let refused_only = resources
        .common
        .repos
        .admin_audit
        .list(
            &AuditEventFilter {
                outcome: Some(AuditOutcome::Success),
                ..AuditEventFilter::default()
            },
            50,
        )
        .await?;
    assert!(refused_only.is_empty(), "the outcome filter narrows");
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_deleted_account_leaves_no_trace_in_the_trail() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let app = ProviderToolRouter::build_http_app(&resources);
    let operator = admin(&resources, "audit-super@example.com", UserRole::SuperAdmin).await?;
    let email = "leaving-athlete@example.com";
    let leaving = athlete(&resources, email).await?;

    // An earlier action that names the account, by id and by email.
    let suspended = AxumTestRequest::post(&format!("/api/admin/suspend-user/{leaving}"))
        .header("Authorization", &operator.auth)
        .json(&json!({ "reason": "asked to leave" }))
        .send(app.clone())
        .await;
    assert_eq!(suspended.status(), 200);
    let allowed = AxumTestRequest::post("/api/admin/pre-approved-emails")
        .header("Authorization", &operator.auth)
        .json(&json!({ "email": email }))
        .send(app.clone())
        .await;
    assert_eq!(allowed.status(), 200);
    // Another address that contains the leaving one: its row is not about
    // the deleted account and must survive the deletion.
    let bystander = "also-leaving-athlete@example.com";
    let allowed = AxumTestRequest::post("/api/admin/pre-approved-emails")
        .header("Authorization", &operator.auth)
        .json(&json!({ "email": bystander }))
        .send(app.clone())
        .await;
    assert_eq!(allowed.status(), 200);
    assert_eq!(trail(&resources).await?.len(), 3);

    let deleted = AxumTestRequest::delete(&format!("/admin/users/{leaving}"))
        .header("Authorization", &cli_token(&resources, &operator).await?)
        .json(&json!({ "reason": "account closure" }))
        .send(app.clone())
        .await;
    assert_eq!(deleted.status(), 200, "{:?}", deleted.json::<Value>());

    let rows = trail(&resources).await?;
    assert_eq!(
        rows.len(),
        2,
        "the deletion's own row and the bystander's are left: {rows:?}"
    );
    let row = &rows[0];
    assert_eq!(row.action, "user.delete");
    assert_eq!(row.source, "cli");
    assert_eq!(row.outcome, "success");
    assert!(row.metadata["target"]["receipt_id"].is_string(), "{row:?}");
    assert_eq!(
        row.metadata["target"]["subject_hash"],
        subject_hash(leaving),
        "the row carries the subject hash the tombstones carry: {row:?}"
    );
    assert_eq!(
        rows[1].metadata["target"]["email"], bystander,
        "a row about another address survives: {rows:?}"
    );
    let text = serde_json::to_string(row)?;
    assert!(
        !text.contains(&leaving.to_string()) && !text.contains(email),
        "the deletion's row may not name the deleted account: {text}"
    );
    let text = serde_json::to_string(&rows)?;
    assert!(
        !text.contains(&leaving.to_string()) && !text.contains(&format!("\"{email}\"")),
        "nothing in the trail may name the deleted account: {text}"
    );
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_tenant_admin_reads_only_its_own_tenant_and_a_super_admin_reads_all() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let app = ProviderToolRouter::build_http_app(&resources);
    let writer = admin(&resources, "audit-tenant-a@example.com", UserRole::Admin).await?;
    let other = admin(&resources, "audit-tenant-b@example.com", UserRole::Admin).await?;
    let platform = admin(
        &resources,
        "audit-platform@example.com",
        UserRole::SuperAdmin,
    )
    .await?;

    let allowed = AxumTestRequest::post("/api/admin/pre-approved-emails")
        .header("Authorization", &writer.auth)
        .json(&json!({ "email": "tenant-a-invite@example.com" }))
        .send(app.clone())
        .await;
    assert_eq!(allowed.status(), 200);

    let read = |auth: String| {
        let app = app.clone();
        async move {
            let response = AxumTestRequest::get("/api/admin/audit-events")
                .header("Authorization", &auth)
                .send(app)
                .await;
            assert_eq!(response.status(), 200, "{}", response.body_text());
            let body: Value = response.json();
            body["data"]["events"].as_array().expect("events").len()
        }
    };
    assert_eq!(
        read(writer.auth.clone()).await,
        1,
        "the writer's tenant sees its row"
    );
    assert_eq!(
        read(other.auth.clone()).await,
        0,
        "another tenant's admin must not read this tenant's trail"
    );
    assert_eq!(
        read(platform.auth.clone()).await,
        1,
        "a super-admin reads every tenant's rows"
    );
    Ok(())
}

#[tokio::test]
#[serial]
async fn the_console_reads_the_trail_newest_first() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let app = ProviderToolRouter::build_http_app(&resources);
    let operator = admin(&resources, "audit-reader@example.com", UserRole::SuperAdmin).await?;
    for email in ["first@example.com", "second@example.com"] {
        let response = AxumTestRequest::post("/api/admin/pre-approved-emails")
            .header("Authorization", &operator.auth)
            .json(&json!({ "email": email }))
            .send(app.clone())
            .await;
        assert_eq!(response.status(), 200);
    }

    let response = AxumTestRequest::get("/api/admin/audit-events?limit=1")
        .header("Authorization", &operator.auth)
        .send(app.clone())
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json();
    let events = body["data"]["events"].as_array().expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["actor_email"], "audit-reader@example.com");
    let next = body["data"]["next_before"]
        .as_str()
        .expect("a full page has a cursor");

    let older = AxumTestRequest::get(&format!(
        "/api/admin/audit-events?limit=5&before={}",
        urlencoding::encode(next)
    ))
    .header("Authorization", &operator.auth)
    .send(app.clone())
    .await;
    let body: Value = older.json();
    let events = body["data"]["events"].as_array().expect("events");
    assert_eq!(
        events.len(),
        1,
        "the cursor pages past the first row: {body}"
    );
    assert!(body["data"]["next_before"].is_null());
    Ok(())
}

/// `pierre-cli config` validates its admin token inside the config handler,
/// not in a layer: the recorder resolves that token itself.
#[tokio::test]
#[serial]
async fn a_cli_config_write_is_recorded_as_its_operator() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let app = ProviderToolRouter::build_http_app(&resources);
    let operator = admin(&resources, "audit-config@example.com", UserRole::SuperAdmin).await?;
    let athlete_id = athlete(&resources, "config-target@example.com").await?;

    let response = AxumTestRequest::put(&format!("/api/admin/config?user_id={athlete_id}"))
        .header("authorization", &cli_token(&resources, &operator).await?)
        .json(&json!({
            "parameters": { "usage_quotas.max_active_conversations": 0 },
            "reason": "admin_audit_trail_test",
        }))
        .send(app.clone())
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());

    let rows = trail(&resources).await?;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].action, "PUT /api/admin/config");
    assert_eq!(rows[0].source, "cli");
    assert_eq!(
        rows[0].actor_user_id.as_deref(),
        Some(operator.user_id.to_string().as_str())
    );
    Ok(())
}
