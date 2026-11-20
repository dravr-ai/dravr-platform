// ABOUTME: The user a sign-in or session restore hands back says what a Pending account waits on (carnet#1650)
// ABOUTME: review vs confirm_email re-derived from the sign-up policy, so the waiting screen is right on any tab or device

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `UserInfo.activation` carries the value registration answered with:
//! `active`, `confirm_email` (a proven address activates the account) or
//! `review` (an admin decides). Nothing is stored about it, so it is
//! re-derived from the sign-up policy and the account's state, and a sign-in
//! on another device reads the same thing the sign-up tab was told.

use crate::common;
use crate::held_group_invite_test::{athlete, auth_routes, bearer, body_of};
use crate::helpers;

use common::create_test_server_resources;
use helpers::axum_test::AxumTestRequest;
use pierre_auth::dto::auth::LoginRequest;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_services::signup_policy::{set_signup_policy, SignupPolicyUpdate};
use secrecy::SecretString;
use serde_json::{json, Value};
use uuid::Uuid;

const PASSWORD: &str = "securePassword123";

/// Register an account and sign it in as the waiting screen does, returning
/// the user the sign-in hands back.
async fn signed_up_and_in(resources: &ServerContext) -> Value {
    let email = format!("waiting-{}@activation.test", Uuid::new_v4());
    let response = AxumTestRequest::post("/api/auth/register")
        .json(&json!({ "email": email, "password": PASSWORD }))
        .send(auth_routes(resources))
        .await;
    assert_eq!(response.status(), 201, "{}", response.body_text());
    let login = resources
        .auth_routes_context()
        .account_service()
        .login(LoginRequest {
            email,
            password: SecretString::from(PASSWORD),
            timezone: None,
        })
        .await
        .unwrap();
    serde_json::to_value(login.user).unwrap()
}

#[tokio::test]
async fn a_pending_sign_in_says_review_when_sign_up_is_reviewed() {
    let resources = create_test_server_resources().await.unwrap();
    set_signup_policy(
        &resources.agent.database,
        SignupPolicyUpdate {
            activate_on_verified_email: Some(false),
            ..SignupPolicyUpdate::default()
        },
    )
    .await
    .unwrap();
    let user = signed_up_and_in(&resources).await;
    assert_eq!(user["user_status"], "pending", "{user}");
    assert_eq!(user["activation"], "review", "{user}");
}

#[tokio::test]
async fn a_pending_sign_in_says_confirm_email_when_a_proven_address_activates() {
    let resources = create_test_server_resources().await.unwrap();
    set_signup_policy(
        &resources.agent.database,
        SignupPolicyUpdate {
            activate_on_verified_email: Some(true),
            ..SignupPolicyUpdate::default()
        },
    )
    .await
    .unwrap();
    let user = signed_up_and_in(&resources).await;
    assert_eq!(user["user_status"], "pending", "{user}");
    assert_eq!(user["activation"], "confirm_email", "{user}");
}

#[tokio::test]
async fn an_active_session_says_active() {
    let resources = create_test_server_resources().await.unwrap();
    let (user, _) = athlete(&resources, "already-in").await;
    let response = AxumTestRequest::get("/api/auth/session")
        .header("authorization", &bearer(&resources, &user).await)
        .send(auth_routes(&resources))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let body = body_of(&response.body_text());
    assert_eq!(body["user"]["user_status"], "active", "{body}");
    assert_eq!(body["user"]["activation"], "active", "{body}");
}
