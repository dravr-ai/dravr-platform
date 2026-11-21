// ABOUTME: Ending an impersonation session revokes its token at once, not when the hour runs out
// ABOUTME: Drives the auth middleware with a real impersonation JWT; the consent screen never takes one as a session

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! An impersonation token is a one-hour JWT naming the target user. Ending
//! the session used to flip its row and nothing else: the token kept acting
//! as the target for the rest of its hour, and nothing on the request path
//! read the session it named. The middleware now requires the session to be
//! live and to belong to the token's subject.

use crate::common;
use crate::helpers;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use common::{create_test_server_resources, create_test_user_with_email};
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use helpers::first_party_sign_in::{
    authorize_uri, first_party_router, fresh_state, is_redirect, location, FirstPartyClient, Pkce,
    DEFAULT_PEER,
};
use helpers::impersonation::{assert_refusal_audited, assert_support_refused};
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::first_party::WEB_CLIENT_ID;
use pierre_auth::oauth2_server::models::ClientRegistrationRequest;
use pierre_auth::security::cookies::auth_cookie_name;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::errors::ErrorCode;
use pierre_core::models::{User, UserStatus};
use pierre_core::permissions::impersonation::ImpersonationSession;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::tenants::TenantRoutes;
use pierre_routes_auth::AuthRoutes;
use pierre_services::signup_codes::{self, MintRequest};
use serde_json::{json, Value};
use uuid::Uuid;

/// A fresh admin, the only account an impersonation token acts for: the
/// middleware reads the opener's row and refuses one that is not an active
/// admin, so a plain user here would be refused for that and not for what a
/// test set out to show.
async fn admin(resources: &Arc<ServerContext>, email: &str) -> Uuid {
    let (admin_id, _) = create_test_user_with_email(&resources.agent.database, email)
        .await
        .unwrap();
    resources
        .common
        .repos
        .users
        .set_admin_status(admin_id, true)
        .await
        .unwrap();
    admin_id
}

/// A live impersonation of a fresh target by a fresh admin: the session,
/// the target, and the bearer header carrying its token.
async fn live_impersonation(
    resources: &Arc<ServerContext>,
    label: &str,
) -> (ImpersonationSession, User, String) {
    let database = &resources.agent.database;
    let admin_id = admin(resources, &format!("admin-{label}@example.com")).await;
    let (_, target) = create_test_user_with_email(database, &format!("target-{label}@example.com"))
        .await
        .unwrap();
    let session = ImpersonationSession::new(admin_id, target.id, Some("support".to_owned()));
    resources
        .common
        .repos
        .impersonation
        .create_session(&session)
        .await
        .unwrap();
    let token = resources
        .auth
        .auth_manager
        .generate_impersonation_token(
            &target,
            admin_id,
            &session.id,
            &resources.auth.jwks_manager,
            None,
        )
        .unwrap();
    (session, target, format!("Bearer {token}"))
}

/// Restoring a session from an impersonation token would mint the admin
/// an ordinary 24-hour session as the athlete, outside the impersonation's
/// revocation and audit: it is refused, and no cookie is set (carnet#909).
#[tokio::test]
async fn an_impersonation_token_is_not_exchanged_for_a_session() {
    let resources = create_test_server_resources().await.unwrap();
    let (_, target, bearer) = live_impersonation(&resources, "restore").await;

    let response = AxumTestRequest::get("/api/auth/session")
        .header("authorization", &bearer)
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    assert_eq!(response.status(), 403, "{}", response.body_text());
    assert!(response.header("set-cookie").is_none());
    assert_refusal_audited(&resources, target.id, "restore a session").await;
}

