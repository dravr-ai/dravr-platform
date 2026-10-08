// ABOUTME: End-to-end coach-access requests (carnet#738): a coach asks in one tap, a super-admin grants or declines
// ABOUTME: Pins the one-pending rule, the group checks, the notify ping, the super-admin gate and the group attach

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A coach without coach access who makes a group during onboarding gets it
//! coachless (ADR-018). These tests drive the way out: the coach's own route
//! opens a request, the console queue lists it, and a super-admin's grant
//! gives `manages_roster` and attaches the coach to that group — while the
//! request alone grants nothing, a plain admin cannot decide it, and a
//! decided request cannot be decided again.

mod common;
mod helpers;

use anyhow::Result;
use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use helpers::notify_capture::{capture_notify, named};
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::coach_access::CoachAccessRoutes;
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit, AdminRoutes};
use pierre_test_support::delegation::{create_coachless_group, create_group_agent};
use serde_json::{json, Value};
use serial_test::serial;
use std::sync::Arc;
use uuid::Uuid;

/// The admin context the composition root builds from a `ServerContext`.
fn admin_context(resources: &Arc<ServerContext>) -> AdminApiContext {
    AdminApiContext::new(AdminApiContextInit {
        database: resources.agent.database.clone(),
        repos: resources.common.repos.clone(),
        jwt_secret: resources.auth.admin_jwt_secret.to_string(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        harness_config_registry: resources.fitness.harness_config_registry.clone(),
        guardian_config_registry: resources.fitness.guardian_config_registry.clone(),
        prompt_registry: resources.mcp.prompt_registry.clone(),
        tool_description_registry: resources.mcp.tool_description_registry.clone(),
        evidence_registry: resources.mcp.evidence_registry.clone(),
        messaging_strings_registry: resources.mcp.messaging_strings_registry.clone(),
        cageux_config_registry: resources.fitness.cageux_config_registry.clone(),
        persona_contract_registry: resources.fitness.persona_contract_registry.clone(),
        training_catalogue_registry: resources.mcp.training_catalogue_registry.clone(),
        contremaitre_config: None,
        app_behavior: resources.common.config.app_behavior.clone(),
    })
}

/// The console mount: session auth in front of the admin handlers.
fn console(resources: &Arc<ServerContext>) -> axum::Router {
    AdminRoutes::cookie_admin_routes::<ServerContext>(admin_context(resources), resources)
}

/// The caller's own coach-access routes.
fn me(resources: &Arc<ServerContext>) -> axum::Router {
    CoachAccessRoutes::routes(Arc::clone(resources))
}

/// A signed-in user: the bearer header, the user id, its tenant.
struct Session {
    auth: String,
    user_id: Uuid,
    tenant_id: TenantId,
}

/// Create an active user with `role` owning a tenant, and sign it in.
async fn session(resources: &Arc<ServerContext>, email: &str, role: UserRole) -> Result<Session> {
    let mut user = User::new(email.to_owned(), "unused-hash".to_owned(), None);
    user.is_admin = role.is_admin_or_higher();
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
    Ok(Session {
        auth: format!("Bearer {token}"),
        user_id,
        tenant_id,
    })
}

/// A coachless group the session's user owns, as onboarding leaves it.
async fn coachless_group(resources: &Arc<ServerContext>, coach: &Session) -> Result<Uuid> {
    let repos = &resources.common.repos;
    let agent = create_group_agent(repos, coach.user_id, coach.tenant_id).await?;
    Ok(create_coachless_group(
        repos,
        coach.user_id,
        coach.tenant_id,
        &agent,
        "Tuesday Rides",
    )
    .await?)
}

async fn ask(resources: &Arc<ServerContext>, auth: &str, body: Value) -> (u16, Value) {
    let response = AxumTestRequest::post("/api/me/coach-access-request")
        .header("Authorization", auth)
        .json(&body)
        .send(me(resources))
        .await;
    let status = response.status();
    (status, response.json())
}

#[tokio::test]
#[serial]
async fn a_second_tap_returns_the_request_already_waiting() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let coach = session(&resources, "tap-twice@example.com", UserRole::User).await?;

    let none: Value = AxumTestRequest::get("/api/me/coach-access-request")
        .header("Authorization", &coach.auth)
        .send(me(&resources))
        .await
        .json();
    assert_eq!(none["request"], Value::Null, "{none}");

    let (events, _guard) = capture_notify();
    let (first_status, first) = ask(&resources, &coach.auth, json!({})).await;
    assert_eq!(first_status, 201, "{first}");
    assert_eq!(first["request"]["status"], "pending");
    assert_eq!(first["request"]["group_id"], Value::Null);

    let (second_status, second) = ask(&resources, &coach.auth, json!({})).await;
    assert_eq!(second_status, 200, "{second}");
    assert_eq!(second["request"]["id"], first["request"]["id"]);

    let pinged = named(&events, "coach_access.requested");
    assert_eq!(pinged.len(), 1, "one request, one ping: {pinged:?}");
    assert_eq!(pinged[0].field("user_id"), coach.user_id.to_string());
    assert_eq!(
        pinged[0].field("request_id"),
        first["request"]["id"].as_str().unwrap()
    );

    let latest: Value = AxumTestRequest::get("/api/me/coach-access-request")
        .header("Authorization", &coach.auth)
        .send(me(&resources))
        .await
        .json();
    assert_eq!(latest["request"]["id"], first["request"]["id"]);

    // The request alone grants nothing.
    let user = resources
        .common
        .repos
        .users
        .get_global(coach.user_id)
        .await?
        .expect("coach");
    assert!(!user.manages_roster);
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_super_admin_grant_makes_the_requester_the_coach_of_their_group() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let coach = session(&resources, "grant-coach@example.com", UserRole::User).await?;
    let group = coachless_group(&resources, &coach).await?;

    let (status, opened) = ask(&resources, &coach.auth, json!({ "group_id": group })).await;
    assert_eq!(status, 201, "{opened}");
    assert_eq!(opened["request"]["group_id"], group.to_string());
    let request_id = opened["request"]["id"].as_str().unwrap().to_owned();

    let operator = session(&resources, "grant-super@example.com", UserRole::SuperAdmin).await?;
    let queue: Value = AxumTestRequest::get("/api/admin/coach-access-requests")
        .header("Authorization", &operator.auth)
        .send(console(&resources))
        .await
        .json();
    let listed = queue["data"]["requests"].as_array().expect("requests");
    let row = listed
        .iter()
        .find(|r| r["id"] == request_id.as_str())
        .unwrap_or_else(|| panic!("the request must be queued: {queue}"));
    assert_eq!(row["email"], "grant-coach@example.com");
    assert_eq!(row["group_name"], "Tuesday Rides");

    let granted = AxumTestRequest::post(&format!(
        "/api/admin/coach-access-requests/{request_id}/grant"
    ))
    .header("Authorization", &operator.auth)
    .send(console(&resources))
    .await;
    assert_eq!(granted.status(), 200);
    let granted: Value = granted.json();
    assert_eq!(granted["data"]["request"]["status"], "granted", "{granted}");
    assert_eq!(granted["data"]["attached_group_id"], group.to_string());

    let user = repos.users.get_global(coach.user_id).await?.expect("coach");
    assert!(user.manages_roster, "the grant gives coach access");
    let grant = repos
        .users
        .manages_roster_operator_grant(coach.user_id)
        .await?
        .expect("an operator grant, which a TrainingPeaks disconnect never takes back");
    assert_eq!(grant.granted_by, Some(operator.user_id));

    let attached = repos
        .groups
        .get_group(&group.to_string(), coach.tenant_id)
        .await?
        .expect("group");
    assert_eq!(
        attached.coach_user_id,
        Some(coach.user_id),
        "no coach invite needed after a grant"
    );

    let mine: Value = AxumTestRequest::get("/api/me/coach-access-request")
        .header("Authorization", &coach.auth)
        .send(me(&resources))
        .await
        .json();
    assert_eq!(mine["request"]["status"], "granted");
    assert_eq!(mine["request"]["decided_by"], operator.user_id.to_string());

    // Decided once is decided.
    let again = AxumTestRequest::post(&format!(
        "/api/admin/coach-access-requests/{request_id}/decline"
    ))
    .header("Authorization", &operator.auth)
    .send(console(&resources))
    .await;
    assert_eq!(again.status(), 409);

    // Now that they hold coach access, there is nothing left to ask for.
    let (status, refused) = ask(&resources, &coach.auth, json!({})).await;
    assert_eq!(status, 400, "{refused}");
    Ok(())
}

