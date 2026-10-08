// ABOUTME: A WHOOP connect with no client to run under asks for the athlete's own app instead of reaching WHOOP
// ABOUTME: Pins the placeholder credential as unset, the card's own_app_required, and the server callback an own app presents

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Production's shared WHOOP app credentials were the infrastructure's
//! `PLACEHOLDER_FILL_MANUALLY` seed, which the server sent to WHOOP as the
//! `client_id`: an athlete without an app of their own landed on WHOOP's
//! "The requested OAuth 2.0 Client does not exist". An athlete with one sent
//! the redirect URI an old client had stored with it, which their WHOOP app
//! did not register.
//!
//! The placeholder now reads as unset, so the provider card says the athlete
//! needs an app of their own and the launch refuses instead of minting a URL;
//! an athlete's own app is authorized at this server's callback, the one the
//! card and the app routes tell them to register.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate, and the surviving crate doc keeps `missing_docs` quiet.
#![cfg(feature = "provider-whoop")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::env;
use std::sync::Arc;

use axum::Router;
use common::{create_test_server_resources, create_test_user};
use helpers::axum_test::AxumTestRequest;
use pierre_core::constants::oauth::providers::WHOOP;
use pierre_core::models::{TenantId, User};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_auth::AuthRoutes;
use pierre_routes_identity::UserOAuthAppRoutes;
use serde_json::{json, Value};
use serial_test::serial;
use url::Url;

/// What `infra/modules/secrets/main.tf` seeds an unfilled secret with.
const PLACEHOLDER: &str = "PLACEHOLDER_FILL_MANUALLY";

/// The retired domain an old mobile build stored as an own app's redirect.
const STALE_REDIRECT: &str = "https://pierre.fit/api/oauth/callback/whoop";

/// The server's WHOOP app as production configured it: both credentials
/// still the placeholder, and no `PIERRE_`-prefixed pair behind them.
fn placeholder_whoop_env() {
    env::set_var("WHOOP_CLIENT_ID", PLACEHOLDER);
    env::set_var("WHOOP_CLIENT_SECRET", PLACEHOLDER);
    env::remove_var("PIERRE_WHOOP_CLIENT_ID");
    env::remove_var("PIERRE_WHOOP_CLIENT_SECRET");
}

fn clear_whoop_env() {
    env::remove_var("WHOOP_CLIENT_ID");
    env::remove_var("WHOOP_CLIENT_SECRET");
}

/// A user, their personal tenant (which carries no WHOOP credentials) and a
/// session bearer for it.
async fn athlete(resources: &Arc<ServerContext>) -> (User, TenantId, String) {
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()
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
    (user, tenant_id, format!("Bearer {token}"))
}

fn routes(resources: &Arc<ServerContext>) -> Router {
    AuthRoutes::routes(resources.auth_routes_context())
        .merge(UserOAuthAppRoutes::routes::<ServerContext>().with_state(Arc::clone(resources)))
}

async fn whoop_card(resources: &Arc<ServerContext>, auth: &str) -> Value {
    let resp = AxumTestRequest::get("/api/providers")
        .header("authorization", auth)
        .send(routes(resources))
        .await;
    let body: Value = serde_json::from_str(&resp.text()).unwrap();
    body["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["provider"] == WHOOP)
        .unwrap_or_else(|| panic!("no WHOOP card: {body}"))
        .clone()
}

/// The web launch with WHOOP's notice accepted: `(status, Location, body)`.
async fn launch(resources: &Arc<ServerContext>, auth: &str) -> (u16, Option<String>, String) {
    let resp = AxumTestRequest::get("/api/oauth/authorize/whoop?tos_consent=true")
        .header("authorization", auth)
        .send(routes(resources))
        .await;
    let location = resp.header("location").map(ToOwned::to_owned);
    (resp.status(), location, resp.text())
}

fn query_param(url: &str, key: &str) -> Option<String> {
    Url::parse(url)
        .unwrap()
        .query_pairs()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}

#[tokio::test]
#[serial]
async fn a_placeholder_server_app_asks_for_the_athletes_own_and_mints_no_url() {
    placeholder_whoop_env();
    let resources = create_test_server_resources().await.unwrap();
    let (_user, _tenant_id, auth) = athlete(&resources).await;

    let card = whoop_card(&resources, &auth).await;
    assert_eq!(card["own_app_required"], true, "{card}");
    let callback = card["oauth_callback_url"].as_str().unwrap_or_default();
    assert!(
        callback.ends_with("/api/oauth/callback/whoop"),
        "the card names the callback to register: {card}"
    );

    let (status, location, body) = launch(&resources, &auth).await;
    assert_eq!(status, 404, "no client to run under is refused: {body}");
    assert!(
        location.is_none(),
        "nothing redirects to WHOOP: {location:?}"
    );
    assert!(!body.contains(PLACEHOLDER), "{body}");
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["code"], "ResourceNotFound", "{body}");

    clear_whoop_env();
}

#[tokio::test]
#[serial]
async fn an_athletes_own_app_is_authorized_at_the_servers_callback() {
    placeholder_whoop_env();
    let resources = create_test_server_resources().await.unwrap();
    let (user, _tenant_id, auth) = athlete(&resources).await;
    let callback = whoop_card(&resources, &auth).await["oauth_callback_url"]
        .as_str()
        .unwrap()
        .to_owned();

    // An older client still sends the redirect it computed; the app is
    // registered at the server's callback whatever it names.
    let resp = AxumTestRequest::post("/api/users/oauth-apps")
        .header("authorization", &auth)
        .json(&json!({
            "provider": WHOOP,
            "client_id": "own-whoop-client",
            "client_secret": "own-whoop-secret",
            "redirect_uri": STALE_REDIRECT,
        }))
        .send(routes(&resources))
        .await;
    let status = resp.status();
    let registered: Value = serde_json::from_str(&resp.text()).unwrap();
    assert_eq!(status, 201, "{registered}");
    assert_eq!(
        registered["redirect_uri"],
        callback.as_str(),
        "{registered}"
    );

    let resp = AxumTestRequest::get("/api/users/oauth-apps/whoop")
        .header("authorization", &auth)
        .send(routes(&resources))
        .await;
    let summary: Value = serde_json::from_str(&resp.text()).unwrap();
    assert_eq!(summary["redirect_uri"], callback.as_str(), "{summary}");

    let card = whoop_card(&resources, &auth).await;
    assert_eq!(card["own_app_required"], false, "{card}");

    let (status, location, body) = launch(&resources, &auth).await;
    assert_eq!(status, 302, "{body}");
    let location = location.expect("the launch redirects to WHOOP");
    assert_eq!(
        query_param(&location, "client_id").as_deref(),
        Some("own-whoop-client")
    );
    assert_eq!(
        query_param(&location, "redirect_uri").as_deref(),
        Some(callback.as_str())
    );

    // A row an old client stored with the retired domain still presents the
    // server's callback.
    resources
        .common
        .repos
        .oauth_tokens
        .store_user_oauth_app(
            user.id,
            WHOOP,
            "own-whoop-client",
            "own-whoop-secret",
            STALE_REDIRECT,
        )
        .await
        .unwrap();
    let (_, location, body) = launch(&resources, &auth).await;
    let location = location.unwrap_or_else(|| panic!("no redirect: {body}"));
    assert_eq!(
        query_param(&location, "redirect_uri").as_deref(),
        Some(callback.as_str())
    );

    clear_whoop_env();
}
