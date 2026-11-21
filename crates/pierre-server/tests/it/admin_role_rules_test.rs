// ABOUTME: The rules that replaced the super-admin tier, driven over HTTP: who impersonates whom and who manages admins
// ABOUTME: Pins what stays refused with one admin role (carnet#1567), and that an admin's credentials end with its standing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One admin role holds every power, so the rules a tier used to enforce are
//! stated between a user and an admin, and between one admin and another:
//!
//! - an admin impersonates a user, and never another admin;
//! - an admin promotes a user and demotes another admin, and never itself;
//!   it suspends another admin, and never itself;
//! - a user does none of it, and nothing a refused request asked for is
//!   written;
//! - an admin who is demoted, suspended or deleted takes with it the device
//!   logins it approved and the impersonations it had open, in the same
//!   write, and a later promotion or approval brings none of them back; a
//!   deleted admin's logins stay as rows that name no one and act as no
//!   account, refused for good;
//! - a user who becomes an admin stops being impersonated: the sessions open
//!   on the account end with the promotion;
//! - a service token without the super-admin flag suspends a user, approves
//!   one and is handed one's password reset, and none of it for an admin;
//!   bound to a tenant, it suspends and approves only a user held within it.
//!
//! The repository's own refusals (the last active admin is neither demoted,
//! suspended nor deleted) are pinned in `database_users_test` and
//! `admin_user_removal_test`; this file drives the routes in front of them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::admin_user_removal_test::seed_user;
use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::Router;
use chrono::Utc;
use common::{acting_on, create_test_server_resources, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::admin::models::{
    AdminPermission, CreateAdminTokenRequest, DELETED_ACCOUNT_ADMIN, DELETED_ACCOUNT_LABEL,
    DEVICE_CLI_DELETED_ACCOUNT_SERVICE,
};
use pierre_core::errors::ErrorCode;
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::{AuditEventFilter, AuditEventRecord};
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_middleware::cookie_admin_token;
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit, AdminRoutes, ImpersonationRoutes};
use pierre_routes_auth::AuthRoutes;
use pierre_routes_web_admin::WebAdminRoutes;
use pierre_services::admin_ops::{self, AccountCaller, ADMIN_CREDENTIALS_REVOKED};
use secrecy::ExposeSecret;
use serde_json::{json, Value};
use serial_test::serial;
use uuid::Uuid;

/// The password every account of this file signs in with.
const PASSWORD: &str = "Standing-1574!";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// An active account with `role`, and the bearer header of its session.
async fn account(resources: &Arc<ServerContext>, email: &str, role: UserRole) -> (User, String) {
    let password_hash = bcrypt::hash(PASSWORD, 4).unwrap();
    let mut user = User::new(email.to_owned(), password_hash, Some(email.to_owned()));
    user.user_status = UserStatus::Active;
    user.role = role;
    resources.common.repos.users.create(&user).await.unwrap();
    let bearer = format!("Bearer {}", generate_test_token(resources, &user).await);
    (user, bearer)
}

/// The stored account, as the next request will read it.
async fn stored(resources: &Arc<ServerContext>, user_id: Uuid) -> User {
    resources
        .common
        .repos
        .users
        .get_global(user_id)
        .await
        .unwrap()
        .expect("the account exists")
}

async fn start_impersonation(
    resources: &Arc<ServerContext>,
    bearer: &str,
    target: Uuid,
) -> (u16, String) {
    let response = AxumTestRequest::post("/api/admin/impersonate")
        .header("authorization", bearer)
        .json(&json!({ "target_user_id": target.to_string(), "reason": "Ticket 1567" }))
        .send(ImpersonationRoutes::routes(Arc::clone(resources)))
        .await;
    (response.status(), response.text())
}

async fn change_admin(
    resources: &Arc<ServerContext>,
    bearer: &str,
    target: Uuid,
    action: &str,
) -> (u16, String) {
    let response = AxumTestRequest::post(&format!("/api/admin/users/{target}/{action}"))
        .header("authorization", bearer)
        .json(&json!({}))
        .send(WebAdminRoutes::routes(resources.web_admin_context()))
        .await;
    (response.status(), response.text())
}

async fn list_admins(resources: &Arc<ServerContext>, bearer: &str) -> (u16, String) {
    let response = AxumTestRequest::get("/api/admin/admins")
        .header("authorization", bearer)
        .send(WebAdminRoutes::routes(resources.web_admin_context()))
        .await;
    (response.status(), response.text())
}

/// Whether an impersonation session names `user_id` on either side.
async fn has_live_impersonation(resources: &Arc<ServerContext>, user_id: Uuid) -> bool {
    resources
        .common
        .repos
        .impersonation
        .get_active_session(user_id)
        .await
        .unwrap()
        .is_some()
}

/// An admin acts as a user, and never as another admin: that session would
/// hold every power under a name the audit trail gives to someone else. The
/// refusal opens no session.
#[tokio::test]
async fn an_admin_impersonates_a_user_and_never_another_admin() {
    let resources = create_test_server_resources().await.unwrap();
    let (admin, admin_auth) = account(&resources, "acting@example.com", UserRole::Admin).await;
    let (peer, _) = account(&resources, "peer@example.com", UserRole::Admin).await;
    let (athlete, _) = account(&resources, "athlete@example.com", UserRole::User).await;

    let (status, body) = start_impersonation(&resources, &admin_auth, peer.id).await;
    assert_eq!(status, 403, "an admin is never impersonated: {body}");
    assert!(
        body.contains("Cannot impersonate another admin"),
        "the refusal says which rule held: {body}"
    );
    assert!(
        !has_live_impersonation(&resources, peer.id).await,
        "the refused request opened no session on the other admin"
    );

    let (status, body) = start_impersonation(&resources, &admin_auth, admin.id).await;
    assert_eq!(status, 400, "an admin does not impersonate itself: {body}");

    let (status, body) = start_impersonation(&resources, &admin_auth, athlete.id).await;
    assert_eq!(status, 200, "an admin impersonates a user: {body}");
    let started: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(started["target_user"]["id"], athlete.id.to_string());
    assert_eq!(started["target_user"]["role"], "user");
    assert!(
        started["token"].as_str().is_some_and(|t| !t.is_empty()),
        "the session comes with its token: {started}"
    );
    assert!(has_live_impersonation(&resources, athlete.id).await);
}

/// Impersonation is an admin's. A user is refused whoever the target is, and
/// the token of a live impersonation, which signs in as the user, starts no
/// second one.
#[tokio::test]
async fn a_user_and_an_impersonation_token_start_no_impersonation() {
    let resources = create_test_server_resources().await.unwrap();
    let (admin, admin_auth) = account(&resources, "acting@example.com", UserRole::Admin).await;
    let (athlete, athlete_auth) = account(&resources, "athlete@example.com", UserRole::User).await;
    let (other, _) = account(&resources, "other@example.com", UserRole::User).await;

    for target in [other.id, admin.id] {
        let (status, body) = start_impersonation(&resources, &athlete_auth, target).await;
        assert_eq!(status, 403, "a user impersonates no one: {body}");
        assert!(
            !has_live_impersonation(&resources, target).await,
            "the refused request opened no session"
        );
    }

    let (status, body) = start_impersonation(&resources, &admin_auth, athlete.id).await;
    assert_eq!(status, 200, "{body}");
    let started: Value = serde_json::from_str(&body).unwrap();
    let as_athlete = format!("Bearer {}", started["token"].as_str().unwrap());
    let (status, body) = start_impersonation(&resources, &as_athlete, other.id).await;
    assert_eq!(
        status, 403,
        "an impersonation token carries the user's standing, not the admin's: {body}"
    );
    assert!(!has_live_impersonation(&resources, other.id).await);
}

/// Any admin promotes a user, and the account promoted holds every admin
/// power at once, managing admins included: it reads the roster and demotes
/// the admin who promoted it, whose session then manages nothing.
#[tokio::test]
async fn an_admin_promotes_a_user_who_then_manages_admins() {
    let resources = create_test_server_resources().await.unwrap();
    let (first, first_auth) = account(&resources, "first@example.com", UserRole::Admin).await;
    let (second, second_auth) = account(&resources, "second@example.com", UserRole::User).await;

    let (status, body) = list_admins(&resources, &second_auth).await;
    assert_eq!(status, 403, "a user reads no admin roster: {body}");

    let (status, body) = change_admin(&resources, &first_auth, second.id, "promote").await;
    assert_eq!(status, 200, "an admin promotes a user: {body}");
    let promoted: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(promoted["user"]["role"], "admin");
    assert_eq!(promoted["user"]["is_admin"], true);
    let row = stored(&resources, second.id).await;
    assert_eq!(row.role, UserRole::Admin);

    let (status, body) = change_admin(&resources, &first_auth, second.id, "promote").await;
    assert_eq!(status, 400, "an admin is not promoted twice: {body}");

    // The session `second` held as a user is an admin's from this request on:
    // the role is read from the row, never from the token.
    let (status, body) = list_admins(&resources, &second_auth).await;
    assert_eq!(status, 200, "the promoted account reads the roster: {body}");
    let roster: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(roster["count"], 2, "{roster}");
    assert!(
        roster["admins"]
            .as_array()
            .is_some_and(|admins| admins.iter().all(|a| a["role"] == "admin")),
        "one admin role is listed, and only it: {roster}"
    );

    let (status, body) = change_admin(&resources, &second_auth, first.id, "demote").await;
    assert_eq!(status, 200, "an admin demotes another admin: {body}");
    let demoted: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(demoted["user"]["role"], "user");
    assert_eq!(demoted["user"]["is_admin"], false);
    let row = stored(&resources, first.id).await;
    assert_eq!(row.role, UserRole::User);

    let (status, body) = list_admins(&resources, &first_auth).await;
    assert_eq!(status, 403, "a demoted admin reads no roster: {body}");
    let (status, body) = change_admin(&resources, &first_auth, first.id, "promote").await;
    assert_eq!(
        status, 403,
        "a demoted admin does not promote itself back: {body}"
    );
    assert_eq!(stored(&resources, first.id).await.role, UserRole::User);
}

