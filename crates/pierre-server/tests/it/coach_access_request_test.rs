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

use crate::common;
use crate::helpers;
use pierre_test_support::account::delete_account;

use anyhow::Result;
use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use helpers::notify_capture::{capture_notify, named, only};
use pierre_contremaitre::messaging_strings::{
    KEY_COACH_ACCESS_DECLINED_REASON_BODY, KEY_COACH_ACCESS_ROOM_LINE_ATHLETE,
    KEY_COACH_ACCESS_ROOM_LINE_COACH,
};
use pierre_core::errors::ErrorCode;
use pierre_core::models::roster_display_name;
use pierre_core::models::{
    CoachAccessRequest, CoachAccessStatus, Tenant, TenantId, User, UserStatus,
};
use pierre_core::permissions::UserRole;
use pierre_core::untrusted;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_email::templates::CoachAccessOrigin;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::coach_access::{coach_access_notices, CoachAccessRoutes};
use pierre_notifications::models::Notification;
use pierre_notifications::TenantId as NoticeTenantId;
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit, AdminRoutes};
use pierre_services::analytics::resolve_user_email;
use pierre_services::coach_access::{self, RequestOutcome};
use pierre_services::locale::user_locale;
use pierre_test_support::delegation::{
    add_group_member, create_coached_group, create_coachless_group, create_group_agent,
};
use serde_json::{json, Value};
use serial_test::serial;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
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
    assert_eq!(pinged[0].field("origin"), "asked");
    for absent in ["group_id", "group_name", "coached_active_athletes"] {
        assert!(
            !pinged[0].fields.contains_key(absent),
            "{absent}: a request asked from no group renders none: {pinged:?}"
        );
    }
    assert_eq!(
        resolve_user_email(&coach.user_id.to_string()).as_deref(),
        Some("tap-twice@example.com"),
        "the notice names the requester whichever pod they signed in on"
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

    let (events, _guard) = capture_notify();
    let (status, opened) = ask(&resources, &coach.auth, json!({ "group_id": group })).await;
    assert_eq!(status, 201, "{opened}");
    assert_eq!(opened["request"]["group_id"], group.to_string());
    let request_id = opened["request"]["id"].as_str().unwrap().to_owned();

    // The notice names the group the coach asked from, by id and by name.
    let requested = only(&events, "coach_access.requested");
    assert_eq!(requested.field("group_id"), group.to_string());
    assert_eq!(requested.field("group_name"), "Tuesday Rides");
    assert_eq!(requested.field("origin"), "asked");

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

    // The room that saw the request open sees it decided, and where the
    // coach now coaches.
    let decided = only(&events, "coach_access.granted");
    assert_eq!(decided.field("request_id"), request_id);
    assert_eq!(decided.field("user_id"), coach.user_id.to_string());
    assert_eq!(decided.field("group_id"), group.to_string());

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
    assert!(
        named(&events, "coach_access.declined").is_empty(),
        "a refused decision announces nothing"
    );

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
    let (events, _guard) = capture_notify();
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
    let decided = only(&events, "coach_access.declined");
    assert_eq!(decided.field("request_id"), request_id);
    assert_eq!(decided.field("user_id"), coach.user_id.to_string());

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
async fn a_decision_names_a_requester_this_pod_never_saw() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let operator = session(&resources, "unseen-super@example.com", UserRole::SuperAdmin).await?;

    for (email, decision) in [
        ("unseen-granted@example.com", "grant"),
        ("unseen-declined@example.com", "decline"),
    ] {
        let coach = session(&resources, email, UserRole::User).await?;
        // Opened straight in the store, as when another pod announced it:
        // nothing on this one cached who asked.
        let request = CoachAccessRequest::pending(coach.user_id, None, chrono::Utc::now());
        assert!(repos.coach_access_requests.open(&request).await?);
        assert_eq!(resolve_user_email(&coach.user_id.to_string()), None);

        let decided = AxumTestRequest::post(&format!(
            "/api/admin/coach-access-requests/{}/{decision}",
            request.id
        ))
        .header("Authorization", &operator.auth)
        .send(console(&resources))
        .await;
        assert_eq!(decided.status(), 200, "{decision}");
        assert_eq!(
            resolve_user_email(&coach.user_id.to_string()).as_deref(),
            Some(email),
            "the {decision} notice names whose request it was"
        );
    }
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

    delete_account(repos, coach.user_id, None).await?;
    assert!(
        repos.coach_access_requests.get(request_id).await?.is_none(),
        "the request must go with the account"
    );
    Ok(())
}