/// A tenant switch under impersonation stays the impersonation: the new
/// token names the same admin and session and expires no later, so ending
/// the impersonation revokes it too (carnet#909).
#[tokio::test]
async fn a_tenant_switch_under_impersonation_keeps_the_impersonation() {
    let resources = create_test_server_resources().await.unwrap();
    let (session, target, bearer) = live_impersonation(&resources, "switch").await;
    let auth = &resources.auth;
    let original = auth
        .auth_manager
        .validate_token_detailed(bearer.trim_start_matches("Bearer "), &auth.jwks_manager)
        .unwrap();
    let tenant = resources
        .common
        .repos
        .tenants
        .list_for_user(target.id)
        .await
        .unwrap()
        .first()
        .expect("the target owns a tenant")
        .id;

    let response = AxumTestRequest::post("/tenants/switch")
        .header("authorization", &bearer)
        .json(&json!({ "tenant_id": tenant.to_string() }))
        .send(TenantRoutes::routes(Arc::clone(&resources)))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let body: Value = response.json();
    let switched = body["token"].as_str().expect("a token").to_owned();
    let claims = auth
        .auth_manager
        .validate_token_detailed(&switched, &auth.jwks_manager)
        .unwrap();
    assert_eq!(claims.sub, target.id.to_string());
    assert_eq!(claims.active_tenant_id, Some(tenant.to_string()));
    assert_eq!(claims.impersonator_id, original.impersonator_id);
    assert_eq!(
        claims.impersonation_session_id.as_deref(),
        Some(session.id.as_str())
    );
    assert!(
        claims.exp <= original.exp,
        "the switch never extends the hour"
    );
    let expires_in = body["expires_in"].as_u64().expect("an expiry");
    assert!(
        expires_in <= 3600,
        "the response states the impersonation's remaining hour, not a day: {expires_in}"
    );

    resources
        .common
        .repos
        .impersonation
        .end_session(&session.id)
        .await
        .unwrap();
    let refused = auth
        .auth_middleware
        .authenticate_request(Some(&format!("Bearer {switched}")))
        .await
        .expect_err("ending the impersonation revokes the switched token");
    assert_eq!(refused.code, ErrorCode::AuthInvalid);
}

#[tokio::test]
async fn an_ended_impersonation_session_revokes_its_token() {
    let resources = create_test_server_resources().await.unwrap();
    let database = &resources.agent.database;
    let admin_id = admin(&resources, "admin@example.com").await;
    let (_, target) = create_test_user_with_email(database, "target@example.com")
        .await
        .unwrap();
    let repos = &resources.common.repos;

    let session = ImpersonationSession::new(admin_id, target.id, Some("support".to_owned()));
    repos.impersonation.create_session(&session).await.unwrap();
    let token = resources
        .auth
        .auth_manager
        .generate_impersonation_token(
            &target,
            admin_id,
            &session.id,
            &resources.auth.jwks_manager,
            None,
        )
        .unwrap();
    let bearer = format!("Bearer {token}");
    let middleware = &resources.auth.auth_middleware;

    let live = middleware
        .authenticate_request(Some(&bearer))
        .await
        .expect("a live session's token authenticates");
    assert_eq!(live.user_id, target.id);

    repos.impersonation.end_session(&session.id).await.unwrap();

    let refused = middleware
        .authenticate_request(Some(&bearer))
        .await
        .expect_err("an ended session's token must be refused");
    assert_eq!(refused.code, ErrorCode::AuthInvalid);
    assert_eq!(refused.http_status(), 401);
}

#[tokio::test]
async fn an_impersonation_token_for_another_sessions_target_is_refused() {
    let resources = create_test_server_resources().await.unwrap();
    let database = &resources.agent.database;
    let admin_id = admin(&resources, "admin2@example.com").await;
    let (other_id, _) = create_test_user_with_email(database, "other@example.com")
        .await
        .unwrap();
    let (_, target) = create_test_user_with_email(database, "target2@example.com")
        .await
        .unwrap();

    // A live session for someone else, and a token naming it for `target`.
    let session = ImpersonationSession::new(admin_id, other_id, None);
    resources
        .common
        .repos
        .impersonation
        .create_session(&session)
        .await
        .unwrap();
    let token = resources
        .auth
        .auth_manager
        .generate_impersonation_token(
            &target,
            admin_id,
            &session.id,
            &resources.auth.jwks_manager,
            None,
        )
        .unwrap();

    let refused = resources
        .auth
        .auth_middleware
        .authenticate_request(Some(&format!("Bearer {token}")))
        .await
        .expect_err("a session is only good for its own target");
    assert_eq!(refused.code, ErrorCode::AuthInvalid);

    // And a session id nobody created.
    let unknown = resources
        .auth
        .auth_manager
        .generate_impersonation_token(
            &target,
            admin_id,
            &Uuid::new_v4().to_string(),
            &resources.auth.jwks_manager,
            None,
        )
        .unwrap();
    let refused = resources
        .auth
        .auth_middleware
        .authenticate_request(Some(&format!("Bearer {unknown}")))
        .await
        .expect_err("an unknown session revokes its token");
    assert_eq!(refused.code, ErrorCode::AuthInvalid);
}