/// A user manages no admin: it neither promotes itself or anyone else nor
/// demotes an admin, and every refusal leaves the rows as they were.
#[tokio::test]
async fn a_user_neither_promotes_nor_demotes() {
    let resources = create_test_server_resources().await.unwrap();
    let (admin, _) = account(&resources, "standing@example.com", UserRole::Admin).await;
    let (athlete, athlete_auth) = account(&resources, "athlete@example.com", UserRole::User).await;
    let (other, _) = account(&resources, "other@example.com", UserRole::User).await;

    for target in [athlete.id, other.id] {
        let (status, body) = change_admin(&resources, &athlete_auth, target, "promote").await;
        assert_eq!(
            status, 403,
            "a user promotes no one, itself included: {body}"
        );
        let row = stored(&resources, target).await;
        assert_eq!(
            row.role,
            UserRole::User,
            "the refused promotion wrote nothing"
        );
    }

    let (status, body) = change_admin(&resources, &athlete_auth, admin.id, "demote").await;
    assert_eq!(status, 403, "a user demotes no admin: {body}");
    let row = stored(&resources, admin.id).await;
    assert_eq!(
        row.role,
        UserRole::Admin,
        "the refused demotion wrote nothing"
    );
}

/// An admin does not demote itself, and the one admin left cannot be
/// suspended either, by a caller with no session of its own (`pierre-cli`
/// on the host): both are the caller's mistake (400), never a server fault,
/// and the account stays an active admin.
#[tokio::test]
async fn the_only_admin_neither_demotes_nor_suspends_itself() {
    let resources = create_test_server_resources().await.unwrap();
    let (only, only_auth) = account(&resources, "only@example.com", UserRole::Admin).await;

    let (status, body) = change_admin(&resources, &only_auth, only.id, "demote").await;
    assert_eq!(status, 400, "an admin does not demote itself: {body}");

    let host = AccountCaller::host_cli(Some(only.id));
    let refused = admin_ops::transition_user_status(
        &resources.common.repos,
        &acting_on(&resources.common.repos, &host, only.id).await,
        UserStatus::Suspended,
    )
    .await
    .expect_err("the last active admin must not be suspended");
    assert_eq!(
        refused.code,
        ErrorCode::InvalidInput,
        "the repository's refusal reaches the caller as it is, not as an internal error"
    );
    assert!(
        refused.message.contains("last active admin"),
        "the refusal says why: {}",
        refused.message
    );

    let row = stored(&resources, only.id).await;
    assert!(
        row.is_active_admin(),
        "the only admin is still one, and active"
    );
}

/// An admin does not suspend itself, as it does not demote itself: it would
/// be signed out, its device logins withdrawn, and only another admin could
/// reactivate it. Its console session and its own device login are both
/// refused (400) and nothing is written; so is setting itself back to
/// pending, at the service every status write goes through. A second admin is active
/// throughout, so the last-active-admin rule is never what answers, and the
/// same session suspends that other admin: the rule is about the actor being
/// the target, not about admins.
#[tokio::test]
async fn an_admin_does_not_suspend_itself_and_suspends_another_admin() {
    let resources = create_test_server_resources().await.unwrap();
    let (acting, acting_auth) = account(&resources, "acting@example.com", UserRole::Admin).await;
    let (peer, _) = account(&resources, "peer@example.com", UserRole::Admin).await;
    let acting_cli = device_login(&resources, &acting.email).await;

    let from_the_console = suspend(&resources, &acting_auth, acting.id).await;
    let with_its_device_login = suspend_with_token(&resources, &acting_cli, acting.id).await;
    for (status, body) in [from_the_console, with_its_device_login] {
        assert_eq!(status, 400, "an admin does not suspend itself: {body}");
        assert!(
            body.contains("Cannot suspend yourself"),
            "the refusal is the self rule's, not the last admin's: {body}"
        );
    }
    assert!(
        stored(&resources, acting.id).await.is_active_admin(),
        "the refused suspension wrote nothing"
    );
    assert_eq!(
        lists_users(&resources, &acting_cli).await,
        200,
        "and the admin keeps its device login"
    );
    assert!(revocations_on_the_trail(&resources).await.is_empty());

    // The other move out of good standing: back to pending. No route makes
    // it, and the one function every status write goes through refuses it of
    // an admin acting on itself, in that rule's own words. The admin is
    // active and not the last one, so neither "already pending" nor the
    // last-admin rule can be what answers.
    let own_session = AccountCaller::of_token(&cookie_admin_token(&acting, None));
    let refused = admin_ops::transition_user_status(
        &resources.common.repos,
        &acting_on(&resources.common.repos, &own_session, acting.id).await,
        UserStatus::Pending,
    )
    .await
    .expect_err("an admin does not set itself back to pending");
    assert_eq!(refused.code, ErrorCode::InvalidInput);
    assert_eq!(refused.message, "Cannot set yourself back to pending");
    assert!(
        stored(&resources, acting.id).await.is_active_admin(),
        "the refused move wrote nothing"
    );

    // The positive control: the same session, the same route, another admin.
    let (status, body) = suspend(&resources, &acting_auth, peer.id).await;
    assert_eq!(status, 200, "an admin suspends another admin: {body}");
    assert_eq!(
        stored(&resources, peer.id).await.user_status,
        UserStatus::Suspended
    );
    assert!(stored(&resources, acting.id).await.is_active_admin());
}

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

/// The admin-token mount `pierre-cli` reaches, device login included. Every
/// route but the device handshake sits behind the admin-token middleware and
/// nothing else, so what it answers is that middleware's verdict on the token.
fn token_api(resources: &Arc<ServerContext>) -> Router {
    AdminRoutes::routes(admin_context(resources))
}

/// The console mount: the same handlers behind an admin's own session.
fn console(resources: &Arc<ServerContext>) -> Router {
    AdminRoutes::cookie_admin_routes::<ServerContext>(admin_context(resources), resources)
}

/// Start a device login the way `pierre-cli auth login` does: the secret the
/// CLI keeps, and the code an admin types on the approval page.
async fn start_device_login(resources: &Arc<ServerContext>) -> (String, String) {
    let response = AxumTestRequest::post("/admin/device/authorization")
        .send(token_api(resources))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let started: Value = response.json();
    (
        started["device_code"].as_str().unwrap().to_owned(),
        started["user_code"].as_str().unwrap().to_owned(),
    )
}

/// Sign `email` in on the approval page and approve `user_code`; the page.
async fn approve_on_the_page(
    resources: &Arc<ServerContext>,
    email: &str,
    user_code: &str,
) -> String {
    let response = AxumTestRequest::post("/admin/device/approve-web")
        .form(&[
            ("email", email),
            ("password", PASSWORD),
            ("user_code", user_code),
            ("action", "approve"),
        ])
        .send(token_api(resources))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    response.text()
}

/// What the CLI's poll for `device_code` answers: the status and the body.
async fn poll_device_token(resources: &Arc<ServerContext>, device_code: &str) -> (u16, Value) {
    let response = AxumTestRequest::post("/admin/device/token")
        .json(&json!({ "grant_type": DEVICE_GRANT, "device_code": device_code }))
        .send(token_api(resources))
        .await;
    (response.status(), response.json())
}

/// A whole device login approved by `email`: the bearer header of the
/// non-expiring admin token the CLI is handed.
async fn device_login(resources: &Arc<ServerContext>, email: &str) -> String {
    let (device_code, user_code) = start_device_login(resources).await;
    let page = approve_on_the_page(resources, email, &user_code).await;
    assert!(page.contains("Login approved"), "{email} approves: {page}");
    let (status, token) = poll_device_token(resources, &device_code).await;
    assert_eq!(status, 200, "the approved login mints its token: {token}");
    format!("Bearer {}", token["access_token"].as_str().unwrap())
}

/// What the admin-token middleware answers `bearer` on the user listing.
async fn lists_users(resources: &Arc<ServerContext>, bearer: &str) -> u16 {
    AxumTestRequest::get("/admin/users")
        .header("authorization", bearer)
        .send(token_api(resources))
        .await
        .status()
}

/// Suspend `target` from the console, as the admin whose session `bearer` is.
async fn suspend(resources: &Arc<ServerContext>, bearer: &str, target: Uuid) -> (u16, String) {
    let response = AxumTestRequest::post(&format!("/api/admin/suspend-user/{target}"))
        .header("authorization", bearer)
        .json(&json!({ "reason": "carnet#1574" }))
        .send(console(resources))
        .await;
    (response.status(), response.text())
}

/// Approve `target` again from the console.
async fn approve(resources: &Arc<ServerContext>, bearer: &str, target: Uuid) -> (u16, String) {
    let response = AxumTestRequest::post(&format!("/api/admin/approve-user/{target}"))
        .header("authorization", bearer)
        .json(&json!({}))
        .send(console(resources))
        .await;
    (response.status(), response.text())
}

/// Delete `target` with the admin token `bearer`.
async fn delete_account(
    resources: &Arc<ServerContext>,
    bearer: &str,
    target: Uuid,
) -> (u16, String) {
    let response = AxumTestRequest::delete(&format!("/admin/users/{target}"))
        .header("authorization", bearer)
        .json(&json!({ "reason": "carnet#1574" }))
        .send(token_api(resources))
        .await;
    (response.status(), response.text())
}

/// What a request made with `bearer` to a route any signed-in account reads
/// is answered: 200 while the credential acts as its user, 401 once refused.
async fn reads_own_stats(resources: &Arc<ServerContext>, bearer: &str) -> u16 {
    AxumTestRequest::get("/api/user/stats")
        .header("authorization", bearer)
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await
        .status()
}

