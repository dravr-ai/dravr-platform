// ABOUTME: A dead OAuth grant reconnects only through the hosted sign-in: chat, card and every OAuth start a client opens
// ABOUTME: Pins the sign-in that replaces a dead grant keeping its data, and which halves a reclaim, a revocation or the athlete clear

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Sciotte first: the reconnect (carnet#1494, JF's directives 2 and 3).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// A grant whose refresh the provider refused is dead, and it reconnects only
// through its mirror's hosted sign-in: the chat's reconnect offer, the card's
// recommendation, and the OAuth starts a shipped client still opens for it,
// which answer with the sign-in in the authorization's place. The sign-in
// that replaces a dead grant deletes the grant's token and connection rows
// and keeps everything it stored. A seat reclaim and a revocation withdraw
// the grant alone and keep the athlete's session; the athlete's own
// disconnect clears both halves. A grant-only athlete whose grant is
// withdrawn is told once to sign in again.
#![cfg(all(feature = "provider-garmin", feature = "health-sync"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::garmin_history_import_test::{
    athlete, connections, hold_grant, hold_session, reconnect_notices, rows_under, server,
    server_with_garmin_oauth, session_token_held, sign_in, spawn_switchable_scraper, ScraperSwitch,
    LIVE_SESSION,
};
use crate::helpers::axum_test::AxumTestRequest;
use crate::reconnect_prompt_on_served_turn_test::{
    loop_result, offer_target, recover_in, token_after, RECOVERY_LINK_SECRET,
};

use std::sync::Arc;

use chrono::Utc;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{ActivityBuilder, ConnectionStatus, SportType, TenantId};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::grant_withdrawal::GrantWithdrawal;
use pierre_middleware::provider_link_token::{verify_link_token, HOSTED_SIGNIN_PATH};
use pierre_routes_auth::AuthRoutes;
use pierre_services::oauth_flow::OAuthService;
use pierre_services::provider_revocation::DisconnectReason;
use pierre_services::user_removal::ProviderDisconnector;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::Value;
use uuid::Uuid;

/// Flag the athlete's Garmin grant dead, as a refused refresh does.
async fn kill_grant(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) {
    resources
        .common
        .repos
        .provider_connections
        .mark_needs_reauth(
            user_id,
            tenant,
            oauth_providers::GARMIN,
            Some("refresh_refused"),
            Utc::now(),
        )
        .await
        .unwrap();
}

/// The hosted sign-in target a link-token minted under `secret` names, after
/// checking it signs in `user_id`.
fn signed_in_target(token: &str, secret: &str, user_id: Uuid) -> String {
    let claims = verify_link_token(token, secret, "sciotte").expect("a hosted-login token");
    assert_eq!(
        claims.sub,
        user_id.to_string(),
        "bug: the link signs in another athlete"
    );
    claims.tgt
}

/// The token of a hosted sign-in URL on this server.
fn hosted_sign_in_token(resources: &Arc<ServerContext>, url: &str) -> String {
    let prefix = format!(
        "{}{HOSTED_SIGNIN_PATH}?token=",
        resources.common.config.base_url.trim_end_matches('/')
    );
    let token = url
        .strip_prefix(&prefix)
        .unwrap_or_else(|| panic!("bug: the start does not open the hosted sign-in: {url}"));
    urlencoding::decode(token).unwrap().into_owned()
}

