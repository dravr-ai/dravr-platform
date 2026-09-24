// ABOUTME: Integration tests for GET /api/providers — a card reflects EITHER backend that serves it
// ABOUTME: Pins carnet#255 (a Strava OAuth row lights the card) and carnet#352 (a dead Garmin row does not)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The Strava and Garmin cards are named for their MIRROR backends (`sciotte`,
//! `sciotte_garmin`), because the scrape is the supported way in. A connection
//! made through OAuth is stored under the user-facing name (`strava`,
//! `garmin`). One athlete and one provider therefore have two possible row
//! names, and the card has to light up for either.
//!
//! Both clients used to merge this themselves, and only for Strava. Garmin had
//! no such merge, so an athlete with a `garmin` row was told no provider was
//! connected — by the connect screen, the connect banner and the chat header —
//! while `/mcp get_connection_status` said connected (carnet#255).

use std::sync::Arc;

use helpers::axum_test::AxumTestRequest;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{ConnectionType, TenantId, User};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_auth::AuthRoutes;
use serde_json::Value;
use uuid::Uuid;

mod common;
mod helpers;

async fn test_setup() -> (Arc<ServerContext>, Uuid, TenantId, User) {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, user) = common::create_test_user(&resources.agent.database)
        .await
        .unwrap();
    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap();
    let tenant_id = TenantId::from_uuid(tenants[0].id.as_uuid());
    (resources, user_id, tenant_id, user)
}

async fn register_connection(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) {
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant_id, provider, &ConnectionType::OAuth, None)
        .await
        .expect("register test connection");
}

/// The cards `GET /api/providers` serves, as `provider -> connected`.
async fn provider_cards(resources: &Arc<ServerContext>, user: &User) -> Vec<(String, bool)> {
    let token = common::generate_test_token(resources, user).await;
    let app = AuthRoutes::routes(resources.auth_routes_context());

    let resp = AxumTestRequest::get("/api/providers")
        .header("authorization", &format!("Bearer {token}"))
        .send(app)
        .await;
    assert_eq!(resp.status(), 200, "providers status should answer");

    let body: Value = serde_json::from_str(&resp.text()).expect("providers status is JSON");
    body["providers"]
        .as_array()
        .expect("providers is an array")
        .iter()
        .map(|p| {
            (
                p["provider"].as_str().unwrap_or_default().to_owned(),
                p["connected"].as_bool().unwrap_or_default(),
            )
        })
        .collect()
}

fn card(cards: &[(String, bool)], name: &str) -> bool {
    cards
        .iter()
        .find(|(provider, _)| provider == name)
        .unwrap_or_else(|| panic!("no {name} card in {cards:?}"))
        .1
}

/// Coalescing stops at what the fetch path can use (carnet#352). Garmin's OAuth
/// API is uncredentialed and `resolve_backend` sends every Garmin request to
/// `sciotte_garmin`, so a bare `garmin` row serves nothing and must leave the
/// card dark. Counting it read "Garmin connecté" in the chat header above a
/// coach reply saying the connection had expired.
#[tokio::test]
async fn garmin_oauth_row_alone_leaves_the_garmin_card_dark() {
    let (resources, user_id, tenant_id, user) = test_setup().await;
    register_connection(&resources, user_id, tenant_id, oauth_providers::GARMIN).await;

    let cards = provider_cards(&resources, &user).await;

    assert!(
        !card(&cards, oauth_providers::SCIOTTE_GARMIN),
        "a garmin OAuth row serves no fetch, so the Garmin card must stay dark: {cards:?}"
    );
    assert!(
        !cards.iter().any(|(_, connected)| *connected),
        "nothing can serve this athlete, so nothing may read as connected: {cards:?}"
    );
}

/// The case both clients used to patch for themselves. Now the server answers
/// it, so the two clients cannot drift apart or from `/mcp`.
#[tokio::test]
async fn strava_oauth_row_lights_the_strava_card() {
    let (resources, user_id, tenant_id, user) = test_setup().await;
    register_connection(&resources, user_id, tenant_id, oauth_providers::STRAVA).await;

    let cards = provider_cards(&resources, &user).await;

    assert!(
        card(&cards, oauth_providers::SCIOTTE),
        "a strava connection must show on the Strava-path card: {cards:?}"
    );
}

/// Coalescing must not have broken the path it was already serving: a scrape
/// connection is stored under the mirror name and lights the same card.
#[tokio::test]
async fn mirror_row_still_lights_its_own_card() {
    let (resources, user_id, tenant_id, user) = test_setup().await;
    register_connection(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;

    let cards = provider_cards(&resources, &user).await;

    assert!(
        card(&cards, oauth_providers::SCIOTTE_GARMIN),
        "a sciotte_garmin connection must show on the Garmin card: {cards:?}"
    );
}

/// TrainingPeaks has only its mirror, so its scrape row is the whole card —
/// and must light that card alone.
#[tokio::test]
async fn trainingpeaks_mirror_row_lights_its_own_card_and_no_other() {
    let (resources, user_id, tenant_id, user) = test_setup().await;
    register_connection(
        &resources,
        user_id,
        tenant_id,
        oauth_providers::SCIOTTE_TRAININGPEAKS,
    )
    .await;

    let cards = provider_cards(&resources, &user).await;

    assert!(
        card(&cards, oauth_providers::SCIOTTE_TRAININGPEAKS),
        "a sciotte_trainingpeaks connection must show on the TrainingPeaks card: {cards:?}"
    );
    assert!(
        !card(&cards, oauth_providers::SCIOTTE) && !card(&cards, oauth_providers::SCIOTTE_GARMIN),
        "a TrainingPeaks connection must not light the Strava or Garmin card: {cards:?}"
    );
}

/// Coalescing widens what counts as connected, so pin the other direction too:
/// an athlete with no connection has no card lit, and one provider does not
/// light another.
#[tokio::test]
async fn connections_do_not_bleed_across_providers() {
    let (resources, user_id, tenant_id, user) = test_setup().await;

    let none = provider_cards(&resources, &user).await;
    assert!(
        !none.iter().any(|(_, connected)| *connected),
        "no card may read connected before anything is connected: {none:?}"
    );

    register_connection(&resources, user_id, tenant_id, oauth_providers::GARMIN).await;
    let cards = provider_cards(&resources, &user).await;

    assert!(
        !card(&cards, oauth_providers::SCIOTTE),
        "a garmin connection must not light the Strava card: {cards:?}"
    );
    assert!(
        !card(&cards, oauth_providers::WHOOP),
        "a garmin connection must not light the Whoop card: {cards:?}"
    );
}