/// Open an impersonation of `target` as the admin whose session `bearer` is;
/// the bearer header of the impersonation token.
async fn impersonate(resources: &Arc<ServerContext>, bearer: &str, target: Uuid) -> String {
    let (status, body) = start_impersonation(resources, bearer, target).await;
    assert_eq!(status, 200, "{body}");
    let started: Value = serde_json::from_str(&body).unwrap();
    format!("Bearer {}", started["token"].as_str().unwrap())
}

/// The rows the trail holds of admin credentials withdrawn, newest first.
async fn revocations_on_the_trail(resources: &Arc<ServerContext>) -> Vec<AuditEventRecord> {
    let filter = AuditEventFilter {
        action_prefix: Some(ADMIN_CREDENTIALS_REVOKED.to_owned()),
        ..AuditEventFilter::default()
    };
    resources
        .common
        .repos
        .admin_audit
        .list(&filter, 50)
        .await
        .unwrap()
}

/// Whether a token acting as `admin` is still live in the token table.
async fn holds_a_live_device_token(resources: &Arc<ServerContext>, admin: Uuid) -> bool {
    resources
        .common
        .repos
        .admin
        .list_tokens(false)
        .await
        .unwrap()
        .iter()
        .any(|token| token.operator_user_id == Some(admin))
}

/// A device login is a non-expiring token carrying its approver's every
/// power. Demoting the approver revokes it in the same request, the trail
/// says so, and promoting the account again does not bring the token back.
/// Another admin's device login is the control: it works before and after.
#[tokio::test]
async fn a_demoted_admin_loses_the_device_logins_it_approved() {
    let resources = create_test_server_resources().await.unwrap();
    let (leaving, _) = account(&resources, "leaving@example.com", UserRole::Admin).await;
    let (staying, staying_auth) = account(&resources, "staying@example.com", UserRole::Admin).await;
    let leaving_cli = device_login(&resources, &leaving.email).await;
    let staying_cli = device_login(&resources, &staying.email).await;
    assert_eq!(lists_users(&resources, &leaving_cli).await, 200);
    assert_eq!(lists_users(&resources, &staying_cli).await, 200);
    assert!(revocations_on_the_trail(&resources).await.is_empty());

    let (status, body) = change_admin(&resources, &staying_auth, leaving.id, "demote").await;
    assert_eq!(status, 200, "{body}");

    assert_eq!(
        lists_users(&resources, &leaving_cli).await,
        401,
        "the demoted admin's device login is refused"
    );
    assert_eq!(
        lists_users(&resources, &staying_cli).await,
        200,
        "the other admin's device login is untouched"
    );
    assert!(!holds_a_live_device_token(&resources, leaving.id).await);
    assert!(holds_a_live_device_token(&resources, staying.id).await);

    let trail = revocations_on_the_trail(&resources).await;
    assert_eq!(trail.len(), 1, "one withdrawal, one row: {trail:?}");
    let row = &trail[0];
    assert_eq!(row.outcome, "success");
    assert_eq!(row.source, "console");
    assert_eq!(
        row.actor_user_id.as_deref(),
        Some(staying.id.to_string().as_str()),
        "the row names the admin who demoted"
    );
    assert_eq!(row.metadata["cause"], "demoted");
    assert_eq!(row.metadata["target"]["user_id"], leaving.id.to_string());
    assert_eq!(row.metadata["device_tokens_revoked"], 1);
    assert_eq!(
        row.metadata["device_token_ids"].as_array().map(Vec::len),
        Some(1)
    );

    // Revoked, not merely refused while the role is gone: the token stays
    // dead once the account is an admin again.
    let (status, body) = change_admin(&resources, &staying_auth, leaving.id, "promote").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        lists_users(&resources, &leaving_cli).await,
        401,
        "a promotion does not revive a revoked device login"
    );
    let fresh = device_login(&resources, &leaving.email).await;
    assert_eq!(
        lists_users(&resources, &fresh).await,
        200,
        "the admin signs in again for a new one"
    );
}

/// A suspended admin still knows its password and still has the role, so the
/// suspension is what must end its device login: the token it holds is
/// revoked, it approves no new one, and approving the account again brings
/// the old token back no more than a promotion does.
#[tokio::test]
async fn a_suspended_admin_loses_its_device_login_and_approves_no_new_one() {
    let resources = create_test_server_resources().await.unwrap();
    let (leaving, _) = account(&resources, "leaving@example.com", UserRole::Admin).await;
    let (staying, staying_auth) = account(&resources, "staying@example.com", UserRole::Admin).await;
    let leaving_cli = device_login(&resources, &leaving.email).await;
    assert_eq!(lists_users(&resources, &leaving_cli).await, 200);

    let (status, body) = suspend(&resources, &staying_auth, leaving.id).await;
    assert_eq!(status, 200, "{body}");

    assert_eq!(
        lists_users(&resources, &leaving_cli).await,
        401,
        "the suspended admin's device login is refused"
    );
    assert!(!holds_a_live_device_token(&resources, leaving.id).await);
    let trail = revocations_on_the_trail(&resources).await;
    assert_eq!(trail.len(), 1, "{trail:?}");
    assert_eq!(trail[0].metadata["cause"], "deactivated");
    assert_eq!(
        trail[0].actor_user_id.as_deref(),
        Some(staying.id.to_string().as_str())
    );

    // The role is still `admin`; the password still verifies. Neither mints.
    let (device_code, user_code) = start_device_login(&resources).await;
    let page = approve_on_the_page(&resources, &leaving.email, &user_code).await;
    assert!(
        page.contains("not an admin in good standing"),
        "a suspended admin approves no device login: {page}"
    );
    let (status, pending) = poll_device_token(&resources, &device_code).await;
    assert_eq!(status, 400, "{pending}");
    assert_eq!(pending["error"], "authorization_pending", "{pending}");
    // The control: the same request, approved by an admin in good standing.
    let page = approve_on_the_page(&resources, &staying.email, &user_code).await;
    assert!(page.contains("Login approved"), "{page}");
    let (status, minted) = poll_device_token(&resources, &device_code).await;
    assert_eq!(status, 200, "{minted}");

    let (status, body) = approve(&resources, &staying_auth, leaving.id).await;
    assert_eq!(status, 200, "{body}");
    assert!(stored(&resources, leaving.id).await.is_active_admin());
    assert_eq!(
        lists_users(&resources, &leaving_cli).await,
        401,
        "approving the account again does not revive its revoked device login"
    );
}

/// Mint a service token as `pierre-cli token generate` does, naming no
/// admin: with the super-admin flag, or with `ManageUsers` alone. Its bearer
/// header.
async fn service_token(resources: &Arc<ServerContext>, name: &str, super_admin: bool) -> String {
    let request = if super_admin {
        CreateAdminTokenRequest::super_admin(name.to_owned())
    } else {
        CreateAdminTokenRequest {
            permissions: Some(vec![AdminPermission::ManageUsers]),
            ..CreateAdminTokenRequest::new(name.to_owned())
        }
    };
    let minted = resources
        .common
        .repos
        .admin
        .create_token(
            &request,
            resources.auth.admin_jwt_secret.as_ref(),
            &resources.auth.jwks_manager,
        )
        .await
        .unwrap();
    format!("Bearer {}", minted.jwt_token.expose_secret())
}

/// What a token without the super-admin flag is answered on an admin account.
const RANK_REFUSAL: &str = "Acting on an admin account requires a super-admin token";

/// A `ManageUsers` token without the super-admin flag, bound to a desk
/// tenant every one of `members` is held in and in no other: a token that
/// reaches those accounts, so that what refuses it on an admin is the rank
/// rule. A token bound to no tenant reaches no account (carnet#1589). Its
/// bearer header.
async fn desk_token(resources: &Arc<ServerContext>, members: &[Uuid]) -> String {
    let repos = &resources.common.repos;
    let mut owner = User::new(
        format!("desk-owner-{}@example.com", Uuid::new_v4()),
        "hash".to_owned(),
        None,
    );
    owner.user_status = UserStatus::Active;
    repos.users.create(&owner).await.unwrap();
    let desk = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: desk,
            name: "support desk".to_owned(),
            slug: format!("desk-{desk}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: owner.id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    for member in members {
        repos.users.update_tenant_id(*member, desk).await.unwrap();
    }
    tenant_bound_token(resources, desk).await
}

/// Suspend `target` with the admin token `bearer`, as `pierre-cli` does.
async fn suspend_with_token(
    resources: &Arc<ServerContext>,
    bearer: &str,
    target: Uuid,
) -> (u16, String) {
    let response = AxumTestRequest::post(&format!("/admin/suspend-user/{target}"))
        .header("authorization", bearer)
        .json(&json!({ "reason": "carnet#1574" }))
        .send(token_api(resources))
        .await;
    (response.status(), response.text())
}

/// Suspending an admin withdraws the device logins it approved, which is
/// more than `ManageUsers` holds: a service token suspends an admin only
/// with the super-admin flag, as it deletes one. Without the flag the same
/// token still suspends a user. A second admin stays active throughout, so
/// the last-active-admin guard is never what answers.
#[tokio::test]
async fn a_token_without_the_super_admin_flag_suspends_a_user_and_never_an_admin() {
    let resources = create_test_server_resources().await.unwrap();
    let (admin, _) = account(&resources, "standing@example.com", UserRole::Admin).await;
    account(&resources, "peer@example.com", UserRole::Admin).await;
    let (athlete, _) = account(&resources, "athlete@example.com", UserRole::User).await;
    let admin_cli = device_login(&resources, &admin.email).await;
    let manage_users = desk_token(&resources, &[admin.id, athlete.id]).await;
    let flagged = service_token(&resources, "platform-ops", true).await;

    let (status, body) = suspend_with_token(&resources, &manage_users, admin.id).await;
    assert_eq!(status, 403, "ManageUsers alone suspends no admin: {body}");
    assert!(
        body.contains(RANK_REFUSAL),
        "the refusal is the rank rule's: {body}"
    );
    assert!(
        stored(&resources, admin.id).await.is_active_admin(),
        "the refused suspension wrote nothing"
    );
    assert_eq!(
        lists_users(&resources, &admin_cli).await,
        200,
        "and the admin keeps its device login"
    );
    assert!(revocations_on_the_trail(&resources).await.is_empty());

    // The positive control: the same token, the same route, a plain user.
    let (status, body) = suspend_with_token(&resources, &manage_users, athlete.id).await;
    assert_eq!(status, 200, "ManageUsers suspends a user: {body}");
    assert_eq!(
        stored(&resources, athlete.id).await.user_status,
        UserStatus::Suspended
    );

    // The flag is the rank: the same request from a super-admin token lands.
    let (status, body) = suspend_with_token(&resources, &flagged, admin.id).await;
    assert_eq!(status, 200, "a super-admin token suspends an admin: {body}");
    assert_eq!(
        stored(&resources, admin.id).await.user_status,
        UserStatus::Suspended
    );
    assert_eq!(
        lists_users(&resources, &admin_cli).await,
        401,
        "and the suspended admin's device login is withdrawn with it"
    );
}

