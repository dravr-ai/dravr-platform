// ABOUTME: The operator's connect order (carnet#1472): a provider flipped to the sign-in first takes a new usr/pwd sign-in with seats free
// ABOUTME: Pins the card, the refusal, the move and the reconnect reading the one order, and the super-admin route that flips it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Connect order (carnet#1472).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// By default a new Garmin connection goes through Garmin's OAuth while a seat
// is free. With the order flipped to the sign-in first, the same athlete signs
// in with usr/pwd, the card recommends the sign-in and asks no move, and the
// reconnect is the ordinary sign-in: every surface reads one order. The order
// is flipped through `PUT /admin/settings/connect-order`, super-admin only.
#![cfg(all(feature = "provider-garmin", feature = "health-sync"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::garmin_history_import_test::{
    athlete, calls_matching, hold_session, server, sign_in, spawn_switchable_scraper,
    ScraperSwitch, LIVE_SESSION,
};
use crate::helpers::axum_test::AxumTestRequest;

use std::sync::Arc;

use axum::http::StatusCode;
use axum::Router;
use pierre_auth::connect_order::{connect_order, set_connect_order, ConnectOrder};
use pierre_config::environment::ServerConfig;
use pierre_core::admin::models::{AdminPermission, CreateAdminTokenRequest};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit, AdminRoutes};
use pierre_routes_auth::AuthRoutes;
use pierre_tool_runtime::protocol::reauth_notice::reconnects_through_oauth;
use pierre_tool_runtime::runtime::ToolRuntime;
use secrecy::ExposeSecret;
use serde_json::{json, Value};

/// A config whose Garmin client holds `cap` seats, with `scraper` as the
/// usr/pwd sign-in service.
fn config_with_cap(scraper: &str, cap: u32) -> ServerConfig {
    let mut config = common::test_server_config_with_sciotte(scraper);
    config.oauth.garmin_seat_cap = Some(cap);
    config
}

/// Put Garmin's account sign-in first, as `pierre-cli settings connect-order` does.
async fn sign_in_first(resources: &Arc<ServerContext>) {
    set_connect_order(
        resources.common.repos.system_settings.as_ref(),
        oauth_providers::GARMIN,
        ConnectOrder::SignInFirst,
    )
    .await
    .unwrap();
}

