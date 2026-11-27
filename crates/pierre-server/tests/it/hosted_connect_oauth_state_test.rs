// ABOUTME: Pins that a hosted-connect provider OAuth start sends the provider an opaque state
// ABOUTME: The connect link token stays in the server-side state row, and the callback still returns the athlete to the picker
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Hosted connect OAuth state suite (carnet#1873).
//!
//! The hosted picker starts a provider's OAuth with a return URL that carries
//! the athlete's connect link token. That URL used to ride base64-encoded in
//! the `state`, so the token reached the provider's authorize URL and every
//! request log of our callback. These tests drive the real routes: the state
//! the provider receives names nothing, the callback returns the athlete to the
//! picker with its token from the row it redeems, and a state the server never
//! issued — including one forged in the old shape — returns nobody there.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::{header, StatusCode};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use pierre_core::models::TenantId;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_middleware::provider_link_token::mint_connect_link_token;
use pierre_routes_auth::AuthRoutes;
use url::Url;
use uuid::Uuid;

/// The server's own origin, a loopback URL the redirect allowlist accepts.
const BASE_URL: &str = "http://localhost:8081";

/// A server whose Strava OAuth app is configured, an athlete, and their tenant.
async fn setup() -> (Arc<ServerContext>, Uuid, TenantId) {
    let mut config = common::test_server_config();
    BASE_URL.clone_into(&mut config.base_url);
    let resources = common::create_test_server_resources_with_config(config)
        .await
        .unwrap();
    let (user_id, _) = common::create_test_user(&resources.agent.database)
        .await
        .unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("list tenants")
        .first()
        .expect("user has a tenant")
        .id;
    (resources, user_id, tenant_id)
}

async fn get(resources: &Arc<ServerContext>, uri: &str) -> AxumTestResponse {
    AxumTestRequest::get(uri)
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await
}

fn location(response: &AxumTestResponse) -> Option<String> {
    response
        .header(header::LOCATION.as_str())
        .map(str::to_owned)
}

/// The picker URL a hosted start returns the athlete to.
fn picker_url(token: &str) -> String {
    format!(
        "{BASE_URL}/providers/connect?token={}",
        urlencoding::encode(token)
    )
}

/// Start Strava's OAuth from the hosted picker and return the provider's
/// authorize URL it redirects to.
async fn start(resources: &Arc<ServerContext>, token: &str) -> Url {
    let response = get(
        resources,
        &format!(
            "/api/providers/connect/oauth-init/strava?token={}",
            urlencoding::encode(token)
        ),
    )
    .await;
    assert_eq!(
        response.status_code(),
        StatusCode::FOUND,
        "the hosted start redirects to Strava: {}",
        response.body_text()
    );
    Url::parse(&location(&response).expect("the start names the provider")).unwrap()
}

fn state_of(authorize: &Url) -> String {
    authorize
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
        .expect("the authorize URL carries a state")
}

/// Answer the callback as a provider whose athlete declined: the cancel path
/// redeems the state and returns the athlete wherever its row says.
async fn decline(resources: &Arc<ServerContext>, state: &str) -> AxumTestResponse {
    get(
        resources,
        &format!(
            "/api/oauth/callback/strava?error=access_denied&state={}",
            urlencoding::encode(state)
        ),
    )
    .await
}

#[tokio::test]
async fn the_state_a_hosted_start_sends_the_provider_names_no_token_url_or_user() {
    let (resources, user_id, tenant_id) = setup().await;
    let token = mint_connect_link_token(
        user_id,
        tenant_id.as_uuid(),
        "telegram",
        None,
        &resources.auth.admin_jwt_secret,
    )
    .expect("mint connect token");

    let authorize = start(&resources, &token).await;
    let state = state_of(&authorize);
    let return_url = picker_url(&token);
    let encoded = URL_SAFE_NO_PAD.encode(return_url.as_bytes());
    let user = user_id.to_string();
    for (what, leak) in [
        ("the connect token", token.as_str()),
        ("the return URL", return_url.as_str()),
        ("the base64 return URL", encoded.as_str()),
        ("the user id", user.as_str()),
    ] {
        assert!(
            !state.contains(leak),
            "{what} rides the state the provider sees: {state}"
        );
        assert!(
            authorize
                .query_pairs()
                .all(|(_, value)| !value.contains(leak)),
            "{what} reaches the provider's authorize URL: {authorize}"
        );
    }
    assert!(
        !state.contains(':'),
        "the state has no segments to decode: {state}"
    );
}

#[tokio::test]
async fn the_callback_returns_the_athlete_to_the_picker_with_its_token() {
    let (resources, user_id, tenant_id) = setup().await;
    let token = mint_connect_link_token(
        user_id,
        tenant_id.as_uuid(),
        "telegram",
        None,
        &resources.auth.admin_jwt_secret,
    )
    .expect("mint connect token");
    let state = state_of(&start(&resources, &token).await);

    let response = decline(&resources, &state).await;
    assert_eq!(
        response.status_code(),
        StatusCode::FOUND,
        "the callback redirects: {}",
        response.body_text()
    );
    assert_eq!(
        location(&response).as_deref(),
        Some(
            format!(
                "{}&provider=strava&success=false&error=access_denied",
                picker_url(&token)
            )
            .as_str()
        ),
        "the athlete lands back on the hosted picker, connect token intact"
    );

    let replay = decline(&resources, &state).await;
    assert!(
        location(&replay).is_none_or(|to| !to.contains("/providers/connect")),
        "a redeemed state returns nobody to the picker a second time: {:?}",
        location(&replay)
    );
}

#[tokio::test]
async fn a_tampered_or_forged_state_returns_nobody_to_the_picker() {
    let (resources, user_id, tenant_id) = setup().await;
    let token = mint_connect_link_token(
        user_id,
        tenant_id.as_uuid(),
        "telegram",
        None,
        &resources.auth.admin_jwt_secret,
    )
    .expect("mint connect token");
    let state = state_of(&start(&resources, &token).await);

    let tampered = format!("{state}x");
    let forged = format!(
        "{user_id}:{}:{}",
        Uuid::new_v4(),
        URL_SAFE_NO_PAD.encode(picker_url(&token).as_bytes())
    );
    for (what, bad) in [("a tampered state", &tampered), ("a forged state", &forged)] {
        let response = decline(&resources, bad).await;
        assert!(
            location(&response).is_none_or(|to| !to.contains("/providers/connect")),
            "{what} must not return the athlete to the picker: {:?}",
            location(&response)
        );
    }

    let response = decline(&resources, &state).await;
    assert!(
        location(&response).is_some_and(|to| to.contains("/providers/connect?token=")),
        "the genuine state still redeems after the refused ones: {:?}",
        location(&response)
    );
}