// ============================================================================
// The consent screen: an impersonation is not the athlete's session
// ============================================================================

/// `GET /oauth2/authorize` for `client_id` at `redirect_uri`, carrying `jwt`
/// in the web app's session cookie, the bridge the consent screen accepts.
async fn authorize_with_web_cookie(
    resources: &Arc<ServerContext>,
    client_id: &str,
    redirect_uri: &str,
    jwt: &str,
) -> AxumTestResponse {
    let pkce = Pkce::generate();
    AxumTestRequest::get(&authorize_uri(
        client_id,
        redirect_uri,
        &pkce.challenge,
        &fresh_state(),
    ))
    .header("cookie", &format!("{}={jwt}", auth_cookie_name()))
    .send(first_party_router(resources, DEFAULT_PEER))
    .await
}

/// The impersonation token behind `bearer`.
fn raw(bearer: &str) -> &str {
    bearer.strip_prefix("Bearer ").unwrap()
}

/// The target's own session, as the web app holds it.
fn own_session(resources: &Arc<ServerContext>, target: &User) -> String {
    resources
        .auth
        .auth_manager
        .generate_token(target, &resources.auth.jwks_manager)
        .unwrap()
}

/// The validator every session-taking surface shares refuses an
/// impersonation token, while the middleware's own validator still accepts
/// it: the impersonation keeps working on the routes it is for.
#[tokio::test]
async fn an_impersonation_token_does_not_validate_as_a_session() {
    let resources = create_test_server_resources().await.unwrap();
    let (_, target, bearer) = live_impersonation(&resources, "validate").await;
    let auth = &resources.auth;

    let audit = resources.common.repos.admin_audit.as_ref();
    let refused = auth
        .auth_manager
        .validate_session_token(
            raw(&bearer),
            &auth.jwks_manager,
            audit,
            "validate a session",
        )
        .await
        .expect_err("an impersonation token is not a session");
    assert_eq!(
        refused.code,
        ErrorCode::AuthInvalid,
        "an impersonation token is refused as an invalid credential"
    );
    assert_refusal_audited(&resources, target.id, "validate a session").await;

    let claims = auth
        .auth_manager
        .validate_token(raw(&bearer), &auth.jwks_manager)
        .expect("the token itself is valid");
    assert_eq!(
        claims.sub,
        target.id.to_string(),
        "the impersonation token names its target"
    );
    auth.auth_manager
        .validate_session_token(
            &own_session(&resources, &target),
            &auth.jwks_manager,
            audit,
            "validate a session",
        )
        .await
        .expect("the target's own session is one");
}

/// Placed in the web app's cookie, an impersonation token would sign the
/// admin in to Dravr's own app as the athlete: the consent skip hands
/// `dravr-web` a code at once, redeemed for an ordinary session that outlives
/// the hour and the impersonation's end (carnet#1560). It is sent to log in
/// instead, and the athlete's own session still gets its code.
#[tokio::test]
async fn an_impersonation_token_cannot_sign_in_to_a_first_party_app() {
    let resources = create_test_server_resources().await.unwrap();
    let (_, target, bearer) = live_impersonation(&resources, "first-party").await;
    let redirect_uri = FirstPartyClient::Web.redirect_uri(&resources.common.config.oauth2_server);

    let response =
        authorize_with_web_cookie(&resources, WEB_CLIENT_ID, &redirect_uri, raw(&bearer)).await;
    let sent_to = location(&response);
    assert!(
        sent_to.starts_with("/oauth2/login") && !sent_to.contains("code="),
        "no session: the admin is sent to log in, got {sent_to}"
    );
    assert_refusal_audited(&resources, target.id, "sign in to the OAuth consent screen").await;

    let response = authorize_with_web_cookie(
        &resources,
        WEB_CLIENT_ID,
        &redirect_uri,
        &own_session(&resources, &target),
    )
    .await;
    let sent_to = location(&response);
    assert!(
        sent_to.starts_with(&redirect_uri) && sent_to.contains("code="),
        "the athlete's own session gets its code: {sent_to}"
    );
}

