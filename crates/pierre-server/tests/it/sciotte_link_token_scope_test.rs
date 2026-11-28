// ABOUTME: A hosted-login link-token reaches only the session sign-in flow, for the platform it was minted for
// ABOUTME: No route mints one over HTTP or connects/disconnects with one; a garmin token cannot sign in to TrainingPeaks

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A provider link-token is a 24-hour bearer embedded in a chat message. It
//! used to also authenticate `POST /api/providers/sciotte/connect` (which
//! stored whatever session blob the caller sent as the user's session) and
//! `DELETE /api/providers/sciotte/disconnect`, neither of which any client
//! called, and an admin-authenticated mint route no service called either —
//! every live mint is in-process. Those three routes are gone. What remains is
//! the login flow, and a provider-specific token signs in only to its own
//! platform: a Garmin reconnect link cannot accept the TrainingPeaks notice or
//! store a TrainingPeaks session.

use crate::common;
use crate::helpers;

use common::{create_test_server_resources, create_test_user_with_email};
use helpers::axum_test::AxumTestRequest;
use helpers::hosted_signin::{embedded_page_token, hosted_signin_session};
use pierre_middleware::provider_link_token::{
    mint_connect_link_token, mint_link_token, MintProviderLinkTokenArgs,
};
use pierre_routes_auth::AuthRoutes;
use serde_json::{json, Value};

const REFUSAL: &str = "This login link is for another provider; ask for a fresh link for this one";

#[tokio::test]
async fn no_route_mints_connects_or_disconnects_with_a_link_token() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user_with_email(&resources.agent.database, "scope@link.test")
        .await
        .unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()[0]
        .id;
    let token = mint_link_token(
        &MintProviderLinkTokenArgs {
            user_id,
            tenant_id: tenant_id.as_uuid(),
            provider: "sciotte",
            target: "strava",
            channel: "telegram",
            channel_thread: None,
        },
        &resources.auth.admin_jwt_secret,
    )
    .unwrap();
    let bearer = format!("Bearer {token}");
    let routes = || AuthRoutes::routes(resources.auth_routes_context());

    let connect = AxumTestRequest::post("/api/providers/sciotte/connect")
        .header("authorization", &bearer)
        .json(&json!({ "session_id": "attacker-chosen-session" }))
        .send(routes())
        .await;
    assert_eq!(connect.status(), 404, "the connect route is gone");

    let disconnect = AxumTestRequest::delete("/api/providers/sciotte/disconnect")
        .header("authorization", &bearer)
        .send(routes())
        .await;
    assert_eq!(disconnect.status(), 404, "the disconnect route is gone");

    let mint = AxumTestRequest::post("/api/channels/provider/sciotte/link-token")
        .header("authorization", &bearer)
        .json(&json!({ "user_id": user_id, "tenant_id": tenant_id, "channel": "slack" }))
        .send(routes())
        .await;
    assert_eq!(mint.status(), 404, "no route mints a link-token");

    assert!(
        resources
            .common
            .repos
            .oauth_tokens
            .get_token(user_id, tenant_id, "sciotte")
            .await
            .unwrap()
            .is_none(),
        "no session was stored from a caller-supplied blob"
    );
}

#[tokio::test]
async fn a_provider_link_token_signs_in_only_to_its_own_platform() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user_with_email(&resources.agent.database, "tgt@link.test")
        .await
        .unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()[0]
        .id;
    let garmin_session = hosted_signin_session(
        &MintProviderLinkTokenArgs {
            user_id,
            tenant_id: tenant_id.as_uuid(),
            provider: "sciotte",
            target: "garmin",
            channel: "telegram",
            channel_thread: None,
        },
        &resources.auth.admin_jwt_secret,
    );

    let response = AxumTestRequest::post("/api/providers/sciotte/login")
        .header("authorization", &format!("Bearer {garmin_session}"))
        .json(&json!({
            "email": "athlete@example.com",
            "password": "secret",
            "target": "trainingpeaks",
            "tos_consent": true,
        }))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    assert_eq!(response.status(), 403);
    let body: Value = response.json();
    assert_eq!(body["message"].as_str(), Some(REFUSAL), "{body}");

    // The connect picker's token carries no platform, so the page's choice
    // stands: it is not refused by the platform check.
    let connect_token = mint_connect_link_token(
        user_id,
        tenant_id.as_uuid(),
        "telegram",
        None,
        &resources.auth.admin_jwt_secret,
    )
    .unwrap();
    let response = AxumTestRequest::post("/api/providers/sciotte/login")
        .header("authorization", &format!("Bearer {connect_token}"))
        .json(&json!({
            "email": "athlete@example.com",
            "password": "secret",
            "target": "garmin",
        }))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    let body: Value = response.json();
    assert_ne!(
        body["message"].as_str(),
        Some(REFUSAL),
        "a connect token chooses its platform on the page: {body}"
    );
}

