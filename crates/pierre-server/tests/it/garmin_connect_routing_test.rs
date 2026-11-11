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
// the pushes. The grant is the one ongoing source: a usr/pwd session beside it
// serves nothing and its expiry never reads as a reconnect, while what it
// imported is served beside the pushes (carnet#1386).
#![cfg(all(feature = "provider-garmin", feature = "health-sync"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;
use crate::helpers::sciotte_mock::{seed_scrape_session, spawn_mock_scraper_with_activity};

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
    Activity, ActivityBuilder, ConnectionStatus, ConnectionType, SportType, TenantId, User,
    UserOAuthToken,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_middleware::provider_link_token::mint_connect_link_token;
use pierre_providers::core::ActivityQueryParams;
use pierre_routes_auth::AuthRoutes;
use pierre_services::health_sync::PierreSyncStorage;
use pierre_tool_runtime::activity_fetch::fetch_provider_activities;
use pierre_tool_runtime::group_fitness::fetch_member_snapshots;
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

/// The health sync follows the one Garmin rule (carnet#1386, carnet#1257):
/// the API grant serves Garmin whatever usr/pwd session stands beside it, so
/// an API user is left to the pushes, live session or not; only a usr/pwd-only
/// athlete is pulled.
#[tokio::test]
async fn a_garmin_api_user_leaves_the_health_sync_roster_whatever_session_stands_beside_it() {
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
    // Holds both, the session live: the grant serves, its pushes feed the days.
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
    // Holds both, the session dead: the API serves, its pushes feed the days.
    let (dead, _, dead_tenant) =
        user_and_tenant(&resources, "garmin-roster-dead@example.com").await;
    connect(
        &resources,
        dead,
        dead_tenant,
        oauth_providers::SCIOTTE_GARMIN,
        None,
    )
    .await;
    mark_dead(
        &resources,
        dead,
        dead_tenant,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await;
    connect(
        &resources,
        dead,
        dead_tenant,
        oauth_providers::GARMIN,
        Some("garmin-user-dead"),
    )
    .await;

    let storage = PierreSyncStorage::new(&resources.common.repos);
    let roster = storage.list_connected_users("garmin").await.unwrap();
    let ids: Vec<String> = roster.iter().map(|u| u.user_id.clone()).collect();

    assert!(
        ids.contains(&usr_pwd.to_string()),
        "bug: a usr/pwd-only athlete keeps their session's pull: {ids:?}"
    );
    assert!(
        !ids.contains(&both.to_string()),
        "bug: a live session beside the API grant serves nothing; the pushes feed the days: {ids:?}"
    );
    assert!(
        !ids.contains(&dead.to_string()),
        "bug: a dead session beside the API grant leaves the days to the pushes: {ids:?}"
    );

    // A sync asked for on request reads the same way: no session beside the
    // grant, live or dead, has anything of its own to pull.
    assert!(
        storage
            .get_credentials(&both.to_string(), "garmin")
            .await
            .unwrap()
            .is_none(),
        "bug: a live session beside the API grant has nothing of its own to pull"
    );
    assert!(
        storage
            .get_credentials(&dead.to_string(), "garmin")
            .await
            .unwrap()
            .is_none(),
        "bug: a dead session beside the API grant has nothing of its own to pull"
    );
    assert!(storage
        .get_credentials(&usr_pwd.to_string(), "garmin")
        .await
        .unwrap()
        .is_some());
}

/// Where the API serves Garmin, the usr/pwd health
/// pull steps aside only for an athlete whose API connection pushes their
/// wellness. One who withheld (or never granted) the health export gets no
/// wellness push, so the usr/pwd connection keeps pulling it; one who granted
/// it is left to the pushes.
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
            mark_dead(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await;
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
    // The usr/pwd session stays theirs to pull, but this one is flagged for a
    // reconnect: a sync hands out no credential for it, so it signs in to
    // nothing until the athlete reconnects (or the throttled retry re-arms it).
    assert!(
        storage
            .get_credentials(&withheld.to_string(), "garmin")
            .await
            .unwrap()
            .is_none(),
        "bug: a sync must not read a usr/pwd session flagged for a reconnect"
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

/// The routing order slice C asked for: a Garmin read of a user whose API
/// grant serves alone goes to `GarminProvider`, which serves the cached pushes
/// through the head read and never calls Garmin for data — no Connect path,
/// no `user/id`, nothing. A session beside the grant would serve first
/// (carnet#1494), so this athlete holds the grant alone.
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

/// Mark the user's `provider` connection dead, as a failed scrape or refresh does.
async fn mark_dead(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    provider: &str,
) {
    resources
        .common
        .repos
        .provider_connections
        .mark_needs_reauth(
            user_id,
            tenant,
            provider,
            Some("session_expired"),
            Utc::now(),
        )
        .await
        .unwrap();
}

/// An athlete whose usr/pwd session died and who then connects Garmin's OAuth
/// keeps the session's reconnect up: the session still serves the card first
/// (carnet#1494), so the card reads "Reconnect" until they sign in again, the
/// grant only standing behind it. The flag follows the backend that serves
/// the card, the session, whatever the grant does.
#[tokio::test]
async fn a_garmin_oauth_grant_beside_a_dead_usr_pwd_session_keeps_its_reconnect_up() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, user, tenant) =
        user_and_tenant(&resources, "garmin-reconnect-oauth@example.com").await;
    connect(
        &resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE_GARMIN,
        None,
    )
    .await;
    mark_dead(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await;

    let cards_before = cards(&resources, &user).await;
    let before = card(&cards_before, oauth_providers::SCIOTTE_GARMIN);
    assert_eq!(
        before["needs_reauth"], true,
        "bug: a dead usr/pwd session must ask for a reconnect: {before}"
    );

    connect(
        &resources,
        user_id,
        tenant,
        oauth_providers::GARMIN,
        Some("garmin-user-reconnect"),
    )
    .await;
    let cards_after = cards(&resources, &user).await;
    let after = card(&cards_after, oauth_providers::SCIOTTE_GARMIN);
    assert_eq!(
        after["connected"], true,
        "bug: the Garmin card must read connected: {after}"
    );
    assert_eq!(
        after["connected_backend"],
        oauth_providers::SCIOTTE_GARMIN,
        "bug: the session must serve the card ahead of the grant behind it: {after}"
    );
    assert_eq!(
        after["history"]["state"], "not_applicable",
        "a session serving the card reads its own history: {after}"
    );
    assert_eq!(
        after["needs_reauth"], true,
        "bug: a live grant hid the dead session's reconnect: {after}"
    );
    assert!(
        after.get("session_needs_reauth").is_none(),
        "the card carries one reconnect flag, the serving backend's: {after}"
    );

    mark_dead(&resources, user_id, tenant, oauth_providers::GARMIN).await;
    let cards_dead = cards(&resources, &user).await;
    let dead = card(&cards_dead, oauth_providers::SCIOTTE_GARMIN);
    assert_eq!(
        dead["needs_reauth"], true,
        "bug: the dead session still asks for its reconnect: {dead}"
    );
}

/// A `get_activities` call naming Garmin, as the coach makes it.
async fn read_garmin(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) -> Value {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer");
    GetActivitiesTool
        .execute(
            &runtime,
            &ctx,
            json!({ "provider": "garmin", "limit": 10, "mode": "summary" }),
        )
        .await
        .structured_content
        .expect("structured content")
}

/// The ids a `get_activities` payload lists, in order.
fn listed_ids(payload: &Value) -> Vec<String> {
    payload["activities"]
        .as_array()
        .unwrap_or_else(|| panic!("activities: {payload}"))
        .iter()
        .filter_map(|a| a["id"].as_str().map(str::to_owned))
        .collect()
}

/// Store `rows` under the cache key `key` for the user, as a push or a usr/pwd
/// session's capture stores them.
async fn store_under(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    key: &str,
    rows: &[Activity],
) {
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities_synced_at(user_id, &tenant, key, rows, Utc::now())
        .await
        .unwrap();
}

/// A grant holder whose session the carnet#1386 import retired keeps the
/// history it imported: their Garmin read serves what the session stored
/// under its mirror key beside what Garmin pushed (`history_keys(garmin)`,
/// kept by carnet#1494). A workout both recorded is one session, the API's
/// copy standing for it; nothing calls Garmin or the scraper.
#[tokio::test]
async fn a_grant_holders_garmin_read_serves_the_sessions_history_merged_with_the_pushes() {
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
    let (user_id, _, tenant) = user_and_tenant(&resources, "garmin-dual-key@example.com").await;
    connect(
        &resources,
        user_id,
        tenant,
        oauth_providers::GARMIN,
        Some("garmin-user-dual"),
    )
    .await;
    let ride_start = Utc::now() - Duration::days(2);
    let ride = |id: &str, key: &str| {
        ActivityBuilder::new(id, "Morning Ride", SportType::Ride, ride_start, 3_600, key)
            .source(oauth_providers::GARMIN)
            .distance_meters(30_000.0)
            .build()
    };
    store_under(
        &resources,
        user_id,
        tenant,
        oauth_providers::GARMIN,
        &[ride("20123456789", oauth_providers::GARMIN)],
    )
    .await;
    let older_run = ActivityBuilder::new(
        "18000000001",
        "Imported Run",
        SportType::Run,
        Utc::now() - Duration::days(20),
        2_400,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .source(oauth_providers::GARMIN)
    .distance_meters(8_000.0)
    .build();
    store_under(
        &resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE_GARMIN,
        &[
            ride("18000000002", oauth_providers::SCIOTTE_GARMIN),
            older_run,
        ],
    )
    .await;

    let payload = read_garmin(&resources, user_id, tenant).await;

    assert!(payload.get("error").is_none(), "{payload}");
    assert_eq!(
        listed_ids(&payload),
        ["20123456789", "18000000001"],
        "bug: the session's history must be served beside the pushes, and the ride both \
         recorded must be one session the API's copy stands for: {payload}"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the Garmin API is never called for data"
    );
}

/// A usr/pwd-only Garmin athlete (no grant) keeps reading through their
/// session: the scrape answers the read.
#[tokio::test]
async fn a_usr_pwd_only_garmin_athlete_still_reads_through_the_session() {
    common::init_server_config();
    common::init_test_http_clients();
    let scraper = spawn_mock_scraper_with_activity(json!({
        "id": "18000000001",
        "name": "Morning Run",
        "sport_type": "run",
        "start_date": (Utc::now() - Duration::days(3)).to_rfc3339(),
        "duration_seconds": 2_400,
        "provider": "garmin",
        "distance_meters": 8_000.0,
    }))
    .await;
    let config = common::test_server_config_with_sciotte(&scraper);
    let resources = common::create_test_server_resources_with_config(config)
        .await
        .unwrap();
    let (user_id, _, tenant) = user_and_tenant(&resources, "garmin-usrpwd-only@example.com").await;
    connect(
        &resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE_GARMIN,
        None,
    )
    .await;
    seed_scrape_session(
        &resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE_GARMIN,
        "garmin-usrpwd-session",
    )
    .await;

    let payload = read_garmin(&resources, user_id, tenant).await;

    assert!(payload.get("error").is_none(), "{payload}");
    assert_eq!(
        listed_ids(&payload),
        ["18000000001"],
        "bug: with no grant the usr/pwd session is Garmin's source: {payload}"
    );
}

/// Without a usr/pwd session the grant still serves, with the honest empty
/// answer for a store nothing was pushed to.
#[tokio::test]
async fn a_garmin_api_grant_with_nothing_pushed_and_no_session_answers_empty() {
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
    let (user_id, _, tenant) = user_and_tenant(&resources, "garmin-empty@example.com").await;
    connect(
        &resources,
        user_id,
        tenant,
        oauth_providers::GARMIN,
        Some("garmin-user-empty"),
    )
    .await;

    let payload = read_garmin(&resources, user_id, tenant).await;

    assert!(payload.get("error").is_none(), "{payload}");
    assert!(
        listed_ids(&payload).is_empty(),
        "nothing was pushed, and no session reads the history: {payload}"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the Garmin API is never called for data"
    );
}

/// The card reads the same rule (carnet#1386): the grant serves the card
/// whatever usr/pwd session stands beside it, its history reading `needed`
/// while nothing is copied, on the hosted picker too, where the card opens
/// the history step. What the session stored makes it `imported`, and the
/// session dying asks nothing: the card stays "Connected" until the grant
/// itself is flagged.
#[tokio::test]
async fn the_garmin_card_follows_the_session_once_one_stands_beside_the_grant() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, user, tenant) = user_and_tenant(&resources, "garmin-card-flag@example.com").await;
    connect(
        &resources,
        user_id,
        tenant,
        oauth_providers::GARMIN,
        Some("garmin-user-card"),
    )
    .await;

    let alone = cards(&resources, &user).await;
    let alone = card(&alone, oauth_providers::SCIOTTE_GARMIN);
    assert_eq!(alone["connected"], true, "{alone}");
    assert_eq!(
        alone["connected_backend"],
        oauth_providers::GARMIN,
        "{alone}"
    );
    assert_eq!(
        alone["history"]["state"], "needed",
        "bug: an empty Garmin grant alone must ask for its history: {alone}"
    );
    assert!(alone.get("oauth_awaiting_activities").is_none(), "{alone}");

    let page = picker_page(&resources, user_id, tenant).await;
    let target = page
        .find(r#""target":"garmin""#)
        .unwrap_or_else(|| panic!("a Garmin card on the picker: {page}"));
    let card_start = page[..target].rfind('{').unwrap();
    let card_end = target + page[target..].find('}').unwrap();
    let picker_card = &page[card_start..card_end];
    assert!(
        picker_card.contains(r#""history_state":"needed""#)
            && picker_card.contains(r#""history_sign_in":true"#),
        "the hosted picker opens the history step with its one-time sign-in: {picker_card}"
    );
    assert!(
        !picker_card.contains("awaits_activities"),
        "the picker card names its history state alone: {picker_card}"
    );
    assert!(
        page.contains(r#""tagImportHistory":"Import my history""#),
        "the card's tag is worded in the page's strings"
    );

    connect(
        &resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE_GARMIN,
        None,
    )
    .await;
    let beside = cards(&resources, &user).await;
    let beside = card(&beside, oauth_providers::SCIOTTE_GARMIN);
    assert_eq!(
        beside["connected_backend"],
        oauth_providers::SCIOTTE_GARMIN,
        "bug: the live usr/pwd session serves the card before the grant: {beside}"
    );
    assert_eq!(
        beside["history"]["state"], "not_applicable",
        "a session serving the card reads its own history: {beside}"
    );

    let imported = ActivityBuilder::new(
        "18999999999",
        "Imported Ride",
        SportType::Ride,
        Utc::now() - Duration::days(40),
        1_800,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .source(oauth_providers::GARMIN)
    .build();
    store_under(
        &resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE_GARMIN,
        &[imported],
    )
    .await;
    let after_import = cards(&resources, &user).await;
    let after_import = card(&after_import, oauth_providers::SCIOTTE_GARMIN);
    assert_eq!(
        after_import["history"]["state"], "not_applicable",
        "the session's own rows are its window, not a history to import: {after_import}"
    );

    mark_dead(&resources, user_id, tenant, oauth_providers::GARMIN).await;
    let grant_dead = cards(&resources, &user).await;
    let grant_dead = card(&grant_dead, oauth_providers::SCIOTTE_GARMIN);
    assert_eq!(
        grant_dead["needs_reauth"], false,
        "bug: a flagged grant behind a live session asked the athlete to reconnect: {grant_dead}"
    );

    mark_dead(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await;
    let dead = cards(&resources, &user).await;
    let dead = card(&dead, oauth_providers::SCIOTTE_GARMIN);
    assert_eq!(
        dead["connected_backend"],
        oauth_providers::SCIOTTE_GARMIN,
        "{dead}"
    );
    assert_eq!(
        dead["needs_reauth"], true,
        "bug: the dead session serving the card must ask for its reconnect: {dead}"
    );
}

/// The rule holds for every mirror pair: a dead Strava usr/pwd session beside
/// a live Strava grant still serves the card first, so the card asks for the
/// session's reconnect (carnet#1494).
#[tokio::test]
async fn a_dead_strava_usr_pwd_session_beside_a_live_grant_asks_for_its_reconnect() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, user, tenant) =
        user_and_tenant(&resources, "strava-session-reauth@example.com").await;
    connect(&resources, user_id, tenant, oauth_providers::SCIOTTE, None).await;
    connect(&resources, user_id, tenant, oauth_providers::STRAVA, None).await;
    mark_dead(&resources, user_id, tenant, oauth_providers::SCIOTTE).await;

    let cards = cards(&resources, &user).await;
    let strava = card(&cards, oauth_providers::SCIOTTE);
    assert_eq!(
        strava["connected_backend"],
        oauth_providers::SCIOTTE,
        "bug: the session must serve the card before the grant behind it: {strava}"
    );
    assert_eq!(
        strava["needs_reauth"], true,
        "bug: the dead session serving the card must ask for its reconnect: {strava}"
    );
}

/// A grant holder beside a live usr/pwd session, with a ride Garmin pushed
/// under `garmin` and the session's capture of it, plus an older imported run,
/// under `sciotte_garmin`.
async fn dual_key_athlete(resources: &Arc<ServerContext>, email: &str) -> (Uuid, User, TenantId) {
    let (user_id, user, tenant) = user_and_tenant(resources, email).await;
    connect(
        resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE_GARMIN,
        None,
    )
    .await;
    connect(
        resources,
        user_id,
        tenant,
        oauth_providers::GARMIN,
        Some("garmin-user-dual-dead"),
    )
    .await;
    let ride_start = Utc::now() - Duration::days(2);
    let ride = |id: &str, key: &str| {
        ActivityBuilder::new(id, "Morning Ride", SportType::Ride, ride_start, 3_600, key)
            .source(oauth_providers::GARMIN)
            .distance_meters(30_000.0)
            .build()
    };
    store_under(
        resources,
        user_id,
        tenant,
        oauth_providers::GARMIN,
        &[ride("20123456789", oauth_providers::GARMIN)],
    )
    .await;
    let older_run = ActivityBuilder::new(
        "18000000001",
        "Imported Run",
        SportType::Run,
        Utc::now() - Duration::days(20),
        2_400,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .source(oauth_providers::GARMIN)
    .distance_meters(8_000.0)
    .build();
    store_under(
        resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE_GARMIN,
        &[
            ride("18000000002", oauth_providers::SCIOTTE_GARMIN),
            older_run,
        ],
    )
    .await;
    (user_id, user, tenant)
}

/// The ids stored under the cache key `key`, newest first.
async fn stored_ids(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    key: &str,
) -> Vec<String> {
    resources
        .common
        .repos
        .activity_cache
        .get_cached_activities(
            user_id,
            &tenant,
            Some(key),
            Utc::now() - Duration::days(400),
            Utc::now(),
            100,
        )
        .await
        .unwrap()
        .iter()
        .map(|a| a.id().to_owned())
        .collect()
}

/// A read naming the usr/pwd session beside a Garmin grant is the grant's
/// (carnet#1386): it serves the grant's merged window, never re-arms a grant
/// flagged for a withdrawn permission, and writes nothing under the
/// session's key. Reading it as the session once re-armed the grant from the
/// session's read and wrote Garmin's pushes under `sciotte_garmin`.
#[tokio::test]
async fn a_read_naming_the_session_beside_a_flagged_grant_is_the_grants() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, _, tenant) = dual_key_athlete(&resources, "garmin-beside-read@example.com").await;
    resources
        .common
        .repos
        .provider_connections
        .mark_needs_reauth(
            user_id,
            tenant,
            oauth_providers::GARMIN,
            Some("permission_withdrawn"),
            Utc::now(),
        )
        .await
        .unwrap();
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let params = ActivityQueryParams {
        limit: Some(10),
        offset: None,
        before: None,
        after: Some((Utc::now() - Duration::days(60)).timestamp()),
    };

    let served = fetch_provider_activities(
        &runtime,
        oauth_providers::SCIOTTE_GARMIN,
        user_id,
        &tenant.to_string(),
        &params,
    )
    .await
    .expect("the grant's stored window answers");

    let ids: Vec<&str> = served.iter().map(Activity::id).collect();
    assert_eq!(
        ids,
        ["20123456789", "18000000001"],
        "the grant's window, its session's history merged in"
    );
    let garmin = resources
        .common
        .repos
        .provider_connections
        .get_for_user(user_id, Some(tenant))
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.provider == oauth_providers::GARMIN)
        .expect("the Garmin connection");
    assert_eq!(
        garmin.status,
        ConnectionStatus::NeedsReauth,
        "bug: a read naming the session re-armed the flagged grant"
    );
    assert_eq!(
        stored_ids(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        ["18000000002", "18000000001"],
        "bug: the read wrote under the session's key"
    );
}

/// A dead Garmin grant with a usr/pwd session row beside it keeps serving
/// what the athlete holds (carnet#1386): with no other provider to stand in,
/// the turn answers through that session row from the grant's stored window,
/// the session's imported history merged in, rather than a bare reconnect. A
/// grant with no session row beside it gets the reconnect every dead primary
/// gets.
#[tokio::test]
async fn a_dead_garmin_grant_still_serves_the_stored_history() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, _, tenant) = dual_key_athlete(&resources, "garmin-dead-grant@example.com").await;
    mark_dead(&resources, user_id, tenant, oauth_providers::GARMIN).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer");

    let payload = GetActivitiesTool
        .execute(&runtime, &ctx, json!({ "limit": 10, "mode": "summary" }))
        .await
        .structured_content
        .expect("structured content");

    assert!(payload.get("error").is_none(), "{payload}");
    assert_eq!(
        listed_ids(&payload),
        ["20123456789", "18000000001"],
        "bug: the dead grant's stored history must still be served: {payload}"
    );
}

/// A grant holder whose Garmin grant is flagged, with the pool full, is
/// recommended the usr/pwd sign-in: a dead grant alone reads through the
/// mirror (carnet#1494), so the grant no longer pins the card to its OAuth.
#[tokio::test]
async fn a_flagged_garmin_grant_alone_is_recommended_the_sign_in_with_the_pool_full() {
    let resources = common::create_test_server_resources_with_config(config_with_garmin_cap(1))
        .await
        .unwrap();
    let (holder, _, holder_tenant) =
        user_and_tenant(&resources, "garmin-seat-other@example.com").await;
    connect(
        &resources,
        holder,
        holder_tenant,
        oauth_providers::GARMIN,
        Some("garmin-user-other"),
    )
    .await;
    let (user_id, user, tenant) =
        user_and_tenant(&resources, "garmin-flagged-full@example.com").await;
    connect(
        &resources,
        user_id,
        tenant,
        oauth_providers::GARMIN,
        Some("garmin-user-flagged"),
    )
    .await;
    mark_dead(&resources, user_id, tenant, oauth_providers::GARMIN).await;

    let cards = cards(&resources, &user).await;
    let garmin = card(&cards, oauth_providers::SCIOTTE_GARMIN);
    assert_eq!(garmin["needs_reauth"], true, "{garmin}");
    assert_eq!(garmin["seats_left"], 0, "the pool is full: {garmin}");
    assert_eq!(
        garmin["recommended_backend"], "mirror",
        "bug: a dead grant alone kept the card on its OAuth: {garmin}"
    );
}

/// The group agent's connection alerts follow the same rule as the card
/// (carnet#1494): a flagged grant behind a live usr/pwd session asks nothing
/// of the athlete, so the room is not told to reconnect Garmin until the
/// session that serves first is flagged.
#[tokio::test]
async fn a_flagged_grant_behind_a_garmin_session_raises_no_group_alert() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, _, tenant) = user_and_tenant(&resources, "garmin-group-alert@example.com").await;
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
        Some("garmin-user-group"),
    )
    .await;
    mark_dead(&resources, user_id, tenant, oauth_providers::GARMIN).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();

    let quiet = fetch_member_snapshots(&runtime, &[user_id], tenant).await;
    assert!(
        quiet[0].needs_reauth_providers.is_empty(),
        "bug: the grant behind the session told the room to reconnect: {:?}",
        quiet[0].needs_reauth_providers
    );

    mark_dead(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await;
    let flagged = fetch_member_snapshots(&runtime, &[user_id], tenant).await;
    assert_eq!(
        flagged[0].needs_reauth_providers,
        ["garmin"],
        "a dead session that serves first is the athlete's to reconnect"
    );
}