/// Nor can it approve a third-party application in the athlete's name,
/// even once the impersonation has ended.
#[tokio::test]
async fn an_impersonation_token_cannot_approve_a_third_party_application() {
    let resources = create_test_server_resources().await.unwrap();
    let (session, target, bearer) = live_impersonation(&resources, "third-party").await;
    let redirect_uri = "https://app.example.com/callback";
    let client = ClientRegistrationManager::new(resources.common.repos.oauth2_server.clone())
        .register_client(
            ClientRegistrationRequest {
                redirect_uris: vec![redirect_uri.to_owned()],
                client_name: Some("Third-party application".to_owned()),
                client_uri: None,
                grant_types: None,
                response_types: None,
                scope: None,
            },
            MAX_PENDING_REGISTRATIONS,
        )
        .await
        .unwrap();

    let live =
        authorize_with_web_cookie(&resources, &client.client_id, redirect_uri, raw(&bearer)).await;
    assert!(
        location(&live).starts_with("/oauth2/login"),
        "a live impersonation is no session"
    );
    resources
        .common
        .repos
        .impersonation
        .end_session(&session.id)
        .await
        .unwrap();
    let ended =
        authorize_with_web_cookie(&resources, &client.client_id, redirect_uri, raw(&bearer)).await;
    assert!(
        location(&ended).starts_with("/oauth2/login"),
        "nor is an ended one"
    );

    let own = authorize_with_web_cookie(
        &resources,
        &client.client_id,
        redirect_uri,
        &own_session(&resources, &target),
    )
    .await;
    assert!(
        !is_redirect(&own),
        "the athlete's own session is not sent away from the consent screen, got {:?}",
        own.header("location")
    );
    assert_eq!(
        own.status(),
        200,
        "the athlete's own session reaches the consent screen"
    );
}

/// The pending-account routes take a session too: an impersonation token is
/// refused there with the `403` every support-session write answers (a `401`
/// signs the clients out), and the refusal written to the admin audit trail
/// (carnet#1634). The athlete's own session is still served.
#[tokio::test]
async fn an_impersonation_token_clears_no_import_tombstone() {
    let resources = create_test_server_resources().await.unwrap();
    let (_, target, bearer) = live_impersonation(&resources, "tombstone").await;
    // A verified address, so the athlete's own clear is served in full.
    resources
        .common
        .repos
        .email_verification
        .mark_verified(target.id)
        .await
        .unwrap();
    let clear = |authorization: String| {
        AxumTestRequest::post("/api/user/import-tombstones/clear")
            .header("authorization", &authorization)
            .json(&json!({}))
            .send(AuthRoutes::routes(resources.auth_routes_context()))
    };

    let refused = clear(bearer.clone()).await;
    assert_support_refused(
        refused.status(),
        &refused.body_text(),
        "clear an import tombstone",
    );
    assert_refusal_audited(&resources, target.id, "clear an import tombstone").await;

    let served = clear(format!("Bearer {}", own_session(&resources, &target))).await;
    assert_eq!(served.status(), 200, "{}", served.body_text());
}

/// Redeeming a signup code is a pending-account route too: an impersonation
/// token is refused with a `403` and the refusal audited (carnet#1634); the
/// athlete's own session redeems.
#[tokio::test]
async fn an_impersonation_token_redeems_no_signup_code() {
    let resources = create_test_server_resources().await.unwrap();
    let (_, target, bearer) = live_impersonation(&resources, "signup-code").await;
    resources
        .common
        .repos
        .users
        .update_status(target.id, UserStatus::Pending, None)
        .await
        .unwrap();
    let code = signup_codes::mint(
        &resources.common.repos,
        None,
        &MintRequest {
            note: Some("support cohort".to_owned()),
            prefix: Some("beta".to_owned()),
            max_uses: None,
            expires_in_days: Some(7),
        },
    )
    .await
    .unwrap()
    .code;
    let redeem = |authorization: String| {
        AxumTestRequest::post("/api/user/signup-code/redeem")
            .header("authorization", &authorization)
            .json(&json!({ "code": code }))
            .send(
                AuthRoutes::routes(resources.auth_routes_context()).layer(MockConnectInfo(
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(198, 18, 7, 7)), 40_000),
                )),
            )
    };

    let refused = redeem(bearer.clone()).await;
    assert_support_refused(
        refused.status(),
        &refused.body_text(),
        "redeem a signup code",
    );
    assert_refusal_audited(&resources, target.id, "redeem a signup code").await;

    let served = redeem(format!("Bearer {}", own_session(&resources, &target))).await;
    assert_eq!(served.status(), 200, "{}", served.body_text());
}