/// The Garmin card `GET /api/providers` serves the bearer of `token`.
async fn garmin_card(resources: &Arc<ServerContext>, token: &str) -> Value {
    let resp = AxumTestRequest::get("/api/providers")
        .header("authorization", &format!("Bearer {token}"))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    let body: Value = serde_json::from_str(&resp.text()).unwrap();
    body["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|card| card["provider"] == oauth_providers::SCIOTTE_GARMIN)
        .expect("the Garmin card is served")
        .clone()
}

/// A dead Garmin grant's chat reconnect is the hosted Garmin sign-in, and
/// its card asks for a reconnect that recommends the sign-in, though a seat
/// is free on Garmin's OAuth.
#[tokio::test]
async fn a_dead_grant_reconnects_through_the_hosted_sign_in_in_chat_and_on_its_card() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server_with_garmin_oauth(&scraper).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    kill_grant(&resources, user_id, tenant).await;

    let mut result = loop_result(Some(oauth_providers::GARMIN), None);
    let prompt = recover_in(&resources, user_id, tenant, "en", &mut result)
        .await
        .prompt
        .expect("the athlete gets a reconnect control");
    let link = token_after(
        &offer_target(&resources, &prompt.url).await,
        HOSTED_SIGNIN_PATH,
    );
    assert_eq!(
        signed_in_target(&link, RECOVERY_LINK_SECRET, user_id),
        "garmin",
        "bug: a dead grant's chat reconnect is not the Garmin sign-in"
    );

    let card = garmin_card(&resources, &token).await;
    assert_eq!(card["connected_backend"], oauth_providers::GARMIN, "{card}");
    assert_eq!(
        card["needs_reauth"], true,
        "bug: the dead grant reads as live: {card}"
    );
    assert_eq!(
        card["recommended_backend"], "mirror",
        "bug: the card sends a dead grant's reconnect to the OAuth: {card}"
    );
}

/// The OAuth starts a client opens for a dead grant answer with the hosted
/// sign-in: the mobile init in its own JSON shape, its URL in
/// `authorization_url` (a shipped build opens whatever it is given), and the
/// web launch with a redirect to it. A live grant still starts the OAuth.
#[tokio::test]
async fn an_oauth_start_of_a_dead_grant_answers_with_the_hosted_sign_in() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server_with_garmin_oauth(&scraper).await;
    let secret = resources.auth.admin_jwt_secret.to_string();
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;

    let mobile_init = || {
        AxumTestRequest::get("/api/oauth/mobile/init/garmin")
            .header("authorization", &format!("Bearer {token}"))
            .send(AuthRoutes::routes(resources.auth_routes_context()))
    };
    let live: Value = serde_json::from_str(&mobile_init().await.text()).unwrap();
    let live_url = live["authorization_url"].as_str().unwrap_or_default();
    assert!(
        !live_url.contains(HOSTED_SIGNIN_PATH) && live_url.contains("client_id="),
        "bug: a live grant's start is not Garmin's authorization: {live}"
    );

    kill_grant(&resources, user_id, tenant).await;
    let resp = mobile_init().await;
    assert_eq!(resp.status(), 200, "{}", resp.text());
    let dead: Value = serde_json::from_str(&resp.text()).unwrap();
    assert_eq!(
        dead["provider"], "garmin",
        "bug: the JSON shape changed: {dead}"
    );
    let url = dead["authorization_url"]
        .as_str()
        .unwrap_or_else(|| panic!("bug: no authorization_url: {dead}"));
    assert_eq!(
        signed_in_target(&hosted_sign_in_token(&resources, url), &secret, user_id),
        "garmin",
        "bug: the mobile start of a dead grant is not the Garmin sign-in"
    );

    let resp = AxumTestRequest::get("/api/oauth/authorize/garmin")
        .header("authorization", &format!("Bearer {token}"))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    assert_eq!(resp.status(), 302, "{}", resp.text());
    let location = resp.header("location").expect("a redirect");
    assert_eq!(
        signed_in_target(
            &hosted_sign_in_token(&resources, location),
            &secret,
            user_id
        ),
        "garmin",
        "bug: the web start of a dead grant is not the Garmin sign-in"
    );
}