/// A grant racing a decline on the same request (two operators, or one
/// double-submit): whichever claims the decision first decides it, and the
/// access state follows the row — never a `declined` row over a user who
/// holds coach access and coaches the group (carnet#902).
#[tokio::test]
#[serial]
async fn a_grant_racing_a_decline_leaves_the_row_and_the_access_in_agreement() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let operator = session(&resources, "race-super@example.com", UserRole::SuperAdmin).await?;

    for round in 0..30 {
        let coach = session(
            &resources,
            &format!("race-coach-{round}@example.com"),
            UserRole::User,
        )
        .await?;
        let group = coachless_group(&resources, &coach).await?;
        let (status, opened) = ask(&resources, &coach.auth, json!({ "group_id": group })).await;
        assert_eq!(status, 201, "{opened}");
        let request_id: Uuid = opened["request"]["id"].as_str().unwrap().parse()?;

        let (granted, declined) = tokio::join!(
            coach_access::grant(repos, request_id, Some(operator.user_id), None),
            coach_access::decline(repos, request_id, Some(operator.user_id), None, None),
        );
        assert!(
            granted.is_ok() != declined.is_ok(),
            "exactly one decision wins (round {round}): grant {granted:?}, decline {declined:?}"
        );

        let row = repos
            .coach_access_requests
            .get(request_id)
            .await?
            .expect("request");
        let is_granted = row.status == CoachAccessStatus::Granted;
        assert_eq!(is_granted, granted.is_ok(), "round {round}: {row:?}");
        let user = repos.users.get_global(coach.user_id).await?.expect("coach");
        assert_eq!(
            user.manages_roster, is_granted,
            "round {round}: coach access must follow the row ({:?})",
            row.status
        );
        let attached = repos
            .groups
            .get_group(&group.to_string(), coach.tenant_id)
            .await?
            .expect("group");
        assert_eq!(
            attached.coach_user_id == Some(coach.user_id),
            is_granted,
            "round {round}: the group attachment must follow the row ({:?})",
            row.status
        );
    }
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_grant_after_a_decline_is_refused_and_gives_nothing() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let coach = session(&resources, "late-grant-coach@example.com", UserRole::User).await?;
    let group = coachless_group(&resources, &coach).await?;
    let (_, opened) = ask(&resources, &coach.auth, json!({ "group_id": group })).await;
    let request_id: Uuid = opened["request"]["id"].as_str().unwrap().parse()?;
    let operator = session(
        &resources,
        "late-grant-super@example.com",
        UserRole::SuperAdmin,
    )
    .await?;

    coach_access::decline(repos, request_id, Some(operator.user_id), None, None).await?;
    let error = coach_access::grant(repos, request_id, Some(operator.user_id), None)
        .await
        .expect_err("a decided request cannot be granted");
    assert_eq!(error.code, ErrorCode::ResourceAlreadyExists);

    let user = repos.users.get_global(coach.user_id).await?.expect("coach");
    assert!(!user.manages_roster);
    let kept = repos
        .groups
        .get_group(&group.to_string(), coach.tenant_id)
        .await?
        .expect("group");
    assert_eq!(kept.coach_user_id, None);
    Ok(())
}

/// Make every later write to `coaching_groups` fail, so a grant fails at the
/// group attachment, after it gave coach access.
async fn refuse_group_writes(resources: &ServerContext) {
    const NAME: &str = "fault_coaching_groups_refused";
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(&format!(
                "CREATE TRIGGER {NAME} BEFORE UPDATE ON coaching_groups \
                 BEGIN SELECT RAISE(ABORT, 'group write refused'); END"
            ))
            .execute(db.pool())
            .await
            .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(&format!(
                "CREATE FUNCTION {NAME}() RETURNS trigger LANGUAGE plpgsql \
                 AS $$ BEGIN RAISE EXCEPTION 'group write refused'; END $$"
            ))
            .execute(db.pool())
            .await
            .unwrap();
            sqlx::query(&format!(
                "CREATE TRIGGER {NAME} BEFORE UPDATE ON coaching_groups \
                 FOR EACH ROW EXECUTE FUNCTION {NAME}()"
            ))
            .execute(db.pool())
            .await
            .unwrap();
        }
    }
}

