// ABOUTME: Ending an impersonation session revokes its token at once, not when the hour runs out
// ABOUTME: Drives the auth middleware with a real impersonation JWT before and after the session ends

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

use std::sync::Arc;

use common::{create_test_server_resources, create_test_user_with_email};
use helpers::axum_test::AxumTestRequest;
use pierre_core::errors::ErrorCode;
use pierre_core::models::User;
use pierre_core::permissions::impersonation::ImpersonationSession;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::tenants::TenantRoutes;
use pierre_routes_auth::AuthRoutes;
use serde_json::{json, Value};
use uuid::Uuid;

/// A live impersonation of a fresh target by a fresh operator: the session,
/// the target, and the bearer header carrying its token.
async fn live_impersonation(
    resources: &Arc<ServerContext>,
    label: &str,
) -> (ImpersonationSession, User, String) {
    let database = &resources.agent.database;
    let (operator_id, _) =
        create_test_user_with_email(database, &format!("operator-{label}@example.com"))
            .await
            .unwrap();
    let (_, target) = create_test_user_with_email(database, &format!("target-{label}@example.com"))
        .await
        .unwrap();
    let session = ImpersonationSession::new(operator_id, target.id, Some("support".to_owned()));
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
            operator_id,
            &session.id,
            &resources.auth.jwks_manager,
            None,
        )
        .unwrap();
    (session, target, format!("Bearer {token}"))
}

/// Restoring a session from an impersonation token would mint the operator
/// an ordinary 24-hour session as the athlete, outside the impersonation's
/// revocation and audit: it is refused, and no cookie is set (carnet#909).
#[tokio::test]
async fn an_impersonation_token_is_not_exchanged_for_a_session() {
    let resources = create_test_server_resources().await.unwrap();
    let (_, _, bearer) = live_impersonation(&resources, "restore").await;

    let response = AxumTestRequest::get("/api/auth/session")
        .header("authorization", &bearer)
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    assert_eq!(response.status(), 403, "{}", response.body_text());
    assert!(response.header("set-cookie").is_none());
}

/// A tenant switch under impersonation stays the impersonation: the new
/// token names the same operator and session and expires no later, so ending
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
    let (operator_id, _) = create_test_user_with_email(database, "operator@example.com")
        .await
        .unwrap();
    let (_, target) = create_test_user_with_email(database, "target@example.com")
        .await
        .unwrap();
    let repos = &resources.common.repos;

    let session = ImpersonationSession::new(operator_id, target.id, Some("support".to_owned()));
    repos.impersonation.create_session(&session).await.unwrap();
    let token = resources
        .auth
        .auth_manager
        .generate_impersonation_token(
            &target,
            operator_id,
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
    let (operator_id, _) = create_test_user_with_email(database, "operator2@example.com")
        .await
        .unwrap();
    let (other_id, _) = create_test_user_with_email(database, "other@example.com")
        .await
        .unwrap();
    let (_, target) = create_test_user_with_email(database, "target2@example.com")
        .await
        .unwrap();

    // A live session for someone else, and a token naming it for `target`.
    let session = ImpersonationSession::new(operator_id, other_id, None);
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
            operator_id,
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
            operator_id,
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