/// A sign-in over a dead grant deletes the grant's token and connection rows
/// and keeps every activity it stored (directive 3); the new session serves.
/// A live grant beside a sign-in is kept, as the session's fallback.
#[tokio::test]
async fn a_sign_in_over_a_dead_grant_deletes_its_rows_and_keeps_its_data() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    let ride = ActivityBuilder::new(
        "garmin-pushed-ride".to_owned(),
        "Pushed ride".to_owned(),
        SportType::Ride,
        Utc::now() - chrono::Duration::days(3),
        3600,
        "garmin".to_owned(),
    )
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant, oauth_providers::GARMIN, &[ride])
        .await
        .unwrap();
    kill_grant(&resources, user_id, tenant).await;

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "bug: the sign-in is refused: {body}");
    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "the session serves"
    );
    assert!(
        resources
            .common
            .repos
            .oauth_tokens
            .get_token(user_id, tenant, oauth_providers::GARMIN)
            .await
            .unwrap()
            .is_none(),
        "bug: the dead grant's token row is kept"
    );
    assert!(
        !connections(&resources, user_id, tenant)
            .await
            .iter()
            .any(|(provider, _)| provider == oauth_providers::GARMIN),
        "bug: the dead grant's connection row is kept"
    );
    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::GARMIN).await,
        1,
        "bug: the dead grant's data went with its rows"
    );

    let (live, _user, live_tenant, live_token) = athlete(&resources).await;
    hold_grant(&resources, live, live_tenant).await;
    let (status, body) = sign_in(&resources, &live_token).await;
    assert_eq!(status, 200, "{body}");
    assert!(
        resources
            .common
            .repos
            .oauth_tokens
            .get_token(live, live_tenant, oauth_providers::GARMIN)
            .await
            .unwrap()
            .is_some(),
        "bug: a live grant is deleted by a sign-in beside it"
    );
}

/// A seat reclaim and a revocation withdraw the grant alone, and the session
/// keeps serving; the athlete's own disconnect clears both halves of the
/// card.
#[tokio::test]
async fn a_reclaim_or_a_revocation_keeps_the_session_and_the_athletes_disconnect_clears_both() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let service = OAuthService::new(resources.data(), resources.common.config.clone());

    for reason in [
        DisconnectReason::SeatReclaim,
        DisconnectReason::ProviderRevoked,
    ] {
        let (user_id, _user, tenant, _token) = athlete(&resources).await;
        hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
        hold_grant(&resources, user_id, tenant).await;
        service
            .disconnect_provider(
                user_id,
                oauth_providers::GARMIN,
                Some(tenant.as_uuid()),
                reason,
            )
            .await
            .unwrap();
        assert_eq!(
            connections(&resources, user_id, tenant).await,
            vec![(
                oauth_providers::SCIOTTE_GARMIN.to_owned(),
                ConnectionStatus::Active
            )],
            "bug: a {reason:?} took the athlete's session with the grant"
        );
        assert!(
            session_token_held(&resources, user_id, tenant).await,
            "bug: a {reason:?} deleted the session's token"
        );
    }

    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
    hold_grant(&resources, user_id, tenant).await;
    service
        .disconnect_provider(
            user_id,
            oauth_providers::GARMIN,
            Some(tenant.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .unwrap();
    assert_eq!(
        connections(&resources, user_id, tenant).await,
        vec![],
        "bug: the athlete's disconnect left half the card connected"
    );
    assert!(!session_token_held(&resources, user_id, tenant).await);
}

/// A grant-only athlete whose grant the provider revoked is told once to sign
/// in again: the withdrawal is the claim, so a redelivered event sends
/// nothing. An athlete whose session serves beside the grant is told
/// nothing.
#[tokio::test]
async fn a_withdrawn_grant_tells_a_grant_only_athlete_once() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let withdrawal = GrantWithdrawal::new(
        OAuthService::new(resources.data(), resources.common.config.clone()),
        runtime,
    );

    let (alone, _user, alone_tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, alone, alone_tenant).await;
    for _delivery in 0..2 {
        withdrawal
            .disconnect(
                alone,
                oauth_providers::GARMIN,
                alone_tenant,
                DisconnectReason::ProviderRevoked,
            )
            .await
            .unwrap();
    }
    assert_eq!(
        reconnect_notices(&resources, alone, alone_tenant).await,
        1,
        "bug: a grant-only athlete is told other than once"
    );

    let (beside, _user, beside_tenant, _token) = athlete(&resources).await;
    hold_session(&resources, beside, beside_tenant, LIVE_SESSION).await;
    hold_grant(&resources, beside, beside_tenant).await;
    withdrawal
        .disconnect(
            beside,
            oauth_providers::GARMIN,
            beside_tenant,
            DisconnectReason::ProviderRevoked,
        )
        .await
        .unwrap();
    assert_eq!(
        reconnect_notices(&resources, beside, beside_tenant).await,
        0,
        "bug: an athlete whose session serves is asked to reconnect"
    );
}