/// carnet#902: a grant that fails at the group attachment is undone, but the
/// undo puts coach access back to what it was before the grant. A requester
/// with no access before loses the access the grant gave; one an operator
/// granted by hand meanwhile keeps it. Either way the request is pending again.
#[tokio::test]
#[serial]
async fn a_failed_grant_restores_the_coach_access_it_found() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let operator = session(&resources, "undo-super@example.com", UserRole::SuperAdmin).await?;

    let fresh = session(&resources, "undo-fresh@example.com", UserRole::User).await?;
    let fresh_group = coachless_group(&resources, &fresh).await?;
    let (_, opened) = ask(&resources, &fresh.auth, json!({ "group_id": fresh_group })).await;
    let fresh_request: Uuid = opened["request"]["id"].as_str().unwrap().parse()?;

    let granted_by_hand = session(&resources, "undo-hand@example.com", UserRole::User).await?;
    let hand_group = coachless_group(&resources, &granted_by_hand).await?;
    let (_, opened) = ask(
        &resources,
        &granted_by_hand.auth,
        json!({ "group_id": hand_group }),
    )
    .await;
    let hand_request: Uuid = opened["request"]["id"].as_str().unwrap().parse()?;
    repos
        .users
        .set_manages_roster_by_operator(granted_by_hand.user_id, true, Some(operator.user_id))
        .await?;

    refuse_group_writes(&resources).await;
    for request in [fresh_request, hand_request] {
        coach_access::grant(repos, request, Some(operator.user_id), None)
            .await
            .expect_err("the group attachment is refused");
        let reopened = repos
            .coach_access_requests
            .get(request)
            .await?
            .expect("request");
        assert_eq!(reopened.status, CoachAccessStatus::Pending);
    }

    let fresh_user = repos.users.get_global(fresh.user_id).await?.expect("user");
    assert!(
        !fresh_user.manages_roster,
        "the grant's own access is taken back"
    );
    let hand_user = repos
        .users
        .get_global(granted_by_hand.user_id)
        .await?
        .expect("user");
    assert!(
        hand_user.manages_roster,
        "an operator's earlier grant survives the undo"
    );
    Ok(())
}

// ── carnet#1521 / #1522: the coach is told, every coachless group, withdraw ──

/// The console mount with the coach-access notices the composition root
/// wires, so a decision reaches the coach as it does in production.
fn console_telling_the_coach(resources: &Arc<ServerContext>) -> axum::Router {
    let mut context = admin_context(resources);
    context.coach_access_notices = Some(Arc::new(coach_access_notices(resources)));
    AdminRoutes::cookie_admin_routes::<ServerContext>(context, resources)
}

/// `who`'s copy of `group`'s chat in their own tenant, as the app files one.
async fn group_copy(resources: &Arc<ServerContext>, who: &Session, group: Uuid) -> Result<String> {
    let copy = resources
        .common
        .repos
        .chat
        .create_conversation(
            &who.user_id.to_string(),
            who.tenant_id,
            "Group room",
            "test-model",
            None,
            Some(&group.to_string()),
        )
        .await?;
    Ok(copy.id)
}

/// The words of every row in `who`'s conversation `conversation`.
async fn rows_of(resources: &Arc<ServerContext>, who: &Session, conversation: &str) -> Vec<String> {
    resources
        .common
        .repos
        .chat
        .get_messages(conversation, &who.user_id.to_string(), who.tenant_id)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.content)
        .collect()
}

/// The room line `reader` reads for a grant to the coach behind
/// `coach_email`, as the catalogue renders it in the reader's language.
async fn room_line(
    resources: &Arc<ServerContext>,
    reader: &Session,
    coach_email: &str,
    key: &str,
) -> String {
    let user = resources
        .common
        .repos
        .users
        .get_global(reader.user_id)
        .await
        .unwrap()
        .expect("reader");
    let name = untrusted::markdown_literal(&untrusted::display_line(
        &roster_display_name(None, coach_email),
        untrusted::ROSTER_NAME_MAX_CHARS,
    ));
    resources
        .mcp
        .messaging_strings_registry
        .render(key, &user_locale(Some(&user)), &[&name])
}