/// A copy of a hosted sign-in link signs in to nothing (carnet#1874).
///
/// The page burns its link-token on load and signs in with the session token
/// that load minted, so a chat message, a browser history or a forwarded URL
/// reaches no sign-in route, before or after the page opened.
#[tokio::test]
async fn a_copied_link_token_signs_in_to_nothing() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user_with_email(&resources.agent.database, "copy@link.test")
        .await
        .unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()[0]
        .id;
    let link = mint_link_token(
        &MintProviderLinkTokenArgs {
            user_id,
            tenant_id: tenant_id.as_uuid(),
            provider: "sciotte",
            target: "garmin",
            channel: "telegram",
            channel_thread: None,
        },
        &resources.auth.admin_jwt_secret,
    )
    .unwrap();
    let routes = || AuthRoutes::routes(resources.auth_routes_context());
    // A target the link does not name: a bearer the route accepts reaches
    // the platform check and is refused there, one it does not stops at auth.
    let login = |bearer: String| {
        AxumTestRequest::post("/api/providers/sciotte/login")
            .header("authorization", &format!("Bearer {bearer}"))
            .json(&json!({
                "email": "athlete@example.com",
                "password": "secret",
                "target": "trainingpeaks",
                "tos_consent": true,
            }))
    };

    let unopened = login(link.clone()).send(routes()).await;
    assert_eq!(
        unopened.status(),
        401,
        "an unopened link signs in to nothing"
    );

    let page = AxumTestRequest::get(&format!(
        "/providers/signin?token={}",
        urlencoding::encode(&link)
    ))
    .send(routes())
    .await;
    assert_eq!(page.status(), 200, "a fresh link opens the hosted page");
    let page = page.text();
    assert!(
        !page.contains(&link),
        "the page never embeds its link-token"
    );
    let session = embedded_page_token(&page).to_owned();
    assert!(
        !page.contains(&format!("token={}", urlencoding::encode(&session))),
        "the session token goes into no URL the page renders"
    );
    // The page's script builds its URLs at runtime, where no rendered literal
    // shows them, so the served template is scanned for every use of its
    // bearer: each one is an `Authorization` header, none a URL.
    assert_eq!(
        page.matches("CONFIG.token").count(),
        page.matches("'Bearer ' + CONFIG.token").count(),
        "the page's script sends its session only as a bearer header, never in a URL"
    );

    let copied = login(link.clone()).send(routes()).await;
    assert_eq!(
        copied.status(),
        401,
        "a copy of the burned link signs in to nothing"
    );

    let reopened = AxumTestRequest::get(&format!(
        "/providers/signin?token={}",
        urlencoding::encode(&session)
    ))
    .send(routes())
    .await
    .text();
    assert!(
        !reopened.contains("id=\"phase-credentials\""),
        "the session token opens no page: {reopened}"
    );

    let signed_in = login(session).send(routes()).await;
    assert_eq!(
        signed_in.status(),
        403,
        "the page's session reaches the route"
    );
    let body: Value = signed_in.json();
    assert_eq!(body["message"].as_str(), Some(REFUSAL), "{body}");
}

/// A connect token opens no hosted sign-in page.
///
/// The picker keeps its own token and page: the token is not exchanged for a
/// sign-in session, so the page answers with the link error and no bearer.
#[tokio::test]
async fn a_connect_token_opens_no_hosted_sign_in_page() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user_with_email(&resources.agent.database, "picker@link.test")
        .await
        .unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()[0]
        .id;
    let connect = mint_connect_link_token(
        user_id,
        tenant_id.as_uuid(),
        "telegram",
        None,
        &resources.auth.admin_jwt_secret,
    )
    .unwrap();

    let page = AxumTestRequest::get(&format!(
        "/providers/signin?token={}",
        urlencoding::encode(&connect)
    ))
    .send(AuthRoutes::routes(resources.auth_routes_context()))
    .await
    .text();
    assert!(
        !page.contains("id=\"phase-credentials\"") && !page.contains("token: \""),
        "a connect token renders the link error, with no bearer in it: {page}"
    );
}