/// Approve `target` with the admin token `bearer`, as `pierre-cli` does.
async fn approve_with_token(
    resources: &Arc<ServerContext>,
    bearer: &str,
    target: Uuid,
) -> (u16, String) {
    let response = AxumTestRequest::post(&format!("/admin/approve-user/{target}"))
        .header("authorization", bearer)
        .json(&json!({ "reason": "carnet#1574" }))
        .send(token_api(resources))
        .await;
    (response.status(), response.text())
}

/// A suspended admin approved again holds every power, so approving one
/// takes the rank suspending one does: without it a `ManageUsers` token
/// undid the suspension it could not make. The same token still approves a
/// user. A second admin stays active throughout, so the last-active-admin
/// guard is never what answers.
#[tokio::test]
async fn a_token_without_the_super_admin_flag_approves_a_user_and_never_an_admin() {
    let resources = create_test_server_resources().await.unwrap();
    let (admin, _) = account(&resources, "suspended@example.com", UserRole::Admin).await;
    account(&resources, "peer@example.com", UserRole::Admin).await;
    let (athlete, _) = account(&resources, "athlete@example.com", UserRole::User).await;
    let manage_users = desk_token(&resources, &[admin.id, athlete.id]).await;
    let flagged = service_token(&resources, "platform-ops", true).await;
    for suspended in [admin.id, athlete.id] {
        let (status, body) = suspend_with_token(&resources, &flagged, suspended).await;
        assert_eq!(status, 200, "{body}");
    }

    let (status, body) = approve_with_token(&resources, &manage_users, admin.id).await;
    assert_eq!(status, 403, "ManageUsers alone reinstates no admin: {body}");
    assert!(
        body.contains(RANK_REFUSAL),
        "the refusal is the rank rule's: {body}"
    );
    assert_eq!(
        stored(&resources, admin.id).await.user_status,
        UserStatus::Suspended,
        "the refused approval wrote nothing"
    );

    // The positive control: the same token, the same route, a plain user.
    let (status, body) = approve_with_token(&resources, &manage_users, athlete.id).await;
    assert_eq!(status, 200, "ManageUsers approves a user: {body}");
    assert_eq!(
        stored(&resources, athlete.id).await.user_status,
        UserStatus::Active
    );

    // The flag is the rank: the same request from a super-admin token lands.
    let (status, body) = approve_with_token(&resources, &flagged, admin.id).await;
    assert_eq!(status, 200, "a super-admin token approves an admin: {body}");
    assert!(stored(&resources, admin.id).await.is_active_admin());
}

/// An account still waiting for approval, with `role`: a pending sign-up,
/// or one an admin promoted before anyone approved it.
async fn pending_account(resources: &Arc<ServerContext>, email: &str, role: UserRole) -> User {
    let password_hash = bcrypt::hash(PASSWORD, 4).unwrap();
    let mut user = User::new(email.to_owned(), password_hash, Some(email.to_owned()));
    user.user_status = UserStatus::Pending;
    user.role = role;
    resources.common.repos.users.create(&user).await.unwrap();
    user
}

/// Pre-approve `email` with the admin token `bearer`, as
/// `pierre-cli user allow` does.
async fn allow_with_token(
    resources: &Arc<ServerContext>,
    bearer: &str,
    email: &str,
) -> (u16, String) {
    let response = AxumTestRequest::post("/admin/pre-approved-emails")
        .header("authorization", bearer)
        .json(&json!({ "email": email }))
        .send(token_api(resources))
        .await;
    (response.status(), response.text())
}

/// An allow approves a pending account on the spot, whatever tenant it is
/// or will be in: it is platform-wide, so a token without the super-admin
/// flag is refused it before anything is written, bound to a tenant (in its
/// own sentence) or to none (it reaches no tenant), on a pending admin's
/// address and a pending user's alike (carnet#1589). The rank an allow
/// takes on a pending admin is the service's (`account_caller_rank_test`).
/// The flag is the rank.
#[tokio::test]
async fn a_token_without_the_super_admin_flag_allows_no_pending_account() {
    let resources = create_test_server_resources().await.unwrap();
    account(&resources, "standing@example.com", UserRole::Admin).await;
    let admin = pending_account(&resources, "promoted@example.com", UserRole::Admin).await;
    let athlete = pending_account(&resources, "waiting@example.com", UserRole::User).await;
    let bound = desk_token(&resources, &[admin.id, athlete.id]).await;
    let unbound = service_token(&resources, "support-desk", false).await;
    let flagged = service_token(&resources, "platform-ops", true).await;

    for (caller, refusal) in [
        (&bound, "a pre-approval is platform-wide"),
        (&unbound, "Token has no tenant scope"),
    ] {
        // The address as a person types it: the account is found whatever
        // its casing or the space around it.
        for address in [" Promoted@Example.com ", athlete.email.as_str()] {
            let (status, body) = allow_with_token(&resources, caller, address).await;
            assert_eq!(status, 403, "{address}: {body}");
            assert!(body.contains(refusal), "{address}: {body}");
        }
    }
    for pending in [admin.id, athlete.id] {
        assert_eq!(
            stored(&resources, pending).await.user_status,
            UserStatus::Pending,
            "the refused allows approved nobody"
        );
    }
    let standing_allows = resources
        .common
        .repos
        .pre_approved_emails
        .list()
        .await
        .unwrap();
    assert!(
        standing_allows.is_empty(),
        "and recorded no standing allow: {standing_allows:?}"
    );

    // The flag is the rank: the same request from a super-admin token lands.
    let (status, body) = allow_with_token(&resources, &flagged, &admin.email).await;
    assert_eq!(status, 200, "a super-admin token approves an admin: {body}");
    assert!(stored(&resources, admin.id).await.is_active_admin());
}

/// A `ManageUsers` token bound to `tenant`, as
/// `pierre-cli token generate --permissions manage_users --tenant-id` mints
/// it: the scope `admin_user_removal_test` holds a delete and a disconnect
/// to. Its bearer header.
async fn tenant_bound_token(resources: &Arc<ServerContext>, tenant: TenantId) -> String {
    let request = CreateAdminTokenRequest {
        permissions: Some(vec![AdminPermission::ManageUsers]),
        tenant_id: Some(tenant.to_string()),
        ..CreateAdminTokenRequest::new("club-desk".to_owned())
    };
    let minted = resources
        .common
        .repos
        .admin
        .create_token(
            &request,
            resources.auth.admin_jwt_secret.as_ref(),
            &resources.auth.jwks_manager,
        )
        .await
        .unwrap();
    format!("Bearer {}", minted.jwt_token.expose_secret())
}

/// A suspension and an approval change the account in every tenant it is
/// in, as a delete does, so a token bound to one tenant takes the delete's
/// rule on both: a user held in another tenant is not found (404, as a
/// missing account), with nothing written, and a user of its own is
/// accepted.
#[tokio::test]
async fn a_tenant_bound_token_suspends_and_approves_only_inside_its_tenant() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let (inside, tenant, _) = seed_user(repos, "inside").await;
    let (outside, _, _) = seed_user(repos, "outside").await;
    let bound = tenant_bound_token(&resources, tenant).await;
    let flagged = service_token(&resources, "platform-ops", true).await;
    let refusal = "ResourceNotFound";

    let (status, body) = suspend_with_token(&resources, &bound, outside).await;
    assert_eq!(status, 404, "another tenant's user is out of scope: {body}");
    assert!(body.contains(refusal), "{body}");
    assert_eq!(
        stored(&resources, outside).await.user_status,
        UserStatus::Active,
        "the refused suspension wrote nothing"
    );
    let (status, body) = suspend_with_token(&resources, &bound, inside).await;
    assert_eq!(
        status, 200,
        "the token suspends a user of its tenant: {body}"
    );
    assert_eq!(
        stored(&resources, inside).await.user_status,
        UserStatus::Suspended
    );

    // Suspended by a token that reaches every tenant, so an approval has
    // something to undo.
    let (status, body) = suspend_with_token(&resources, &flagged, outside).await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = approve_with_token(&resources, &bound, outside).await;
    assert_eq!(status, 404, "another tenant's user is out of scope: {body}");
    assert!(body.contains(refusal), "{body}");
    assert_eq!(
        stored(&resources, outside).await.user_status,
        UserStatus::Suspended,
        "the refused approval wrote nothing"
    );
    let (status, body) = approve_with_token(&resources, &bound, inside).await;
    assert_eq!(
        status, 200,
        "the token approves a user of its tenant: {body}"
    );
    assert_eq!(
        stored(&resources, inside).await.user_status,
        UserStatus::Active
    );
}

/// What stands where an answer carried a reset token.
const RESET_TOKEN_NOT_QUOTED: &str = "[a reset token, not quoted]";

