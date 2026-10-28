// ABOUTME: The Garmin card connects through Garmin's API while seats remain, and a Garmin API user is read from pushes
// ABOUTME: Seat fallback to the usr/pwd connection, the hosted picker's OAuth-first card, routing and the health-sync skip
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin API connect and routing (carnet#1155).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// The Garmin card works as the Strava card does: a new connection goes
// through Garmin's OAuth while the Developer Program client has a seat, and
// through the connection using usr/pwd from Dravr once the pool is full, the
// count fails or no Garmin client is configured. Which card is listed, its
// order and the withheld raw `garmin` row do not change. Once connected, a
// Garmin API user's reads resolve to the API backend, which serves what
// Garmin pushed without a call to Garmin, and the health sync leaves them to
// the pushes.
#![cfg(all(feature = "provider-garmin", feature = "health-sync"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use chrono::{Duration, Utc};
use dravr_enforme::traits::connection_store::UserConnectionStore;
use dravr_enforme::traits::credential_store::CredentialStore;
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use helpers::axum_test::AxumTestRequest;
use pierre_config::environment::{OAuthProviderConfig, ProviderEndpointOverride, ServerConfig};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::constants::provider_seats::GARMIN_SEAT_CAPACITY_DEFAULT;
use pierre_core::models::{
    ActivityBuilder, ConnectionType, SportType, TenantId, User, UserOAuthToken,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_middleware::provider_link_token::mint_connect_link_token;
use pierre_routes_auth::AuthRoutes;
use pierre_services::health_sync::PierreSyncStorage;
use pierre_tool_runtime::implementations::data::GetActivitiesTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use secrecy::SecretString;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use uuid::Uuid;

/// A user of `resources` and the tenant they own.
async fn user_and_tenant(resources: &Arc<ServerContext>, email: &str) -> (Uuid, User, TenantId) {
    let (user_id, user) = common::create_test_user_with_email(&resources.agent.database, email)
        .await
        .unwrap();
    let tenant = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()
        .first()
        .expect("user has a tenant")
        .id;
    resources
        .common
        .repos
        .users
        .update_locale(user_id, "en")
        .await
        .unwrap();
    (user_id, user, tenant)
}

/// Store a live `provider` token for the user and its connection row, as a
/// connect writes them. `garmin_user` is the Garmin id `user/id` names.
async fn connect(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    provider: &str,
    garmin_user: Option<&str>,
) {
    let now = Utc::now();
    let token = UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant.to_string(),
        provider: provider.to_owned(),
        access_token: SecretString::from(format!("{provider}-access-do-not-log")),
        refresh_token: Some(SecretString::from("refresh-do-not-log".to_owned())),
        token_type: if provider == oauth_providers::GARMIN {
            "Bearer".to_owned()
        } else {
            "session".to_owned()
        },
        expires_at: Some(now + Duration::hours(12)),
        scope: None,
        provider_user_id: garmin_user.map(str::to_owned),
        oauth_app_client_id: None,
        created_at: now,
        updated_at: now,
    };
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&token)
        .await
        .unwrap();
    let kind = if provider == oauth_providers::GARMIN {
        ConnectionType::OAuth
    } else {
        ConnectionType::Manual
    };
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, provider, &kind, None)
        .await
        .unwrap();
}

/// The cards `GET /api/providers` serves.
async fn cards(resources: &Arc<ServerContext>, user: &User) -> Vec<Value> {
    let token = common::generate_test_token(resources, user).await;
    let resp = AxumTestRequest::get("/api/providers")
        .header("authorization", &format!("Bearer {token}"))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    assert_eq!(resp.status(), 200);
    let body: Value = serde_json::from_str(&resp.text()).unwrap();
    body["providers"].as_array().unwrap().clone()
}

fn card<'a>(cards: &'a [Value], provider: &str) -> &'a Value {
    cards
        .iter()
        .find(|c| c["provider"] == provider)
        .unwrap_or_else(|| panic!("no {provider} card in {cards:?}"))
}

