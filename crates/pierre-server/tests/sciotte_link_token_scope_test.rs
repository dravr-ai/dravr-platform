// ABOUTME: A hosted-login link-token reaches only the Sciotte login flow, for the platform it was minted for
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

mod common;
mod helpers;

use common::{create_test_server_resources, create_test_user_with_email};
use helpers::axum_test::AxumTestRequest;
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
    let garmin_token = mint_link_token(
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

    let response = AxumTestRequest::post("/api/providers/sciotte/login")
        .header("authorization", &format!("Bearer {garmin_token}"))
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