/// Ask for a password reset of `target` on `router` (the token mount or the
/// console), at `path`: the status, the answer fit to quote in a failed
/// assertion, and the reset token it handed out. The answer carries that
/// token on the very failure where a reset that should have been refused
/// went through, so it is replaced in what is quoted and returned apart
/// for the one test that completes the reset.
async fn reset_password(router: Router, path: &str, bearer: &str) -> (u16, String, Option<String>) {
    let response = AxumTestRequest::post(path)
        .header("authorization", bearer)
        .send(router)
        .await;
    let status = response.status();
    let text = response.text();
    let Ok(mut answer) = serde_json::from_str::<Value>(&text) else {
        return (status, text, None);
    };
    let reset_token = answer.pointer_mut("/data/reset_token").and_then(|token| {
        let handed_out = token.as_str().map(ToOwned::to_owned);
        *token = json!(RESET_TOKEN_NOT_QUOTED);
        handed_out
    });
    (status, answer.to_string(), reset_token)
}

/// How many password reset tokens were ever issued for `account`, read past
/// the repository.
pub(crate) async fn reset_tokens_issued_for(resources: &Arc<ServerContext>, account: Uuid) -> i64 {
    const SQL: &str = "SELECT COUNT(*) FROM password_reset_tokens WHERE CAST(user_id AS TEXT) = $1";
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(backend) => sqlx::query_scalar(SQL)
            .bind(account.to_string())
            .fetch_one(backend.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(backend) => sqlx::query_scalar(SQL)
            .bind(account.to_string())
            .fetch_one(backend.pool())
            .await
            .unwrap(),
    }
}

/// Complete the reset `reset_token` opens with a new password; the status.
async fn complete_reset(resources: &Arc<ServerContext>, reset_token: &str, password: &str) -> u16 {
    AxumTestRequest::post("/api/auth/complete-reset")
        .json(&json!({ "reset_token": reset_token, "new_password": password }))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await
        .status()
}

/// The reset route answers with the reset token itself, and whoever holds it
/// chooses the account's next password and signs in as the account. Handed
/// to a `ManageUsers` token for an admin, that was every power in two
/// requests, and the admin locked out. Such a token is handed a reset for a
/// user only; a super-admin token and an admin's console session reset an
/// admin's.
#[tokio::test]
async fn a_token_without_the_super_admin_flag_resets_a_users_password_and_never_an_admins() {
    let resources = create_test_server_resources().await.unwrap();
    let (admin, _) = account(&resources, "standing@example.com", UserRole::Admin).await;
    let (_, peer_auth) = account(&resources, "peer@example.com", UserRole::Admin).await;
    let (athlete, _) = account(&resources, "athlete@example.com", UserRole::User).await;
    let manage_users = desk_token(&resources, &[admin.id, athlete.id]).await;
    let flagged = service_token(&resources, "platform-ops", true).await;
    let token_path = |target: Uuid| format!("/admin/users/{target}/reset-password");

    let (status, body, _) =
        reset_password(token_api(&resources), &token_path(admin.id), &manage_users).await;
    assert_eq!(status, 403, "ManageUsers alone resets no admin: {body}");
    assert!(
        body.contains(RANK_REFUSAL),
        "the refusal is the rank rule's: {body}"
    );
    assert!(
        !body.contains("reset_token"),
        "the refusal hands over no reset token: {body}"
    );
    assert_eq!(
        reset_tokens_issued_for(&resources, admin.id).await,
        0,
        "and none was issued behind it"
    );
    // The admin's password is still the one it chose: it approves a login.
    device_login(&resources, &admin.email).await;

    // The positive control: the same token, the same route, a plain user,
    // and the reset it is handed is a working one.
    let (status, body, reset_token) = reset_password(
        token_api(&resources),
        &token_path(athlete.id),
        &manage_users,
    )
    .await;
    assert_eq!(status, 200, "ManageUsers resets a user: {body}");
    let reset_token = reset_token.expect("the answer carries the reset token");
    assert_eq!(reset_tokens_issued_for(&resources, athlete.id).await, 1);
    assert_eq!(
        complete_reset(&resources, &reset_token, "Chosen-by-the-athlete-1574!").await,
        200
    );

    // The flag is the rank: a super-admin token is handed an admin's reset,
    // and so is another admin's console session.
    let (status, body, _) =
        reset_password(token_api(&resources), &token_path(admin.id), &flagged).await;
    assert_eq!(status, 200, "a super-admin token resets an admin: {body}");
    let (status, body, _) = reset_password(
        console(&resources),
        &format!("/api/admin/users/{}/reset-password", admin.id),
        &peer_auth,
    )
    .await;
    assert_eq!(status, 200, "an admin's session resets an admin: {body}");
    assert_eq!(reset_tokens_issued_for(&resources, admin.id).await, 2);
}

/// An approval is not a token yet: the CLI polls for it. An admin demoted
/// between approving and the poll hands the CLI nothing.
#[tokio::test]
async fn an_approval_mints_nothing_once_its_admin_is_demoted() {
    let resources = create_test_server_resources().await.unwrap();
    let (leaving, _) = account(&resources, "leaving@example.com", UserRole::Admin).await;
    let (staying, staying_auth) = account(&resources, "staying@example.com", UserRole::Admin).await;

    let (device_code, user_code) = start_device_login(&resources).await;
    let page = approve_on_the_page(&resources, &leaving.email, &user_code).await;
    assert!(page.contains("Login approved"), "{page}");
    let (status, body) = change_admin(&resources, &staying_auth, leaving.id, "demote").await;
    assert_eq!(status, 200, "{body}");

    let (status, refused) = poll_device_token(&resources, &device_code).await;
    assert_eq!(status, 400, "{refused}");
    assert_eq!(refused["error"], "access_denied", "{refused}");
    assert!(
        !holds_a_live_device_token(&resources, leaving.id).await,
        "no token was minted for the demoted admin"
    );

    // The control: an approval whose admin is still one mints.
    let minted = device_login(&resources, &staying.email).await;
    assert_eq!(lists_users(&resources, &minted).await, 200);
}

/// How many `admin_tokens` rows carry `$1` in any text column.
const TOKEN_ROWS_NAMING_SQL: &str = "SELECT COUNT(*) FROM admin_tokens WHERE \
     CAST(id AS TEXT) LIKE $1 OR service_name LIKE $1 \
     OR COALESCE(service_description, '') LIKE $1 OR token_hash LIKE $1 \
     OR token_prefix LIKE $1 OR jwt_secret_hash LIKE $1 \
     OR CAST(permissions AS TEXT) LIKE $1 OR COALESCE(tenant_id, '') LIKE $1 \
     OR COALESCE(operator_user_id, '') LIKE $1 \
     OR COALESCE(CAST(last_used_ip AS TEXT), '') LIKE $1";

/// How many `admin_tokens` rows name `account` anywhere, read past the
/// repository: by the admin column, in the service name, or elsewhere.
async fn token_rows_naming(resources: &Arc<ServerContext>, account: Uuid) -> i64 {
    let needle = format!("%{account}%");
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(backend) => sqlx::query_scalar(TOKEN_ROWS_NAMING_SQL)
            .bind(needle)
            .fetch_one(backend.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(backend) => sqlx::query_scalar(TOKEN_ROWS_NAMING_SQL)
            .bind(needle)
            .fetch_one(backend.pool())
            .await
            .unwrap(),
    }
}

/// Deleting an admin ends its device login with it, in the deletion's own
/// transaction: the token is refused, no row is left that names the account
/// (the login's own row stays, naming no one), and the trail keeps that it
/// happened without keeping whose it was.
#[tokio::test]
async fn a_deleted_admin_leaves_no_working_device_login() {
    let resources = create_test_server_resources().await.unwrap();
    let (leaving, _) = account(&resources, "leaving@example.com", UserRole::Admin).await;
    let (staying, _) = account(&resources, "staying@example.com", UserRole::Admin).await;
    let leaving_cli = device_login(&resources, &leaving.email).await;
    let staying_cli = device_login(&resources, &staying.email).await;
    assert_eq!(lists_users(&resources, &leaving_cli).await, 200);
    assert_eq!(
        token_rows_naming(&resources, leaving.id).await,
        1,
        "the login's row names its admin"
    );

    let (status, body) = delete_account(&resources, &staying_cli, leaving.id).await;
    assert_eq!(status, 200, "{body}");

    assert_eq!(
        lists_users(&resources, &leaving_cli).await,
        401,
        "the deleted admin's device login is refused"
    );
    assert_eq!(
        lists_users(&resources, &staying_cli).await,
        200,
        "the admin who deleted keeps theirs"
    );
    assert!(!holds_a_live_device_token(&resources, leaving.id).await);
    assert_eq!(
        token_rows_naming(&resources, leaving.id).await,
        0,
        "no token row names the deleted account, by admin or by name"
    );
    assert_eq!(
        token_rows_naming(&resources, staying.id).await,
        1,
        "the other admin's row is as it was"
    );

    let trail = revocations_on_the_trail(&resources).await;
    assert_eq!(trail.len(), 1, "{trail:?}");
    let row = &trail[0];
    assert_eq!(row.metadata["cause"], "deleted");
    assert_eq!(row.source, "cli");
    assert_eq!(
        row.actor_user_id.as_deref(),
        Some(staying.id.to_string().as_str())
    );
    assert_eq!(row.metadata["device_tokens_revoked"], 1);
    let written = row.metadata.to_string();
    assert!(
        !written.contains(&leaving.id.to_string())
            && row.metadata.get("device_token_ids").is_none(),
        "the trail names neither the deleted account nor the tokens that named it: {written}"
    );
}