fn position(cards: &[Value], provider: &str) -> usize {
    cards
        .iter()
        .position(|c| c["provider"] == provider)
        .unwrap_or_else(|| panic!("no {provider} card in {cards:?}"))
}

/// The hosted connect picker's page for the user.
async fn picker_page(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) -> String {
    let token = mint_connect_link_token(
        user_id,
        tenant.as_uuid(),
        "telegram",
        None,
        &resources.auth.admin_jwt_secret,
    )
    .unwrap();
    let resp = AxumTestRequest::get(&format!(
        "/providers/connect?token={}",
        urlencoding::encode(&token)
    ))
    .send(AuthRoutes::routes(resources.auth_routes_context()))
    .await;
    assert_eq!(resp.status(), 200);
    resp.text()
}

/// A config whose Garmin client holds `cap` seats.
fn config_with_garmin_cap(cap: u32) -> ServerConfig {
    let mut config = common::test_server_config();
    config.oauth.garmin_seat_cap = Some(cap);
    config
}

#[tokio::test]
async fn the_garmin_card_offers_garmin_oauth_while_seats_remain() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (_, user, _) = user_and_tenant(&resources, "garmin-seat-open@example.com").await;

    let cards = cards(&resources, &user).await;
    let garmin = card(&cards, oauth_providers::SCIOTTE_GARMIN);
    assert_eq!(garmin["recommended_backend"], "oauth", "{garmin}");
    assert_eq!(garmin["seats_left"], 1_000, "{garmin}");

    // Nothing else about the list moves: the raw garmin row stays withheld,
    // the Garmin card keeps its place after Strava's, and TrainingPeaks keeps
    // no recommendation.
    assert!(
        !cards
            .iter()
            .any(|c| c["provider"] == oauth_providers::GARMIN),
        "{cards:?}"
    );
    assert!(
        position(&cards, oauth_providers::SCIOTTE)
            < position(&cards, oauth_providers::SCIOTTE_GARMIN)
    );
    assert!(card(&cards, oauth_providers::SCIOTTE_TRAININGPEAKS)
        .get("recommended_backend")
        .is_none());
}