#[tokio::test]
#[serial]
async fn only_a_super_admin_decides_and_a_decline_lets_the_coach_ask_again() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let coach = session(&resources, "decline-coach@example.com", UserRole::User).await?;
    let (_, opened) = ask(&resources, &coach.auth, json!({})).await;
    let request_id = opened["request"]["id"].as_str().unwrap().to_owned();

    let admin = session(&resources, "plain-admin@example.com", UserRole::Admin).await?;
    for path in [
        "/api/admin/coach-access-requests".to_owned(),
        format!("/api/admin/coach-access-requests/{request_id}/grant"),
    ] {
        let request = if path.ends_with("/grant") {
            AxumTestRequest::post(&path)
        } else {
            AxumTestRequest::get(&path)
        };
        let refused = request
            .header("Authorization", &admin.auth)
            .send(console(&resources))
            .await;
        assert_eq!(refused.status(), 403, "{path} is super-admin only");
    }
    let user = resources
        .common
        .repos
        .users
        .get_global(coach.user_id)
        .await?
        .expect("coach");
    assert!(!user.manages_roster, "nothing may have been written");

    let operator = session(
        &resources,
        "decline-super@example.com",
        UserRole::SuperAdmin,
    )
    .await?;
    let declined: Value = AxumTestRequest::post(&format!(
        "/api/admin/coach-access-requests/{request_id}/decline"
    ))
    .header("Authorization", &operator.auth)
    .send(console(&resources))
    .await
    .json();
    assert_eq!(
        declined["data"]["request"]["status"], "declined",
        "{declined}"
    );

    let history: Value = AxumTestRequest::get("/api/admin/coach-access-requests?status=declined")
        .header("Authorization", &operator.auth)
        .send(console(&resources))
        .await
        .json();
    assert!(history["data"]["requests"]
        .as_array()
        .expect("requests")
        .iter()
        .any(|r| r["id"] == request_id.as_str()));

    let (status, reopened) = ask(&resources, &coach.auth, json!({})).await;
    assert_eq!(status, 201, "a declined coach may ask again: {reopened}");
    assert_ne!(reopened["request"]["id"], opened["request"]["id"]);
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_coach_can_only_name_their_own_coachless_group() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let owner = session(&resources, "group-owner@example.com", UserRole::User).await?;
    let group = coachless_group(&resources, &owner).await?;

    // Someone else's group is refused, whichever tenant names it.
    let stranger = session(&resources, "stranger@example.com", UserRole::User).await?;
    let (status, body) = ask(&resources, &stranger.auth, json!({ "group_id": group })).await;
    assert_eq!(status, 403, "{body}");

    // A group that already has a coach has nothing to ask for.
    let repos = &resources.common.repos;
    let coach = session(&resources, "existing-coach@example.com", UserRole::User).await?;
    repos
        .groups
        .set_group_coach_user(&group.to_string(), Some(coach.user_id), owner.tenant_id)
        .await?;
    let (status, body) = ask(&resources, &owner.auth, json!({ "group_id": group })).await;
    assert_eq!(status, 400, "{body}");

    let (status, body) = ask(&resources, &owner.auth, json!({ "group_id": "not-a-uuid" })).await;
    assert_eq!(status, 400, "{body}");
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_grant_leaves_a_group_that_found_a_coach_meanwhile_alone() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let coach = session(&resources, "late-coach@example.com", UserRole::User).await?;
    let group = coachless_group(&resources, &coach).await?;
    let (_, opened) = ask(&resources, &coach.auth, json!({ "group_id": group })).await;
    let request_id = opened["request"]["id"].as_str().unwrap().to_owned();

    let other = session(&resources, "other-coach@example.com", UserRole::User).await?;
    repos
        .groups
        .set_group_coach_user(&group.to_string(), Some(other.user_id), coach.tenant_id)
        .await?;

    let operator = session(&resources, "late-super@example.com", UserRole::SuperAdmin).await?;
    let granted: Value = AxumTestRequest::post(&format!(
        "/api/admin/coach-access-requests/{request_id}/grant"
    ))
    .header("Authorization", &operator.auth)
    .send(console(&resources))
    .await
    .json();
    assert_eq!(granted["data"]["request"]["status"], "granted", "{granted}");
    assert_eq!(granted["data"]["attached_group_id"], Value::Null);

    let kept = repos
        .groups
        .get_group(&group.to_string(), coach.tenant_id)
        .await?
        .expect("group");
    assert_eq!(kept.coach_user_id, Some(other.user_id));
    Ok(())
}

#[tokio::test]
#[serial]
async fn deleting_the_requester_takes_their_requests_along() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let coach = session(&resources, "deleted-coach@example.com", UserRole::User).await?;
    let (_, opened) = ask(&resources, &coach.auth, json!({})).await;
    let request_id: Uuid = opened["request"]["id"].as_str().unwrap().parse()?;

    let blockers = repos.users.deletion_blockers(coach.user_id).await?;
    assert!(
        blockers.is_empty(),
        "a coach access request must not block deleting the account: {blockers:?}"
    );
    assert!(repos.coach_access_requests.get(request_id).await?.is_some());

    repos.users.delete(coach.user_id, None).await?;
    assert!(
        repos.coach_access_requests.get(request_id).await?.is_none(),
        "the request must go with the account"
    );
    Ok(())
}