/// An admin deletes its own account with its own device login: the row of
/// the token making the request is retired in that request's transaction.
/// The request is answered as a deletion, not as a fault, and the token is
/// refused from then on.
#[tokio::test]
async fn an_admin_deleting_itself_with_its_own_device_login_is_answered() {
    let resources = create_test_server_resources().await.unwrap();
    let (leaving, _) = account(&resources, "leaving@example.com", UserRole::Admin).await;
    let (staying, _) = account(&resources, "staying@example.com", UserRole::Admin).await;
    let leaving_cli = device_login(&resources, &leaving.email).await;
    let staying_cli = device_login(&resources, &staying.email).await;

    let (status, body) = delete_account(&resources, &leaving_cli, leaving.id).await;
    assert_eq!(status, 200, "{body}");

    assert_eq!(lists_users(&resources, &leaving_cli).await, 401);
    assert_eq!(token_rows_naming(&resources, leaving.id).await, 0);
    assert_eq!(lists_users(&resources, &staying_cli).await, 200);
}

/// A device login another admin rotated is theirs from then on: the
/// replacement acts as no one and keeps the name, which carries the first
/// admin's id. Deleting that admin retires the rows that acted as it (the
/// original its rotation already withdrew, and a login still live), which
/// stay as rows naming no one, and takes its id out of the replacement's
/// name, which goes on working for whoever holds it. No row names the
/// account afterwards, and the retired original is not rotated a second
/// time: the answer names the replacement it already has (409).
#[tokio::test]
#[serial]
async fn a_deleted_admin_is_named_by_no_token_row_however_it_was_rotated() {
    let resources = create_test_server_resources().await.unwrap();
    let (leaving, _) = account(&resources, "leaving@example.com", UserRole::Admin).await;
    let (staying, staying_auth) = account(&resources, "staying@example.com", UserRole::Admin).await;
    let staying_cli = device_login(&resources, &staying.email).await;
    device_login(&resources, &leaving.email).await;
    let admin_tokens = &resources.common.repos.admin;
    let original = admin_tokens
        .list_tokens(false)
        .await
        .unwrap()
        .into_iter()
        .find(|token| token.operator_user_id == Some(leaving.id))
        .expect("the login's row")
        .id;

    // `staying` rotates it from the console and holds the replacement.
    let response = AxumTestRequest::post(&format!("/api/admin/tokens/{original}/rotate"))
        .header("authorization", &staying_auth)
        .json(&json!({}))
        .send(console(&resources))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let rotated: Value = response.json();
    let replacement = rotated["data"]["token_id"].as_str().unwrap().to_owned();
    let replacement_cli = format!("Bearer {}", rotated["data"]["jwt_token"].as_str().unwrap());
    assert_eq!(lists_users(&resources, &replacement_cli).await, 200);
    // And `leaving` signs in again: a second, live login of its own.
    let leaving_cli = device_login(&resources, &leaving.email).await;
    assert_eq!(
        token_rows_naming(&resources, leaving.id).await,
        3,
        "the retired original, the replacement's name, the new login"
    );

    let (status, body) = delete_account(&resources, &staying_cli, leaving.id).await;
    assert_eq!(status, 200, "{body}");

    assert_eq!(token_rows_naming(&resources, leaving.id).await, 0);
    assert_eq!(lists_users(&resources, &leaving_cli).await, 401);
    let retired = admin_tokens
        .get_token_by_id(&original)
        .await
        .unwrap()
        .expect("the retired original is kept");
    assert!(!retired.is_active);
    assert_eq!(retired.service_name, DEVICE_CLI_DELETED_ACCOUNT_SERVICE);
    assert_eq!(
        retired.operator_user_id,
        Some(DELETED_ACCOUNT_ADMIN),
        "kept under an admin that is no account, never without one"
    );
    let response = AxumTestRequest::post(&format!("/api/admin/tokens/{original}/rotate"))
        .header("authorization", &staying_auth)
        .json(&json!({}))
        .send(console(&resources))
        .await;
    assert_eq!(
        response.status(),
        409,
        "a retired row is not rotated back: {}",
        response.body_text()
    );
    let refusal: Value = response.json();
    assert_eq!(
        refusal["data"]["rotated_to_token_id"], replacement,
        "and the answer names the replacement it has: {refusal}"
    );

    let kept = admin_tokens
        .get_token_by_id(&replacement)
        .await
        .unwrap()
        .expect("the replacement is its holder's");
    assert_eq!(kept.service_name, DEVICE_CLI_DELETED_ACCOUNT_SERVICE);
    assert_eq!(kept.operator_user_id, None);
    assert_eq!(
        lists_users(&resources, &replacement_cli).await,
        200,
        "and still works for whoever rotated it"
    );
    assert_eq!(lists_users(&resources, &staying_cli).await, 200);

    // The replacement's signed claims still say the old name. What it writes
    // from here on is named from its row: the trail, which the production
    // composition records every admin write on, does not get the id back.
    let (athlete, _) = account(&resources, "athlete@example.com", UserRole::User).await;
    let response = AxumTestRequest::post(&format!("/admin/suspend-user/{}", athlete.id))
        .header("authorization", &replacement_cli)
        .json(&json!({ "reason": "carnet#1574" }))
        .send(ProviderToolRouter::build_http_app(&resources))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let written = resources
        .common
        .repos
        .admin_audit
        .list(&AuditEventFilter::default(), 200)
        .await
        .unwrap();
    assert!(
        written
            .iter()
            .any(|row| row.metadata["service"] == DEVICE_CLI_DELETED_ACCOUNT_SERVICE),
        "the suspension is on the trail under the row's name: {written:?}"
    );
    let named: Vec<&AuditEventRecord> = written
        .iter()
        .filter(|row| format!("{row:?}").contains(&leaving.id.to_string()))
        .collect();
    assert!(
        named.is_empty(),
        "no row of the trail names the deleted account: {named:?}"
    );

    let trail = revocations_on_the_trail(&resources).await;
    assert_eq!(trail.len(), 1, "{trail:?}");
    assert_eq!(
        trail[0].metadata["device_tokens_revoked"], 1,
        "the one login still live; the original was retired by its rotation"
    );
}

/// A service token named after an account carries its id without being its
/// device login. Deleting the account is not refused over that name: the id
/// is rewritten out of it, the rest of the name stays, and the token is
/// still its holder's working credential.
#[tokio::test]
async fn a_service_token_named_after_an_account_outlives_its_deletion() {
    let resources = create_test_server_resources().await.unwrap();
    let (admin, _) = account(&resources, "admin@example.com", UserRole::Admin).await;
    let (athlete, _) = account(&resources, "athlete@example.com", UserRole::User).await;
    let admin_cli = device_login(&resources, &admin.email).await;
    let export = service_token(&resources, &format!("export-{}", athlete.id), true).await;
    assert_eq!(lists_users(&resources, &export).await, 200);
    assert_eq!(token_rows_naming(&resources, athlete.id).await, 1);

    let (status, body) = delete_account(&resources, &admin_cli, athlete.id).await;
    assert_eq!(status, 200, "a token's name refuses no deletion: {body}");
    assert_eq!(
        device_logins_declared_kept(&body),
        Vec::<String>::new(),
        "an account that approved no device login has none kept"
    );

    assert_eq!(token_rows_naming(&resources, athlete.id).await, 0);
    let renamed = format!("export-{DELETED_ACCOUNT_LABEL}");
    let admin_tokens = &resources.common.repos.admin;
    let names: Vec<String> = admin_tokens
        .list_tokens(false)
        .await
        .unwrap()
        .into_iter()
        .map(|token| token.service_name)
        .collect();
    assert!(
        names.contains(&renamed),
        "only the id left the name: {names:?}"
    );
    assert_eq!(
        lists_users(&resources, &export).await,
        200,
        "and the token still works for whoever holds it"
    );
}

/// The `admin_device_logins` entries of the not-deleted list a deletion
/// answered with: what its receipt says it kept of the account's logins.
fn device_logins_declared_kept(answer: &str) -> Vec<String> {
    let answer: Value = serde_json::from_str(answer).unwrap();
    answer["data"]["ledger"]["not_deleted"]
        .as_array()
        .expect("the ledger's not-deleted list")
        .iter()
        .filter_map(Value::as_str)
        .filter(|copy| copy.starts_with("admin_device_logins:"))
        .map(ToOwned::to_owned)
        .collect()
}

/// How many requests the log holds for the token `token_id`.
async fn requests_logged(resources: &Arc<ServerContext>, token_id: &str) -> i64 {
    const SQL: &str = "SELECT COUNT(*) FROM admin_token_usage WHERE admin_token_id = $1";
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(backend) => sqlx::query_scalar(SQL)
            .bind(token_id)
            .fetch_one(backend.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(backend) => sqlx::query_scalar(SQL)
            .bind(token_id)
            .fetch_one(backend.pool())
            .await
            .unwrap(),
    }
}

/// What the admin-token middleware answers `bearer` on the user listing,
/// with the body: the refusal says which check held.
async fn lists_users_answer(resources: &Arc<ServerContext>, bearer: &str) -> (u16, String) {
    let response = AxumTestRequest::get("/admin/users")
        .header("authorization", bearer)
        .send(token_api(resources))
        .await;
    (response.status(), response.text())
}

/// What the gate answers a token whose admin is no admin.
const ADMIN_REFUSAL: &str = "The admin this token acts as is no longer an active admin";