/// The `provider` card `GET /api/providers` serves the bearer of `token`.
async fn card(resources: &Arc<ServerContext>, token: &str, provider: &str) -> Value {
    let resp = AxumTestRequest::get("/api/providers")
        .header("authorization", &format!("Bearer {token}"))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    let body: Value = serde_json::from_str(&resp.text()).unwrap();
    body["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|card| card["provider"] == provider)
        .unwrap_or_else(|| panic!("the {provider} card is served: {body}"))
        .clone()
}

/// The Garmin card `GET /api/providers` serves the bearer of `token`.
async fn garmin_card(resources: &Arc<ServerContext>, token: &str) -> Value {
    card(resources, token, oauth_providers::SCIOTTE_GARMIN).await
}

/// With seats free and the sign-in first, a new athlete's card recommends
/// the sign-in and their usr/pwd sign-in goes through; flipped back, the
/// same card recommends Garmin's OAuth again.
#[tokio::test]
async fn a_sign_in_first_order_takes_a_new_usr_pwd_sign_in_with_seats_free() {
    let (scraper, calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(config_with_cap(&scraper, 5)).await;
    let (_user_id, _user, _tenant, token) = athlete(&resources).await;

    assert_eq!(
        garmin_card(&resources, &token).await["recommended_backend"],
        "oauth",
        "bug: by default a free seat sends a new connection to Garmin's OAuth"
    );

    sign_in_first(&resources).await;
    let card = garmin_card(&resources, &token).await;
    assert_eq!(
        card["recommended_backend"], "mirror",
        "bug: the sign-in-first order still sends a new connection to the OAuth: {card}"
    );
    assert_eq!(card["seats_left"], 5, "the seats are still counted: {card}");

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(
        status, 200,
        "bug: the sign-in-first order refused the sign-in: {body}"
    );
    assert_eq!(
        body["status"], "connected",
        "bug: the sign-in-first sign-in did not connect: {body}"
    );
    assert_eq!(
        calls_matching(&calls, "login").len(),
        1,
        "bug: the sign-in never reached the sign-in service, or reached it twice"
    );

    set_connect_order(
        resources.common.repos.system_settings.as_ref(),
        oauth_providers::GARMIN,
        ConnectOrder::OAuthFirst,
    )
    .await
    .unwrap();
    let (_other, _user, _tenant, other_token) = athlete(&resources).await;
    assert_eq!(
        garmin_card(&resources, &other_token).await["recommended_backend"],
        "oauth",
        "bug: flipping back left new connections on the sign-in"
    );
}

/// The spec's own case: Strava flipped to the sign-in first sends a new
/// athlete's Strava card to the sign-in with seats to spare, and leaves the
/// Garmin card on Garmin's OAuth.
#[tokio::test]
async fn a_sign_in_first_strava_order_sends_the_strava_card_to_the_sign_in() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(config_with_cap(&scraper, 5)).await;
    let (_user_id, _user, _tenant, token) = athlete(&resources).await;
    let strava = card(&resources, &token, oauth_providers::SCIOTTE).await;
    assert_eq!(
        strava["recommended_backend"], "oauth",
        "precondition: by default a free Strava seat sends a new connection to the OAuth: {strava}"
    );

    set_connect_order(
        resources.common.repos.system_settings.as_ref(),
        oauth_providers::STRAVA,
        ConnectOrder::SignInFirst,
    )
    .await
    .unwrap();
    let strava = card(&resources, &token, oauth_providers::SCIOTTE).await;
    assert_eq!(
        strava["recommended_backend"], "mirror",
        "bug: the sign-in-first Strava order still sends a new connection to the OAuth: {strava}"
    );
    assert_eq!(
        garmin_card(&resources, &token).await["recommended_backend"],
        "oauth",
        "bug: Strava's order moved the Garmin card"
    );
}

/// A usr/pwd-only athlete is never asked to move, and reconnects by sign-in,
/// while the sign-in goes first, though seats are free.
#[tokio::test]
async fn a_sign_in_first_order_asks_no_move_and_reconnects_by_sign_in() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(config_with_cap(&scraper, 5)).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert!(
        reconnects_through_oauth(&runtime, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        "precondition: by default a free seat moves the reconnect to the OAuth"
    );

    sign_in_first(&resources).await;
    let card = garmin_card(&resources, &token).await;
    assert!(
        card.get("migration").is_none(),
        "bug: the sign-in-first order still asks the athlete to move: {card}"
    );
    assert!(
        !reconnects_through_oauth(&runtime, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        "bug: the sign-in-first order still points the reconnect to the OAuth"
    );
}

// ---------------------------------------------------------------------------
// The admin route
// ---------------------------------------------------------------------------

fn admin_api(resources: &Arc<ServerContext>) -> Router {
    AdminRoutes::routes(AdminApiContext::new(AdminApiContextInit {
        database: resources.agent.database.clone(),
        repos: resources.common.repos.clone(),
        jwt_secret: resources.auth.admin_jwt_secret.to_string(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        harness_config_registry: resources.fitness.harness_config_registry.clone(),
        guardian_config_registry: resources.fitness.guardian_config_registry.clone(),
        prompt_registry: resources.mcp.prompt_registry.clone(),
        tool_description_registry: resources.mcp.tool_description_registry.clone(),
        evidence_registry: resources.mcp.evidence_registry.clone(),
        messaging_strings_registry: resources.mcp.messaging_strings_registry.clone(),
        cageux_config_registry: resources.fitness.cageux_config_registry.clone(),
        persona_contract_registry: resources.fitness.persona_contract_registry.clone(),
        training_catalogue_registry: resources.mcp.training_catalogue_registry.clone(),
        contremaitre_config: None,
        app_behavior: resources.common.config.app_behavior.clone(),
    }))
}

async fn admin_bearer(resources: &Arc<ServerContext>, super_admin: bool) -> String {
    let token = resources
        .common
        .repos
        .admin
        .create_token(
            &CreateAdminTokenRequest {
                service_name: "connect-order".to_owned(),
                service_description: None,
                permissions: Some(vec![AdminPermission::ManageUsers]),
                expires_in_days: Some(1),
                is_super_admin: super_admin,
                tenant_id: None,
                operator_user_id: None,
            },
            resources.auth.admin_jwt_secret.as_ref(),
            &resources.auth.jwks_manager,
        )
        .await
        .unwrap();
    format!("Bearer {}", token.jwt_token.expose_secret())
}

/// A super-admin reads both orders, flips Strava's, and reads it back; the
/// store holds what the route said.
#[tokio::test]
async fn a_super_admin_flips_one_providers_order_and_reads_every_order_back() {
    let resources = common::create_test_server_resources().await.unwrap();
    let bearer = admin_bearer(&resources, true).await;

    let shown: Value = AxumTestRequest::get("/admin/settings/connect-order")
        .header("authorization", &bearer)
        .send(admin_api(&resources))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(
        shown["data"]["orders"],
        json!({ "garmin": "oauth_first", "strava": "oauth_first" }),
        "bug: an unset order reads other than OAuth first: {shown}"
    );

    let set: Value = AxumTestRequest::put("/admin/settings/connect-order")
        .header("authorization", &bearer)
        .json(&json!({ "provider": "strava", "order": "sign_in_first" }))
        .send(admin_api(&resources))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(
        set["data"]["orders"],
        json!({ "garmin": "oauth_first", "strava": "sign_in_first" }),
        "bug: the flip reads back wrong, or touched Garmin: {set}"
    );
    assert_eq!(
        connect_order(
            resources.common.repos.system_settings.as_ref(),
            oauth_providers::STRAVA
        )
        .await,
        ConnectOrder::SignInFirst,
        "bug: the route reported a flip the store does not hold"
    );
}

/// The route refuses a token without super-admin, an unknown provider and an
/// unknown order, and writes nothing for any of them.
#[tokio::test]
async fn the_route_refuses_a_plain_admin_and_unknown_values() {
    let resources = common::create_test_server_resources().await.unwrap();
    let plain = admin_bearer(&resources, false).await;
    let root = admin_bearer(&resources, true).await;
    let put = |bearer: String, body: Value| {
        let resources = resources.clone();
        async move {
            AxumTestRequest::put("/admin/settings/connect-order")
                .header("authorization", &bearer)
                .json(&body)
                .send(admin_api(&resources))
                .await
                .status()
        }
    };

    assert_eq!(
        put(
            plain,
            json!({ "provider": "garmin", "order": "sign_in_first" })
        )
        .await,
        403,
        "bug: a plain admin flipped the connect order"
    );
    assert_eq!(
        put(
            root.clone(),
            json!({ "provider": "whoop", "order": "sign_in_first" })
        )
        .await,
        400,
        "bug: a provider with no order was accepted"
    );
    assert_eq!(
        put(
            root,
            json!({ "provider": "garmin", "order": "sciotte_first" })
        )
        .await,
        400,
        "bug: an unknown order was accepted"
    );
    assert_eq!(
        connect_order(
            resources.common.repos.system_settings.as_ref(),
            oauth_providers::GARMIN
        )
        .await,
        ConnectOrder::OAuthFirst,
        "bug: a refused request wrote the order"
    );
}
