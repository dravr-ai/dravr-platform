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

mod common;

use common::{create_test_server_resources, create_test_user_with_email};
use pierre_core::errors::ErrorCode;
use pierre_core::permissions::impersonation::ImpersonationSession;
use uuid::Uuid;

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