/// A deleted admin's device login is kept as a record and is never a
/// credential again. The row stays with the log of the requests it made,
/// inactive, under a name that names no one and an admin that is no
/// account. That admin is what refuses it for good: reactivated by hand
/// it passes the activity check and is refused on the admin, and rotated
/// from there its replacement, live and freshly signed, carries the same
/// admin and is refused the same way. The admin who rotated it keeps its
/// own login throughout.
#[tokio::test]
#[serial]
async fn a_deleted_admins_device_login_is_kept_and_refused_reactivated_or_rotated() {
    let resources = create_test_server_resources().await.unwrap();
    let (leaving, _) = account(&resources, "leaving@example.com", UserRole::Admin).await;
    let (staying, staying_auth) = account(&resources, "staying@example.com", UserRole::Admin).await;
    let leaving_cli = device_login(&resources, &leaving.email).await;
    let staying_cli = device_login(&resources, &staying.email).await;
    let admin_tokens = &resources.common.repos.admin;
    let login = admin_tokens
        .list_tokens(false)
        .await
        .unwrap()
        .into_iter()
        .find(|token| token.operator_user_id == Some(leaving.id))
        .expect("the login's row")
        .id;
    assert_eq!(lists_users(&resources, &leaving_cli).await, 200);
    let logged = requests_logged(&resources, &login).await;
    assert!(logged > 0, "the login's requests are on its log");

    let (status, body) = delete_account(&resources, &staying_cli, leaving.id).await;
    assert_eq!(status, 200, "{body}");
    // The receipt says the rows were kept, and what their log still holds:
    // its rescan lists the two tables with nothing removed, which alone
    // reads as nothing having been there.
    let declared = device_logins_declared_kept(&body);
    assert_eq!(declared.len(), 1, "{body}");
    assert!(
        declared[0].contains("without its IP address or user agent"),
        "the entry says what the kept request log no longer carries: {declared:?}"
    );

    let kept = admin_tokens
        .get_token_by_id(&login)
        .await
        .unwrap()
        .expect("the row stays");
    assert!(!kept.is_active);
    assert_eq!(kept.service_name, DEVICE_CLI_DELETED_ACCOUNT_SERVICE);
    assert_eq!(kept.operator_user_id, Some(DELETED_ACCOUNT_ADMIN));
    assert_eq!(
        requests_logged(&resources, &login).await,
        logged,
        "the request log stays under the row"
    );
    assert_eq!(token_rows_naming(&resources, leaving.id).await, 0);
    assert_eq!(lists_users(&resources, &leaving_cli).await, 401);

    // Reactivated past the repository: the row is live, and still refused,
    // by the admin check this time.
    write_past_the_repository(
        &resources,
        "UPDATE admin_tokens SET is_active = TRUE \
         WHERE service_name = 'device-cli:deleted-account'",
    )
    .await;
    let reactivated = admin_tokens
        .get_token_by_id(&login)
        .await
        .unwrap()
        .expect("the row stays");
    assert!(reactivated.is_active, "the row is live again");
    let (status, body) = lists_users_answer(&resources, &leaving_cli).await;
    assert_eq!(status, 401, "{body}");
    assert!(
        body.contains(ADMIN_REFUSAL),
        "refused on its admin, not on being inactive: {body}"
    );

    // Rotated from there by another admin: the replacement is live, signed
    // now, and acts as the same admin that is no account.
    let response = AxumTestRequest::post(&format!("/api/admin/tokens/{login}/rotate"))
        .header("authorization", &staying_auth)
        .json(&json!({}))
        .send(console(&resources))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let rotated: Value = response.json();
    let replacement = admin_tokens
        .get_token_by_id(rotated["data"]["token_id"].as_str().unwrap())
        .await
        .unwrap()
        .expect("the replacement's row");
    assert!(replacement.is_active);
    assert_eq!(
        replacement.operator_user_id,
        Some(DELETED_ACCOUNT_ADMIN),
        "a rotation hands the row to no one"
    );
    let replacement_cli = format!("Bearer {}", rotated["data"]["jwt_token"].as_str().unwrap());
    let (status, body) = lists_users_answer(&resources, &replacement_cli).await;
    assert_eq!(status, 401, "{body}");
    assert!(body.contains(ADMIN_REFUSAL), "{body}");

    assert_eq!(token_rows_naming(&resources, leaving.id).await, 0);
    assert_eq!(
        lists_users(&resources, &staying_cli).await,
        200,
        "the admin who rotated it keeps its own login"
    );
}

/// An impersonation token acts as the athlete on the authority of the admin
/// who opened the session. It ends with that authority, however the admin
/// lost it, and the admin beside it who lost nothing keeps theirs.
#[tokio::test]
async fn an_admin_out_of_standing_loses_the_impersonation_it_opened() {
    let resources = create_test_server_resources().await.unwrap();
    let (staying, staying_auth) = account(&resources, "staying@example.com", UserRole::Admin).await;
    let (control_target, _) = account(&resources, "control@example.com", UserRole::User).await;
    let as_control = impersonate(&resources, &staying_auth, control_target.id).await;
    let staying_cli = device_login(&resources, &staying.email).await;

    for (how, nth) in [("demote", 1), ("suspend", 2), ("delete", 3)] {
        let (leaving, leaving_auth) =
            account(&resources, &format!("{how}@example.com"), UserRole::Admin).await;
        let (athlete, _) = account(
            &resources,
            &format!("athlete-{how}@example.com"),
            UserRole::User,
        )
        .await;
        let as_athlete = impersonate(&resources, &leaving_auth, athlete.id).await;
        assert_eq!(
            reads_own_stats(&resources, &as_athlete).await,
            200,
            "{how}: a live impersonation acts as the athlete"
        );

        let (status, body) = match how {
            "demote" => change_admin(&resources, &staying_auth, leaving.id, "demote").await,
            "suspend" => suspend(&resources, &staying_auth, leaving.id).await,
            _ => delete_account(&resources, &staying_cli, leaving.id).await,
        };
        assert_eq!(status, 200, "{how}: {body}");

        assert_eq!(
            reads_own_stats(&resources, &as_athlete).await,
            401,
            "{how}: the impersonation ends with the admin's standing"
        );
        assert!(
            !has_live_impersonation(&resources, athlete.id).await,
            "{how}: no session is left open on the athlete"
        );
        assert_eq!(
            reads_own_stats(&resources, &as_control).await,
            200,
            "{how}: another admin's impersonation is untouched"
        );
        if how != "delete" {
            // A deleted account's sessions go with its row, uncounted.
            let trail = revocations_on_the_trail(&resources).await;
            assert_eq!(trail.len(), nth, "{how}: {trail:?}");
            assert_eq!(trail[0].metadata["impersonations_ended"], 1, "{how}");
            assert_eq!(trail[0].metadata["device_tokens_revoked"], 0, "{how}");
        }
    }
}

