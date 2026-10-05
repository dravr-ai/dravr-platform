// ABOUTME: Emails are case-insensitive: one account per address whatever casing it is registered or signed in with
// ABOUTME: Pins the stored form, sign-in in another casing, and the refusal of a second account in another casing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `Jane@Case.Test` and `jane@case.test` name one person. The server used to
//! store an address as typed and compare it exactly, so a registration in one
//! casing could not sign in in another, and a second registration in another
//! casing opened a second account for the same person. Every write now stores
//! the trimmed, lowercase form and every lookup compares case aside, through
//! the real `/api/auth/register` and `/oauth/token` handlers and through the
//! repository every other account-creating path goes through.

mod common;
mod helpers;

use std::sync::Arc;

use common::create_test_server_resources;
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::User;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_auth::AuthRoutes;
use serde_json::{json, Value};
use uuid::Uuid;

const PASSWORD: &str = "securePassword123";

/// POST the real registration handler: the status and the body.
async fn register(res: &Arc<ServerContext>, email: &str) -> (u16, Value) {
    let resp = AxumTestRequest::post("/api/auth/register")
        .json(&json!({
            "email": email,
            "password": PASSWORD,
            "display_name": "Case Tester"
        }))
        .send(AuthRoutes::routes(res.auth_routes_context()))
        .await;
    (resp.status(), resp.json())
}

/// Sign in through the password grant: the status and the body.
async fn sign_in(res: &Arc<ServerContext>, email: &str) -> (u16, Value) {
    let resp = AxumTestRequest::post("/oauth/token")
        .form(&[
            ("grant_type", "password"),
            ("client_id", "dravr-web"),
            ("username", email),
            ("password", PASSWORD),
        ])
        .send(AuthRoutes::routes(res.auth_routes_context()))
        .await;
    (resp.status(), resp.json())
}

/// An account registered as `Jane@Case.Test` is stored as `jane@case.test`
/// and signs in whatever casing, or surrounding whitespace, the address is
/// typed with.
#[tokio::test]
async fn an_account_registered_in_one_casing_signs_in_in_any_other() {
    let res = create_test_server_resources().await.unwrap();

    let (status, body) = register(&res, "Jane@Case.Test").await;
    assert_eq!(status, 201, "{body}");
    let user_id = body["user_id"].as_str().unwrap().to_owned();

    let stored = res
        .common
        .repos
        .users
        .get_global(Uuid::parse_str(&user_id).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.email, "jane@case.test",
        "stored in its normalized form"
    );

    for spelling in ["jane@case.test", "JANE@CASE.TEST", " Jane@Case.Test "] {
        let (status, body) = sign_in(&res, spelling).await;
        assert_eq!(status, 200, "{spelling}: {body}");
        assert_eq!(
            body["user"]["user_id"],
            user_id.as_str(),
            "{spelling}: {body}"
        );
        assert_eq!(
            body["user"]["email"], "jane@case.test",
            "{spelling}: {body}"
        );
    }
}

/// A second registration of an address in another casing is the same person
/// registering twice: refused, and no second account exists afterwards.
#[tokio::test]
async fn a_second_registration_in_another_casing_is_refused() {
    let res = create_test_server_resources().await.unwrap();
    let repos = &res.common.repos;

    let (status, body) = register(&res, "runner@case.test").await;
    assert_eq!(status, 201, "{body}");
    let accounts = repos.users.count().await.unwrap();

    for spelling in ["RUNNER@case.test", " Runner@Case.Test"] {
        let (status, body) = register(&res, spelling).await;
        assert_eq!(status, 403, "{spelling}: {body}");
        assert!(
            body["message"]
                .as_str()
                .is_some_and(|m| m.contains("User already exists")),
            "{spelling}: {body}"
        );
    }
    assert_eq!(repos.users.count().await.unwrap(), accounts);
}

/// The repository is where every other path creates an account (the CLI, the
/// admin setup, the messaging sign-up): it stores the normalized form, finds
/// it in any casing, and refuses a second account in another casing.
#[tokio::test]
async fn the_repository_stores_one_form_and_refuses_a_case_variant() {
    let res = create_test_server_resources().await.unwrap();
    let users = &res.common.repos.users;

    let mut coach = User::new("coach@case.test".to_owned(), "x".to_owned(), None);
    "Coach@Case.Test ".clone_into(&mut coach.email);
    users.create(&coach).await.unwrap();

    let found = users
        .get_by_email("COACH@case.TEST")
        .await
        .unwrap()
        .expect("found in another casing");
    assert_eq!(found.id, coach.id);
    assert_eq!(found.email, "coach@case.test");

    let mut twin = User::new("coach@case.test".to_owned(), "x".to_owned(), None);
    "coach@CASE.test".clone_into(&mut twin.email);
    let refused = users.create(&twin).await.unwrap_err();
    assert!(
        refused.to_string().contains("Email already in use"),
        "{refused}"
    );
    assert!(users.get_global(twin.id).await.unwrap().is_none());
}