/// `who`'s notices of kind `wire` in their own tenant, once one has landed
/// or the dispatch had its time.
async fn notices_of(
    resources: &Arc<ServerContext>,
    who: &Session,
    wire: &str,
) -> Vec<Notification> {
    let service = resources
        .common
        .notification_service
        .as_ref()
        .expect("the server boots with its notification service");
    let mut rows = Vec::new();
    for _ in 0..40 {
        let (all, _, _) = service
            .list_notifications(
                who.user_id,
                NoticeTenantId(who.tenant_id.as_uuid()),
                50,
                0,
                None,
                false,
            )
            .await
            .unwrap();
        rows = all
            .into_iter()
            .filter(|row| row.notification_type == wire)
            .collect();
        if !rows.is_empty() {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    rows
}

#[tokio::test]
#[serial]
async fn one_grant_coaches_every_coachless_group_the_coach_owns_and_says_so_in_each_room(
) -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let coach_email = "every-group-coach@example.com";
    let coach = session(&resources, coach_email, UserRole::User).await?;
    let athlete = session(
        &resources,
        "every-group-athlete@example.com",
        UserRole::User,
    )
    .await?;

    // Two coachless groups, and a third the coach owns that already has a
    // coach of its own, which the grant must leave alone.
    let rides = coachless_group(&resources, &coach).await?;
    let agent = create_group_agent(repos, coach.user_id, coach.tenant_id).await?;
    let marathon =
        create_coachless_group(repos, coach.user_id, coach.tenant_id, &agent, "Marathon").await?;
    let other = session(&resources, "every-group-other@example.com", UserRole::User).await?;
    let coached_elsewhere = create_coached_group(
        repos,
        coach.user_id,
        coach.tenant_id,
        &agent,
        "Taken",
        other.user_id,
    )
    .await?;
    add_group_member(repos, rides, athlete.user_id, athlete.tenant_id).await?;
    let coach_rides = group_copy(&resources, &coach, rides).await?;
    let coach_marathon = group_copy(&resources, &coach, marathon).await?;
    let athlete_rides = group_copy(&resources, &athlete, rides).await?;

    // The coach asked from the marathon group; the request names it.
    let (status, opened) = ask(&resources, &coach.auth, json!({ "group_id": marathon })).await;
    assert_eq!(status, 201, "{opened}");
    let request_id = opened["request"]["id"].as_str().unwrap().to_owned();

    let operator = session(
        &resources,
        "every-group-super@example.com",
        UserRole::SuperAdmin,
    )
    .await?;
    let (events, _guard) = capture_notify();
    let granted = AxumTestRequest::post(&format!(
        "/api/admin/coach-access-requests/{request_id}/grant"
    ))
    .header("Authorization", &operator.auth)
    .send(console_telling_the_coach(&resources))
    .await;
    assert_eq!(granted.status(), 200);
    let granted: Value = granted.json();
    assert_eq!(
        granted["data"]["attached_group_ids"],
        json!([marathon.to_string(), rides.to_string()]),
        "the named group leads, and every coachless group follows: {granted}"
    );
    assert_eq!(granted["data"]["attached_group_id"], marathon.to_string());
    assert_eq!(
        only(&events, "coach_access.granted").field("group_id"),
        marathon.to_string()
    );

    for group in [rides, marathon] {
        let attached = repos
            .groups
            .get_group(&group.to_string(), coach.tenant_id)
            .await?
            .expect("group");
        assert_eq!(attached.coach_user_id, Some(coach.user_id));
    }
    let kept = repos
        .groups
        .get_group(&coached_elsewhere.to_string(), coach.tenant_id)
        .await?
        .expect("group");
    assert_eq!(
        kept.coach_user_id,
        Some(other.user_id),
        "a group with a coach keeps it"
    );

    // Every room the grant attached says so, in each copy of its chat.
    let coach_line = room_line(
        &resources,
        &coach,
        coach_email,
        KEY_COACH_ACCESS_ROOM_LINE_COACH,
    )
    .await;
    let athlete_line = room_line(
        &resources,
        &athlete,
        coach_email,
        KEY_COACH_ACCESS_ROOM_LINE_ATHLETE,
    )
    .await;
    assert!(!coach_line.trim().is_empty() && !athlete_line.trim().is_empty());
    assert_ne!(
        coach_line, athlete_line,
        "the athlete's line says what the coach sees"
    );
    for copy in [&coach_rides, &coach_marathon] {
        assert_eq!(
            rows_of(&resources, &coach, copy).await,
            vec![coach_line.clone()]
        );
    }
    assert_eq!(
        rows_of(&resources, &athlete, &athlete_rides).await,
        vec![athlete_line]
    );

    // And the coach is told in the app, with the way to their coach Home.
    let notices = notices_of(&resources, &coach, "coach_access_granted").await;
    assert_eq!(notices.len(), 1, "{notices:?}");
    let notice = &notices[0];
    assert_eq!(notice.data.as_ref().unwrap()["screen"], "activities");
    let actions = notice.actions.as_ref().expect("the coach Home button");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].id, "open_coach_home");
    assert!(
        notices_of(&resources, &athlete, "coach_access_granted")
            .await
            .is_empty(),
        "only the requester is told"
    );
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_decline_carries_its_reason_to_the_request_the_notice_and_the_queue() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let coach = session(&resources, "reason-coach@example.com", UserRole::User).await?;
    let operator = session(&resources, "reason-super@example.com", UserRole::SuperAdmin).await?;

    let (_, opened) = ask(&resources, &coach.auth, json!({})).await;
    let request_id = opened["request"]["id"].as_str().unwrap().to_owned();
    let declined: Value = AxumTestRequest::post(&format!(
        "/api/admin/coach-access-requests/{request_id}/decline"
    ))
    .header("Authorization", &operator.auth)
    .json(&json!({ "reason": "  No coaching account we could verify.  " }))
    .send(console_telling_the_coach(&resources))
    .await
    .json();
    let reason = "No coaching account we could verify.";
    assert_eq!(
        declined["data"]["request"]["decline_reason"], reason,
        "{declined}"
    );

    // The coach reads it back where they asked, and the queue keeps it.
    let mine: Value = AxumTestRequest::get("/api/me/coach-access-request")
        .header("Authorization", &coach.auth)
        .send(me(&resources))
        .await
        .json();
    assert_eq!(mine["request"]["status"], "declined");
    assert_eq!(mine["request"]["decline_reason"], reason);
    let history: Value = AxumTestRequest::get("/api/admin/coach-access-requests?status=declined")
        .header("Authorization", &operator.auth)
        .send(console(&resources))
        .await
        .json();
    let row = history["data"]["requests"]
        .as_array()
        .expect("requests")
        .iter()
        .find(|r| r["id"] == request_id.as_str())
        .cloned()
        .expect("the declined request is listed");
    assert_eq!(row["decline_reason"], reason);

    // The notice carries it inside the sentence of the coach's language.
    let user = resources
        .common
        .repos
        .users
        .get_global(coach.user_id)
        .await?
        .expect("coach");
    let notices = notices_of(&resources, &coach, "coach_access_declined").await;
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert_eq!(
        notices[0].body,
        resources.mcp.messaging_strings_registry.render(
            KEY_COACH_ACCESS_DECLINED_REASON_BODY,
            &user_locale(Some(&user)),
            &[reason]
        )
    );
    assert!(notices[0].body.contains(reason));

    // A decline given a blank reason stores none.
    let (_, again) = ask(&resources, &coach.auth, json!({})).await;
    let again_id = again["request"]["id"].as_str().unwrap().to_owned();
    let plain: Value = AxumTestRequest::post(&format!(
        "/api/admin/coach-access-requests/{again_id}/decline"
    ))
    .header("Authorization", &operator.auth)
    .json(&json!({ "reason": "   " }))
    .send(console_telling_the_coach(&resources))
    .await
    .json();
    assert_eq!(
        plain["data"]["request"]["decline_reason"],
        Value::Null,
        "{plain}"
    );
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_coach_withdraws_a_waiting_request_and_may_ask_again_at_once() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let coach = session(&resources, "withdraw-coach@example.com", UserRole::User).await?;

    // Nothing waiting, nothing to withdraw.
    let nothing = AxumTestRequest::delete("/api/me/coach-access-request")
        .header("Authorization", &coach.auth)
        .send(me(&resources))
        .await;
    assert_eq!(nothing.status(), 404);

    let (_, opened) = ask(&resources, &coach.auth, json!({})).await;
    let first_id = opened["request"]["id"].as_str().unwrap().to_owned();
    let (events, _guard) = capture_notify();
    let withdrawn = AxumTestRequest::delete("/api/me/coach-access-request")
        .header("Authorization", &coach.auth)
        .send(me(&resources))
        .await;
    assert_eq!(withdrawn.status(), 200);
    let withdrawn: Value = withdrawn.json();
    assert_eq!(withdrawn["request"], Value::Null, "{withdrawn}");
    let cancelled = only(&events, "coach_access.cancelled");
    assert_eq!(cancelled.field("request_id"), first_id);
    assert_eq!(cancelled.field("user_id"), coach.user_id.to_string());

    let mine: Value = AxumTestRequest::get("/api/me/coach-access-request")
        .header("Authorization", &coach.auth)
        .send(me(&resources))
        .await
        .json();
    assert_eq!(mine["request"], Value::Null);

    // Asking again opens a new request at once; the withdrawn one is no
    // longer there for a super-admin to decide.
    let (status, reopened) = ask(&resources, &coach.auth, json!({})).await;
    assert_eq!(status, 201, "{reopened}");
    assert_ne!(reopened["request"]["id"], first_id.as_str());
    let operator = session(
        &resources,
        "withdraw-super@example.com",
        UserRole::SuperAdmin,
    )
    .await?;
    let gone = AxumTestRequest::post(&format!(
        "/api/admin/coach-access-requests/{first_id}/grant"
    ))
    .header("Authorization", &operator.auth)
    .send(console(&resources))
    .await;
    assert_eq!(gone.status(), 404);
    Ok(())
}

