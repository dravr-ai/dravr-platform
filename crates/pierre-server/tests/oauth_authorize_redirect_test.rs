// ABOUTME: Route tests for GET /api/oauth/authorize/{provider}, the OAuth launch surface
// ABOUTME: Asserts it 302s to the provider so a popup opened on it is never left blank

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The launch route exists so a client can `window.open` a real same-origin URL
//! inside the click's user-gesture window instead of opening `about:blank`,
//! awaiting an authorization URL, and assigning `location.href` afterwards.
//!
//! That older shape left the window empty for the whole round trip. On a
//! desktop it flashed past; on an iPhone users read it as a broken connect and
//! gave up before the provider loaded — four occurrences across two people, all
//! iPhone Safari, every one of them a `/mobile/init` that returned 200 with no
//! callback ever following.
//!
//! So these tests assert the *redirect*, not merely a 2xx: a body-bearing
//! success would put us straight back to a client that has to navigate the
//! window itself.

mod common;
mod helpers;

use axum::http::StatusCode;
use axum::Router;
use common::{create_test_server_resources, create_test_user};
use helpers::axum_test::AxumTestRequest;
use pierre_routes_auth::AuthRoutes;

async fn setup() -> (Router, String) {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.coach.database).await.unwrap();
    // The session must carry an active tenant: OAuth state is tenant-scoped, so
    // a tenant-less token is rejected before any authorize URL is built.
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("list tenants")
        .first()
        .expect("a new user has a personal tenant")
        .id;
    let token = resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &user,
            &resources.auth.jwks_manager,
            Some(tenant_id.to_string()),
        )
        .unwrap();
    let router = AuthRoutes::routes(resources.auth_routes_context());
    (router, format!("Bearer {token}"))
}

#[tokio::test]
async fn authorize_redirects_the_browser_to_the_provider() {
    let (router, auth) = setup().await;

    let response = AxumTestRequest::get("/api/oauth/authorize/strava")
        .header("authorization", &auth)
        .send(router)
        .await;

    assert_eq!(
        response.status(),
        StatusCode::FOUND,
        "the launch route must redirect; anything else forces the client back \
         into navigating the window itself, which is what left it blank"
    );

    let location = response
        .header("location")
        .expect("a 302 must carry a Location header");
    assert!(
        location.starts_with("https://www.strava.com/oauth/authorize"),
        "Location must point at Strava's authorize endpoint, got: {location}"
    );
    assert!(
        location.contains("client_id="),
        "the authorize URL must carry a client_id, got: {location}"
    );
    assert!(
        location.contains("state="),
        "the authorize URL must carry the CSRF state the callback validates, got: {location}"
    );
}

#[tokio::test]
async fn authorize_without_a_session_does_not_redirect() {
    let (router, _auth) = setup().await;

    let response = AxumTestRequest::get("/api/oauth/authorize/strava")
        .send(router)
        .await;

    assert_ne!(
        response.status(),
        StatusCode::FOUND,
        "an unauthenticated caller must never be handed an authorize URL bound \
         to somebody's state"
    );
    assert!(
        (400..500).contains(&response.status()),
        "expected a client error for a session-less launch, got {}",
        response.status()
    );
}

#[tokio::test]
async fn authorize_rejects_a_provider_that_does_not_support_oauth() {
    let (router, auth) = setup().await;

    let response = AxumTestRequest::get("/api/oauth/authorize/not_a_provider")
        .header("authorization", &auth)
        .send(router)
        .await;

    assert_ne!(
        response.status(),
        StatusCode::FOUND,
        "an unknown provider must not produce a redirect to nowhere"
    );
}