#[tokio::test]
async fn a_full_garmin_pool_falls_back_to_the_usr_pwd_connection() {
    let resources = common::create_test_server_resources_with_config(config_with_garmin_cap(1))
        .await
        .unwrap();
    let (holder, _, holder_tenant) =
        user_and_tenant(&resources, "garmin-seat-holder@example.com").await;
    connect(
        &resources,
        holder,
        holder_tenant,
        oauth_providers::GARMIN,
        Some("garmin-user-holder"),
    )
    .await;
    let (user_id, user, tenant) = user_and_tenant(&resources, "garmin-seat-late@example.com").await;

    let cards = cards(&resources, &user).await;
    let garmin = card(&cards, oauth_providers::SCIOTTE_GARMIN);
    assert_eq!(garmin["recommended_backend"], "mirror", "{garmin}");
    assert_eq!(garmin["seats_left"], 0, "{garmin}");

    // The hosted picker offers the usr/pwd connection on the same card.
    let page = picker_page(&resources, user_id, tenant).await;
    assert!(
        page.contains(r#"{"provider":"sciotte_garmin","#),
        "the Garmin card stays the credential card: {page}"
    );
    assert!(!page.contains(r#"{"provider":"garmin","#), "{page}");
}

/// Fill every seat of the default Garmin client (the Starter tier's 1,000)
/// with live Garmin API grants, one athlete each.
async fn fill_the_garmin_pool(resources: &Arc<ServerContext>) {
    for n in 0..GARMIN_SEAT_CAPACITY_DEFAULT {
        let (holder, _, holder_tenant) = common::create_test_user_with_plan(
            &resources.agent.database,
            &format!("garmin-seat-{n}@example.com"),
            "starter",
        )
        .await
        .unwrap();
        connect(
            resources,
            holder,
            holder_tenant,
            oauth_providers::GARMIN,
            Some(&format!("garmin-user-{n}")),
        )
        .await;
    }
}

/// The card is drawn before the connect starts, so two athletes shown the
/// last seat could both start Garmin's OAuth. The server holds the cap where
/// the flow starts: with 1,000 seats used, neither the hosted picker's start
/// nor the app's authorize launch reaches Garmin for a new athlete — the
/// picker goes back to itself, which offers the usr/pwd connection — while an
/// athlete already holding a Garmin grant reconnects through OAuth.
#[tokio::test]
async fn a_garmin_oauth_start_with_1000_seats_used_falls_back_to_the_usr_pwd_connect() {
    let resources = common::create_test_server_resources().await.unwrap();
    fill_the_garmin_pool(&resources).await;
    let (user_id, user, tenant) =
        user_and_tenant(&resources, "garmin-seat-raced@example.com").await;

    let link = mint_connect_link_token(
        user_id,
        tenant.as_uuid(),
        "telegram",
        None,
        &resources.auth.admin_jwt_secret,
    )
    .unwrap();
    let resp = AxumTestRequest::get(&format!(
        "/api/providers/connect/oauth-init/garmin?token={}",
        urlencoding::encode(&link)
    ))
    .send(AuthRoutes::routes(resources.auth_routes_context()))
    .await;
    assert_eq!(
        resp.status(),
        302,
        "the hosted start redirects rather than failing"
    );
    let location = resp.header("location").unwrap_or_default();
    assert!(
        location.contains("/providers/connect?token="),
        "a full pool sends the athlete back to the picker, not to Garmin: {location}"
    );
    let page = picker_page(&resources, user_id, tenant).await;
    assert!(
        page.contains(r#"{"provider":"sciotte_garmin","#),
        "the picker it returns to offers the usr/pwd connection: {page}"
    );

    let jwt = common::generate_test_token(&resources, &user).await;
    let resp = AxumTestRequest::get("/api/oauth/authorize/garmin")
        .header("authorization", &format!("Bearer {jwt}"))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    assert_ne!(
        resp.status(),
        302,
        "the app's authorize launch starts no Garmin flow with the pool full"
    );
    assert!(
        resp.status() >= 400,
        "a full pool refuses the authorize launch, got {}",
        resp.status()
    );

    // An athlete already holding a Garmin grant re-signs the seat they hold.
    let (holder, holder_user, holder_tenant) =
        user_and_tenant(&resources, "garmin-seat-reconnect@example.com").await;
    connect(
        &resources,
        holder,
        holder_tenant,
        oauth_providers::GARMIN,
        Some("garmin-user-reconnect"),
    )
    .await;
    let jwt = common::generate_test_token(&resources, &holder_user).await;
    let resp = AxumTestRequest::get("/api/oauth/authorize/garmin")
        .header("authorization", &format!("Bearer {jwt}"))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    assert_eq!(
        resp.status(),
        302,
        "a reconnect of a held grant starts Garmin's OAuth whatever the pool"
    );
}

#[tokio::test]
async fn a_server_without_a_garmin_client_offers_the_usr_pwd_connection() {
    let mut config = common::test_server_config();
    config.oauth.garmin = OAuthProviderConfig::default();
    let resources = common::create_test_server_resources_with_config(config)
        .await
        .unwrap();
    let (_, user, _) = user_and_tenant(&resources, "garmin-no-client@example.com").await;

    let cards = cards(&resources, &user).await;
    let garmin = card(&cards, oauth_providers::SCIOTTE_GARMIN);
    assert_eq!(garmin["recommended_backend"], "mirror", "{garmin}");
    assert_eq!(garmin["seats_left"], 0, "{garmin}");
}

#[tokio::test]
async fn the_hosted_picker_starts_garmin_oauth_while_seats_remain() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, _, tenant) = user_and_tenant(&resources, "garmin-picker@example.com").await;

    let page = picker_page(&resources, user_id, tenant).await;
    let start = page
        .find(r#"{"provider":"garmin","#)
        .unwrap_or_else(|| panic!("the Garmin card launches Garmin's OAuth: {page}"));
    let garmin_card = &page[start..start + page[start..].find('}').unwrap()];
    assert!(garmin_card.contains(r#""kind":"oauth""#), "{garmin_card}");
    assert!(
        garmin_card.contains(r#""target":"garmin""#),
        "{garmin_card}"
    );
    assert!(
        !page.contains(r#"{"provider":"sciotte_garmin","#),
        "one Garmin card: {page}"
    );
}

#[tokio::test]
async fn a_garmin_api_user_leaves_the_health_sync_roster_and_a_usr_pwd_user_stays() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (usr_pwd, _, usr_pwd_tenant) =
        user_and_tenant(&resources, "garmin-roster-usrpwd@example.com").await;
    connect(
        &resources,
        usr_pwd,
        usr_pwd_tenant,
        oauth_providers::SCIOTTE_GARMIN,
        None,
    )
    .await;
    // Holds both: the API wins their reads, so its pushes feed their days.
    let (both, _, both_tenant) =
        user_and_tenant(&resources, "garmin-roster-both@example.com").await;
    connect(
        &resources,
        both,
        both_tenant,
        oauth_providers::SCIOTTE_GARMIN,
        None,
    )
    .await;
    connect(
        &resources,
        both,
        both_tenant,
        oauth_providers::GARMIN,
        Some("garmin-user-both"),
    )
    .await;

    let storage = PierreSyncStorage::new(&resources.common.repos);
    let roster = storage.list_connected_users("garmin").await.unwrap();
    let ids: Vec<String> = roster.iter().map(|u| u.user_id.clone()).collect();

    assert!(ids.contains(&usr_pwd.to_string()), "{ids:?}");
    assert!(!ids.contains(&both.to_string()), "{ids:?}");

    // Nor does any other pull reach them: a sync asked for on request finds
    // no credential of theirs, while the usr/pwd user's session still serves.
    assert!(storage
        .get_credentials(&both.to_string(), "garmin")
        .await
        .unwrap()
        .is_none());
    assert!(storage
        .get_credentials(&usr_pwd.to_string(), "garmin")
        .await
        .unwrap()
        .is_some());
}

/// The usr/pwd health pull steps aside only for an athlete whose Garmin API
/// connection pushes their wellness. One who withheld (or never granted) the
/// health export gets no wellness push, so the usr/pwd connection keeps
/// pulling it; one who granted it is left to the pushes.
#[tokio::test]
async fn a_garmin_api_user_without_the_health_grant_keeps_the_usr_pwd_wellness_pull() {
    let resources = common::create_test_server_resources().await.unwrap();
    let both_with_grants = |email: &'static str, grants: &'static [&'static str]| {
        let resources = resources.clone();
        async move {
            let (user_id, _, tenant) = user_and_tenant(&resources, email).await;
            connect(
                &resources,
                user_id,
                tenant,
                oauth_providers::SCIOTTE_GARMIN,
                None,
            )
            .await;
            connect(
                &resources,
                user_id,
                tenant,
                oauth_providers::GARMIN,
                Some(email),
            )
            .await;
            let grants: Vec<String> = grants.iter().map(|g| (*g).to_owned()).collect();
            let recorded = resources
                .common
                .repos
                .provider_connections
                .set_provider_grants(user_id, tenant, oauth_providers::GARMIN, &grants)
                .await
                .unwrap();
            assert!(recorded, "the API connection's grants are recorded");
            user_id
        }
    };
    let withheld =
        both_with_grants("garmin-roster-no-health@example.com", &["ACTIVITY_EXPORT"]).await;
    let granted = both_with_grants(
        "garmin-roster-health@example.com",
        &["ACTIVITY_EXPORT", "HEALTH_EXPORT"],
    )
    .await;

    let storage = PierreSyncStorage::new(&resources.common.repos);
    let roster = storage.list_connected_users("garmin").await.unwrap();
    let ids: Vec<String> = roster.iter().map(|u| u.user_id.clone()).collect();

    assert!(
        ids.contains(&withheld.to_string()),
        "no health grant: nothing pushes their wellness, so the usr/pwd pull stays: {ids:?}"
    );
    assert!(
        !ids.contains(&granted.to_string()),
        "health granted: the pushes feed their wellness: {ids:?}"
    );
    assert!(
        storage
            .get_credentials(&withheld.to_string(), "garmin")
            .await
            .unwrap()
            .is_some(),
        "an on-request pull still finds the usr/pwd session without the health grant"
    );
    assert!(
        storage
            .get_credentials(&granted.to_string(), "garmin")
            .await
            .unwrap()
            .is_none(),
        "an on-request pull finds no credential while the API pushes their wellness"
    );
}

/// A listener standing in for Garmin's API that counts every connection it
/// receives and answers none.
async fn counting_garmin_api() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&seen);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            drop(stream);
        }
    });
    (format!("http://{addr}"), seen)
}

/// The routing order slice C asked for: a Garmin read of a user holding the
/// API backend goes to `GarminProvider`, which serves the cached pushes
/// through the head read and never calls Garmin for data — no Connect path,
/// no `user/id`, nothing.
#[tokio::test]
async fn a_garmin_api_users_read_serves_the_pushes_and_never_calls_garmin() {
    common::init_server_config();
    common::init_test_http_clients();
    let (api_base, calls) = counting_garmin_api().await;
    let mut config = common::test_server_config();
    common::set_provider_endpoints(
        &mut config,
        oauth_providers::GARMIN,
        ProviderEndpointOverride {
            api_base_url: Some(api_base),
            ..ProviderEndpointOverride::default()
        },
    );
    let resources = common::create_test_server_resources_with_config(config)
        .await
        .unwrap();
    let (user_id, _, tenant) = user_and_tenant(&resources, "garmin-routed@example.com").await;
    connect(
        &resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE_GARMIN,
        None,
    )
    .await;
    connect(
        &resources,
        user_id,
        tenant,
        oauth_providers::GARMIN,
        Some("garmin-user-routed"),
    )
    .await;
    let received = Utc::now() - Duration::days(2);
    let pushed = ActivityBuilder::new(
        "20123456789",
        "Pushed Ride",
        SportType::Ride,
        Utc::now() - Duration::days(2),
        3_600,
        oauth_providers::GARMIN,
    )
    .source(oauth_providers::GARMIN)
    .distance_meters(30_000.0)
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities_synced_at(
            user_id,
            &tenant,
            oauth_providers::GARMIN,
            &[pushed],
            received,
        )
        .await
        .unwrap();

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer");
    let response = GetActivitiesTool
        .execute(
            &runtime,
            &ctx,
            json!({ "provider": "garmin", "limit": 10, "mode": "summary" }),
        )
        .await;

    let payload = response.structured_content.expect("structured content");
    assert!(payload.get("error").is_none(), "{payload}");
    let ids: Vec<&str> = payload["activities"]
        .as_array()
        .expect("activities")
        .iter()
        .filter_map(|a| a["id"].as_str())
        .collect();
    assert_eq!(ids, ["20123456789"], "{payload}");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "a Garmin API read is served from the pushes, never from Garmin"
    );
    let last_sync = resources
        .common
        .repos
        .activity_cache
        .latest_activity_sync(user_id, &tenant, oauth_providers::GARMIN)
        .await
        .unwrap()
        .expect("the push recorded a sync");
    assert_eq!(
        last_sync.timestamp(),
        received.timestamp(),
        "the read does not restamp the pushes as a fresh sync"
    );
}