#[tokio::test]
#[serial]
async fn an_owner_who_never_said_they_coach_may_ask_for_their_coachless_group() -> Result<()> {
    let resources = create_test_server_resources().await?;
    // No onboarding answer at all: asking is open to the owner of a coachless
    // group without coach access, never gated on "I coach others".
    let owner = session(&resources, "never-answered@example.com", UserRole::User).await?;
    let group = coachless_group(&resources, &owner).await?;
    let (code, opened) = ask(&resources, &owner.auth, json!({ "group_id": group })).await;
    assert_eq!(code, 201, "{opened}");
    assert_eq!(opened["request"]["group_id"], group.to_string());
    Ok(())
}

/// The coach's current request, as the app reads it.
async fn current(resources: &Arc<ServerContext>, coach: &Session) -> Value {
    AxumTestRequest::get("/api/me/coach-access-request")
        .header("Authorization", &coach.auth)
        .send(me(resources))
        .await
        .json::<Value>()["request"]
        .clone()
}

/// Asking again replaces an earlier decline (carnet#1522): a coach who was
/// declined, asked again and withdrew that ask reads the plain ask — no
/// request — after a reload too, while the console keeps the decline and its
/// reason. A decline nobody asked past still reads as the decline.
#[tokio::test]
#[serial]
async fn asking_again_replaces_a_decline_so_a_withdrawn_ask_reads_the_plain_ask() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let coach = session(&resources, "replaced-coach@example.com", UserRole::User).await?;
    let operator = session(
        &resources,
        "replaced-super@example.com",
        UserRole::SuperAdmin,
    )
    .await?;

    let (_, first) = ask(&resources, &coach.auth, json!({})).await;
    let first_id = first["request"]["id"].as_str().unwrap().to_owned();
    let declined = AxumTestRequest::post(&format!(
        "/api/admin/coach-access-requests/{first_id}/decline"
    ))
    .header("Authorization", &operator.auth)
    .json(&json!({ "reason": "No coaching account we could verify." }))
    .send(console(&resources))
    .await;
    assert_eq!(declined.status(), 200);

    // A decline with no new ask still reads as the decline, with its reason.
    let still = current(&resources, &coach).await;
    assert_eq!(still["id"], first_id.as_str());
    assert_eq!(still["status"], "declined");
    assert_eq!(
        still["decline_reason"],
        "No coaching account we could verify."
    );

    // Asking again replaces it; withdrawing that ask leaves the plain ask.
    let (status, second) = ask(&resources, &coach.auth, json!({})).await;
    assert_eq!(status, 201, "{second}");
    let withdrawn: Value = AxumTestRequest::delete("/api/me/coach-access-request")
        .header("Authorization", &coach.auth)
        .send(me(&resources))
        .await
        .json();
    assert_eq!(withdrawn["request"], Value::Null, "{withdrawn}");
    assert_eq!(
        current(&resources, &coach).await,
        Value::Null,
        "a reload reads the plain ask, not the old decline"
    );

    // The console keeps the decline and its reason; the row says what replaced it.
    let history: Value = AxumTestRequest::get("/api/admin/coach-access-requests?status=declined")
        .header("Authorization", &operator.auth)
        .send(console(&resources))
        .await
        .json();
    let kept = history["data"]["requests"]
        .as_array()
        .expect("requests")
        .iter()
        .find(|r| r["id"] == first_id.as_str())
        .cloned()
        .expect("the declined request stays in the queue's history");
    assert_eq!(
        kept["decline_reason"],
        "No coaching account we could verify."
    );
    let row = repos
        .coach_access_requests
        .get(first_id.parse()?)
        .await?
        .expect("declined row");
    assert!(row.superseded_at.is_some(), "{row:?}");

    // And the coach may ask once more, as from scratch.
    let (status, third) = ask(&resources, &coach.auth, json!({})).await;
    assert_eq!(status, 201, "{third}");
    assert_eq!(current(&resources, &coach).await["status"], "pending");
    Ok(())
}