/// Run one statement past the repository, as a seeder's SQL or a hand at
/// `psql` writes a row: no guard, no revocation, only what the database
/// itself does to the write.
async fn write_past_the_repository(resources: &Arc<ServerContext>, statement: &'static str) {
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(backend) => {
            sqlx::raw_sql(statement)
                .execute(backend.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(backend) => {
            sqlx::raw_sql(statement)
                .execute(backend.pool())
                .await
                .unwrap();
        }
    }
}

/// The revocation runs in the repository's own write. What holds for a row
/// written past the repository is that both gates read the admin's row: a
/// role changed by a seeder's SQL or by hand refuses the device login and
/// the impersonation on their next request, with nothing revoked.
#[tokio::test]
async fn both_gates_read_the_admins_row_whoever_wrote_it() {
    let resources = create_test_server_resources().await.unwrap();
    let (leaving, leaving_auth) = account(&resources, "leaving@example.com", UserRole::Admin).await;
    account(&resources, "staying@example.com", UserRole::Admin).await;
    let (athlete, _) = account(&resources, "athlete@example.com", UserRole::User).await;
    let leaving_cli = device_login(&resources, &leaving.email).await;
    let as_athlete = impersonate(&resources, &leaving_auth, athlete.id).await;
    assert_eq!(lists_users(&resources, &leaving_cli).await, 200);
    assert_eq!(reads_own_stats(&resources, &as_athlete).await, 200);

    write_past_the_repository(
        &resources,
        "UPDATE users SET role = 'user' WHERE email = 'leaving@example.com'",
    )
    .await;
    assert!(!stored(&resources, leaving.id).await.role.is_admin());

    assert!(
        holds_a_live_device_token(&resources, leaving.id).await
            && has_live_impersonation(&resources, athlete.id).await,
        "this writer revoked nothing: the rows are as they were"
    );
    assert_eq!(
        lists_users(&resources, &leaving_cli).await,
        401,
        "the token's admin is no longer one"
    );
    assert_eq!(
        reads_own_stats(&resources, &as_athlete).await,
        401,
        "the session's admin is no longer one"
    );

    // Nothing was revoked, so the rows answer again with the role: the gate
    // is a read of the row, and the revocation is what makes a loss final.
    write_past_the_repository(
        &resources,
        "UPDATE users SET role = 'admin' WHERE email = 'leaving@example.com'",
    )
    .await;
    assert_eq!(lists_users(&resources, &leaving_cli).await, 200);
    assert_eq!(reads_own_stats(&resources, &as_athlete).await, 200);
}

/// The repository's write is where the loss is made final, so it holds for
/// every caller: a role or a status changed straight on the repository, as
/// `pierre-cli` on the database does, deactivates the device login and ends
/// the impersonation in the transaction that changes the row, reports both,
/// and a later promotion or approval brings neither back.
#[tokio::test]
async fn the_repository_write_withdraws_the_credentials_whoever_calls_it() {
    for how in ["demote", "suspend"] {
        let resources = create_test_server_resources().await.unwrap();
        let (leaving, leaving_auth) =
            account(&resources, "leaving@example.com", UserRole::Admin).await;
        account(&resources, "staying@example.com", UserRole::Admin).await;
        let (athlete, _) = account(&resources, "athlete@example.com", UserRole::User).await;
        let leaving_cli = device_login(&resources, &leaving.email).await;
        let as_athlete = impersonate(&resources, &leaving_auth, athlete.id).await;

        let users = &resources.common.repos.users;
        let write = if how == "demote" {
            users.set_admin_status(leaving.id, false).await.unwrap()
        } else {
            users
                .update_status(leaving.id, UserStatus::Suspended, None)
                .await
                .unwrap()
        };
        assert_eq!(write.withdrawn.device_tokens.len(), 1, "{how}");
        assert_eq!(write.withdrawn.impersonations, 1, "{how}");
        assert!(
            !holds_a_live_device_token(&resources, leaving.id).await
                && !has_live_impersonation(&resources, athlete.id).await,
            "{how}: the write itself withdrew both"
        );

        let back = if how == "demote" {
            users.set_admin_status(leaving.id, true).await.unwrap()
        } else {
            users
                .update_status(leaving.id, UserStatus::Active, None)
                .await
                .unwrap()
        };
        assert!(back.user.is_active_admin(), "{how}: an admin again");
        assert!(back.withdrawn.is_empty(), "{how}");
        assert_eq!(
            lists_users(&resources, &leaving_cli).await,
            401,
            "{how}: the device login stays revoked"
        );
        assert_eq!(
            reads_own_stats(&resources, &as_athlete).await,
            401,
            "{how}: the impersonation stays ended"
        );
    }
}

/// An admin is never impersonated, and that is refused when a session
/// starts. A user promoted while a session is open on the account would be
/// acted as with every power, under someone else's session: the promotion
/// ends those sessions, and the impersonation token is refused.
#[tokio::test]
async fn a_promotion_ends_the_impersonations_open_on_the_new_admin() {
    let resources = create_test_server_resources().await.unwrap();
    let (_, acting_auth) = account(&resources, "acting@example.com", UserRole::Admin).await;
    let (rising, _) = account(&resources, "rising@example.com", UserRole::User).await;
    let (bystander, _) = account(&resources, "bystander@example.com", UserRole::User).await;
    let (_, other_auth) = account(&resources, "other@example.com", UserRole::Admin).await;
    let as_rising = impersonate(&resources, &acting_auth, rising.id).await;
    let as_bystander = impersonate(&resources, &other_auth, bystander.id).await;
    assert_eq!(reads_own_stats(&resources, &as_rising).await, 200);

    let (status, body) = change_admin(&resources, &other_auth, rising.id, "promote").await;
    assert_eq!(status, 200, "{body}");

    assert!(
        !has_live_impersonation(&resources, rising.id).await,
        "the session open on the promoted account is ended"
    );
    assert_eq!(
        reads_own_stats(&resources, &as_rising).await,
        401,
        "nobody acts as the new admin"
    );
    assert!(
        has_live_impersonation(&resources, bystander.id).await,
        "an impersonation of another user is untouched"
    );
    assert_eq!(reads_own_stats(&resources, &as_bystander).await, 200);
}

/// The same rule at the gate, for a role written past the repository: the
/// session row is still live, and the token is refused because its target
/// now holds the admin role.
#[tokio::test]
async fn an_impersonation_is_refused_once_its_target_holds_the_admin_role() {
    let resources = create_test_server_resources().await.unwrap();
    let (_, acting_auth) = account(&resources, "acting@example.com", UserRole::Admin).await;
    let (rising, _) = account(&resources, "rising@example.com", UserRole::User).await;
    let as_rising = impersonate(&resources, &acting_auth, rising.id).await;
    assert_eq!(reads_own_stats(&resources, &as_rising).await, 200);

    write_past_the_repository(
        &resources,
        "UPDATE users SET role = 'admin' WHERE email = 'rising@example.com'",
    )
    .await;

    assert!(
        has_live_impersonation(&resources, rising.id).await,
        "this writer ended nothing"
    );
    assert_eq!(
        reads_own_stats(&resources, &as_rising).await,
        401,
        "the gate reads the target's row"
    );
}

/// Set `target`'s role with the admin token `bearer`, as
/// `pierre-cli user set --role` does (carnet#1575), through the app the
/// composition root serves, where every admin write is recorded.
async fn set_role(
    resources: &Arc<ServerContext>,
    bearer: &str,
    target: Uuid,
    role: &str,
) -> (u16, String) {
    let response = AxumTestRequest::post(&format!("/admin/users/{target}/role"))
        .header("authorization", bearer)
        .json(&json!({ "role": role }))
        .send(ProviderToolRouter::build_http_app(resources))
        .await;
    (response.status(), response.text())
}

/// The rows the trail holds of role changes made over the token API.
async fn role_changes_on_the_trail(resources: &Arc<ServerContext>) -> Vec<AuditEventRecord> {
    let filter = AuditEventFilter {
        action_prefix: Some("user.role".to_owned()),
        ..AuditEventFilter::default()
    };
    resources
        .common
        .repos
        .admin_audit
        .list(&filter, 50)
        .await
        .unwrap()
}

/// A deployed environment's admins are managed from `pierre-cli`: a device
/// login, which acts as the admin who approved it, makes a user an admin and
/// makes it a user again, and the trail names that admin each time.
#[tokio::test]
async fn a_device_login_promotes_a_user_and_demotes_it_again() {
    let resources = create_test_server_resources().await.unwrap();
    let (acting, _) = account(&resources, "acting-cli@example.com", UserRole::Admin).await;
    let (rising, _) = account(&resources, "rising-cli@example.com", UserRole::User).await;
    let cli = device_login(&resources, &acting.email).await;

    let (status, body) = set_role(&resources, &cli, rising.id, "admin").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(stored(&resources, rising.id).await.role, UserRole::Admin);

    let (status, body) = set_role(&resources, &cli, rising.id, "user").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(stored(&resources, rising.id).await.role, UserRole::User);

    let trail = role_changes_on_the_trail(&resources).await;
    assert_eq!(trail.len(), 2, "one row per change: {trail:?}");
    for row in &trail {
        assert_eq!(row.outcome, "success");
        assert_eq!(row.source, "cli");
        assert_eq!(
            row.actor_user_id.as_deref(),
            Some(acting.id.to_string().as_str()),
            "the row names the admin behind the device login"
        );
    }
    assert_eq!(trail[0].metadata["after"]["role"], "user", "{trail:?}");
    assert_eq!(trail[1].metadata["after"]["role"], "admin", "{trail:?}");
}

/// A role change hands over or takes away every power, so a service token
/// makes none: without the super-admin flag it lacks the rank, and with it
/// the token still acts as no admin the change could be checked against.
/// Nothing a refused request asked for is written.
#[tokio::test]
async fn a_service_token_changes_no_role() {
    let resources = create_test_server_resources().await.unwrap();
    let (rising, _) = account(&resources, "service-target@example.com", UserRole::User).await;
    let manage_users = service_token(&resources, "role-manage-users", false).await;
    let super_service = service_token(&resources, "role-super-service", true).await;

    let (status, body) = set_role(&resources, &manage_users, rising.id, "admin").await;
    assert_eq!(status, 403, "{body}");
    assert!(body.contains("requires a super-admin token"), "{body}");

    let (status, body) = set_role(&resources, &super_service, rising.id, "admin").await;
    assert_eq!(status, 403, "{body}");
    assert!(body.contains("token that acts as an admin"), "{body}");

    assert_eq!(stored(&resources, rising.id).await.role, UserRole::User);
}

/// An admin does not demote itself over the token API either, whoever else
/// is an admin: a second admin is present, so the last-active-admin guard is
/// not what answers. An unknown role is refused before anything is read.
#[tokio::test]
async fn a_device_login_does_not_demote_its_own_admin() {
    let resources = create_test_server_resources().await.unwrap();
    let (acting, _) = account(&resources, "self-cli@example.com", UserRole::Admin).await;
    account(&resources, "other-cli@example.com", UserRole::Admin).await;
    let cli = device_login(&resources, &acting.email).await;

    let (status, body) = set_role(&resources, &cli, acting.id, "user").await;
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("Cannot demote yourself"), "{body}");
    assert_eq!(stored(&resources, acting.id).await.role, UserRole::Admin);

    let (status, body) = set_role(&resources, &cli, acting.id, "owner").await;
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("expected admin or user"), "{body}");
}

/// What a promotion of an account that is not active is told (carnet#1589).
const APPROVE_FIRST: &str = "Approve the account first, then promote it.";

/// An admin is approved first, then promoted. A pending account and a
/// suspended one are refused the role on the console, over the token API
/// and from the host (`pierre-cli user promote` makes the same write), each
/// told its status and what to do first, and none of them holds the role
/// afterwards. Approved, the same account is promoted.
#[tokio::test]
async fn an_account_that_is_not_active_is_not_promoted() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let (acting, session) = account(&resources, "acting@example.com", UserRole::Admin).await;
    let cli = device_login(&resources, &acting.email).await;
    let waiting = pending_account(&resources, "waiting@example.com", UserRole::User).await;
    let (barred, _) = account(&resources, "barred@example.com", UserRole::User).await;
    let (status, body) = suspend(&resources, &session, barred.id).await;
    assert_eq!(status, 200, "{body}");

    for (target, standing) in [(waiting.id, "pending"), (barred.id, "suspended")] {
        let told = format!("this one is {standing}");

        let (status, body) = change_admin(&resources, &session, target, "promote").await;
        assert_eq!(
            status, 400,
            "the console promotes no {standing} account: {body}"
        );
        assert!(
            body.contains(&told) && body.contains(APPROVE_FIRST),
            "{body}"
        );

        let (status, body) = set_role(&resources, &cli, target, "admin").await;
        assert_eq!(status, 400, "nor does the token API: {body}");
        assert!(
            body.contains(&told) && body.contains(APPROVE_FIRST),
            "{body}"
        );

        let on_the_host = admin_ops::grant_admin_role(repos, &stored(&resources, target).await)
            .await
            .expect_err("nor does the host");
        assert_eq!(on_the_host.code, ErrorCode::InvalidInput, "{standing}");
        assert!(
            on_the_host.to_string().contains(APPROVE_FIRST),
            "{on_the_host}"
        );

        assert_eq!(
            stored(&resources, target).await.role,
            UserRole::User,
            "a refused promotion wrote nothing: {standing}"
        );
    }
    assert!(
        role_changes_on_the_trail(&resources).await.is_empty(),
        "and put no role change on the trail"
    );

    // Approved first, then promoted.
    let (status, body) = approve(&resources, &session, waiting.id).await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = change_admin(&resources, &session, waiting.id, "promote").await;
    assert_eq!(status, 200, "an approved account is promoted: {body}");
    assert!(stored(&resources, waiting.id).await.is_active_admin());
}