/// A second tenant owned by the session's user, joined after their first,
/// so their home tenant stays the first.
async fn second_tenant(resources: &Arc<ServerContext>, owner: &Session) -> Result<TenantId> {
    let tenant_id = TenantId::generate();
    resources
        .common
        .repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: "Second tenant".to_owned(),
            slug: format!("second-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: owner.user_id,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .await?;
    Ok(tenant_id)
}

/// The coach a group has now, read back from the store.
async fn coach_of(resources: &Arc<ServerContext>, group: Uuid, tenant: TenantId) -> Option<Uuid> {
    resources
        .common
        .repos
        .groups
        .get_group(&group.to_string(), tenant)
        .await
        .unwrap()
        .expect("group")
        .coach_user_id
}

/// A grant stays in the coach's own tenant (Phil, 2026-11-13): the tenant of
/// the group the request names, else the coach's home tenant. A coachless
/// group the coach owns in another tenant is left coachless.
#[tokio::test]
#[serial]
async fn a_grant_attaches_only_the_coachless_groups_in_the_coachs_own_tenant() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let repos = &resources.common.repos;
    let operator = session(
        &resources,
        "tenant-scope-super@example.com",
        UserRole::SuperAdmin,
    )
    .await?;

    // A request naming no group: the coach's home tenant.
    let coach = session(&resources, "tenant-scope-home@example.com", UserRole::User).await?;
    let elsewhere = second_tenant(&resources, &coach).await?;
    let home_group = coachless_group(&resources, &coach).await?;
    let other_agent = create_group_agent(repos, coach.user_id, elsewhere).await?;
    let other_group =
        create_coachless_group(repos, coach.user_id, elsewhere, &other_agent, "Elsewhere").await?;
    let (_, opened) = ask(&resources, &coach.auth, json!({})).await;
    let request: Uuid = opened["request"]["id"].as_str().unwrap().parse()?;
    let outcome = coach_access::grant(repos, request, Some(operator.user_id), None).await?;
    assert_eq!(
        outcome.attached_groups,
        vec![home_group],
        "a request naming no group attaches the home tenant's coachless group only"
    );
    assert_eq!(
        coach_of(&resources, home_group, coach.tenant_id).await,
        Some(coach.user_id),
        "the home tenant's group reads the requester as its coach"
    );
    assert_eq!(
        coach_of(&resources, other_group, elsewhere).await,
        None,
        "a coachless group in another tenant is left coachless"
    );

    // A request naming a group: that group's tenant, not the home one.
    let named = session(&resources, "tenant-scope-named@example.com", UserRole::User).await?;
    let named_tenant = second_tenant(&resources, &named).await?;
    let home_only = coachless_group(&resources, &named).await?;
    let named_agent = create_group_agent(repos, named.user_id, named_tenant).await?;
    let asked_from =
        create_coachless_group(repos, named.user_id, named_tenant, &named_agent, "Asked").await?;
    let RequestOutcome::Opened(opened) = coach_access::request_access(
        repos,
        named.user_id,
        Some((asked_from, named_tenant)),
        CoachAccessOrigin::Asked,
    )
    .await?
    else {
        panic!("a new request opens");
    };
    let outcome = coach_access::grant(repos, opened.id, Some(operator.user_id), None).await?;
    assert_eq!(
        outcome.attached_groups,
        vec![asked_from],
        "a request naming a group attaches that group's tenant's coachless group only"
    );
    assert_eq!(
        coach_of(&resources, asked_from, named_tenant).await,
        Some(named.user_id),
        "the named group reads the requester as its coach"
    );
    assert_eq!(
        coach_of(&resources, home_only, named.tenant_id).await,
        None,
        "the home tenant's group is not the named group's tenant"
    );
    Ok(())
}
