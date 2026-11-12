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
//
// Garmin's grant writes what its session cannot (workout and calendar push),
// so Garmin signs in first and authorizes its grant right after (JF's
// directive 8): the sign-in's answer names that authorization while the
// grant is owed, the hosted sign-in's carries its URL, the picker keeps a
// connected card with an owed grant pickable, an OAuth start of a dead grant
// beside a live session authorizes it, and a write the grant alone makes is
// refused with its authorization link.
#![cfg(all(feature = "provider-garmin", feature = "health-sync"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::garmin_sign_in_test::{
    athlete, calls_matching, connections, hold_grant, hold_session, reconnect_notices, rows_under,
    server, server_with_garmin_oauth, session_token_held, sign_in, spawn_switchable_scraper,
    ScraperSwitch, LIVE_SESSION,
};
use crate::helpers::axum_test::AxumTestRequest;
use crate::reconnect_prompt_on_served_turn_test::{
    loop_result, offer_target, recover_in, token_after, RECOVERY_LINK_SECRET,
};

use std::sync::Arc;

use chrono::Utc;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{ActivityBuilder, ConnectionStatus, SportType, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::grant_withdrawal::GrantWithdrawal;
use pierre_middleware::provider_link_token::{
    mint_connect_link_token, mint_connect_oauth_init_url, mint_link_token, verify_link_token,
    MintProviderLinkTokenArgs, HOSTED_SIGNIN_PATH,
};
use pierre_providers::backend_resolver;
use pierre_providers::spi::ProviderCapabilities;
use pierre_routes_auth::AuthRoutes;
use pierre_services::oauth_flow::OAuthService;
use pierre_services::provider_revocation::DisconnectReason;
use pierre_services::user_removal::ProviderDisconnector;
use pierre_tool_runtime::implementations::oauth_authorize::with_grant_link;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
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
/// nothing. An athlete whose session serves beside the Garmin grant is told
/// once too, that the reads go on and only the grant's workout and calendar
/// writes need it again (JF's directive 8), and a redelivery sends nothing.
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
    for _delivery in 0..2 {
        withdrawal
            .disconnect(
                beside,
                oauth_providers::GARMIN,
                beside_tenant,
                DisconnectReason::ProviderRevoked,
            )
            .await
            .unwrap();
    }
    assert_eq!(
        reconnect_notices(&resources, beside, beside_tenant).await,
        1,
        "bug: an athlete whose session serves is not told once that Garmin's writes stopped"
    );
}

/// A withdrawal that failed after deleting its grant's token, before it
/// told the athlete, still owes the notice: its retry finds the grant's
/// connection row standing, claims the notice on it and sends it once, and a
/// further redelivery sends nothing.
#[tokio::test]
async fn a_withdrawal_retried_after_its_token_delete_still_tells_the_athlete_once() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let withdrawal = GrantWithdrawal::new(
        OAuthService::new(resources.data(), resources.common.config.clone()),
        runtime,
    );
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    // What a disconnect that failed past its token delete leaves behind.
    resources
        .common
        .repos
        .oauth_tokens
        .delete_token(user_id, tenant, oauth_providers::GARMIN)
        .await
        .unwrap();

    for _retry in 0..2 {
        withdrawal
            .disconnect(
                user_id,
                oauth_providers::GARMIN,
                tenant,
                DisconnectReason::ProviderRevoked,
            )
            .await
            .unwrap();
    }
    assert_eq!(
        reconnect_notices(&resources, user_id, tenant).await,
        1,
        "bug: the retry of a withdrawal that lost its token row told the athlete other than once"
    );
    assert_eq!(connections(&resources, user_id, tenant).await, vec![]);
}

/// Two deliveries of one revocation racing each other tell a grant-only
/// athlete once: the notice is claimed on the grant's connection row, and
/// only one claim wins.
#[tokio::test]
async fn concurrent_withdrawals_of_one_grant_tell_the_athlete_once() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let withdrawal = GrantWithdrawal::new(
        OAuthService::new(resources.data(), resources.common.config.clone()),
        runtime,
    );
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;

    let deliver = || {
        withdrawal.disconnect(
            user_id,
            oauth_providers::GARMIN,
            tenant,
            DisconnectReason::ProviderRevoked,
        )
    };
    let (first, second) = tokio::join!(deliver(), deliver());
    first.unwrap();
    second.unwrap();
    assert_eq!(
        reconnect_notices(&resources, user_id, tenant).await,
        1,
        "bug: two racing deliveries of one revocation told the athlete twice"
    );
}

/// A Garmin sign-in goes on to Garmin's authorization while its grant is
/// owed (directive 8): over a dead grant it names the grant, for the app to
/// start; beside a live grant it asks for nothing.
#[tokio::test]
async fn a_garmin_sign_in_goes_on_to_its_grant_only_while_it_is_owed() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server_with_garmin_oauth(&scraper).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    kill_grant(&resources, user_id, tenant).await;

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "connected", "{body}");
    assert_eq!(
        body["oauth_follow_on"]["provider"],
        oauth_providers::GARMIN,
        "bug: the sign-in over a dead grant does not go on to its authorization: {body}"
    );
    assert!(
        body["oauth_follow_on"].get("authorization_url").is_none(),
        "the app starts the authorization its own way: {body}"
    );

    let (other, _user, other_tenant, other_token) = athlete(&resources).await;
    hold_grant(&resources, other, other_tenant).await;
    let (_, beside) = sign_in(&resources, &other_token).await;
    assert_eq!(beside["status"], "connected", "{beside}");
    assert!(
        beside["oauth_follow_on"].is_null(),
        "bug: a live grant is asked for again: {beside}"
    );
}

/// The hosted sign-in a Garmin link opens holds no token that can start an
/// authorization, so its answer carries Garmin's authorization URL itself.
#[tokio::test]
async fn the_hosted_garmin_sign_in_carries_its_grants_authorization_url() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server_with_garmin_oauth(&scraper).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    let link = mint_link_token(
        &MintProviderLinkTokenArgs {
            user_id,
            tenant_id: tenant.as_uuid(),
            provider: "sciotte",
            target: "garmin",
            channel: "telegram",
            channel_thread: None,
        },
        &resources.auth.admin_jwt_secret,
    )
    .unwrap();

    let (status, body) = sign_in(&resources, &link).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["oauth_follow_on"]["provider"],
        oauth_providers::GARMIN,
        "{body}"
    );
    let url = body["oauth_follow_on"]["authorization_url"]
        .as_str()
        .unwrap_or_else(|| panic!("bug: the hosted sign-in has no way on to the grant: {body}"));
    assert!(
        url.contains("client_id=") && !url.contains(HOSTED_SIGNIN_PATH),
        "bug: the follow-on is not Garmin's authorization: {url}"
    );
}

/// An OAuth start of a dead Garmin grant beside a live session authorizes the
/// grant: the session already serves the reads, and only the grant pushes
/// workouts. A dead grant alone still answers with the sign-in (above).
#[tokio::test]
async fn an_oauth_start_of_a_dead_garmin_grant_beside_a_live_session_authorizes_it() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server_with_garmin_oauth(&scraper).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
    hold_grant(&resources, user_id, tenant).await;
    kill_grant(&resources, user_id, tenant).await;

    let resp = AxumTestRequest::get("/api/oauth/mobile/init/garmin")
        .header("authorization", &format!("Bearer {token}"))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    assert_eq!(resp.status(), 200, "{}", resp.text());
    let started: Value = serde_json::from_str(&resp.text()).unwrap();
    let url = started["authorization_url"].as_str().unwrap_or_default();
    assert!(
        url.contains("client_id=") && !url.contains(HOSTED_SIGNIN_PATH),
        "bug: a dead grant beside a live session was sent back to the sign-in: {started}"
    );
}

/// The link a dead or withdrawn Garmin grant's notice carries opens Garmin's
/// own authorization when it is followed, beside a live session: never the
/// sign-in the athlete already holds, nor the picker. Opened while the grant
/// is flagged, and again once a deregistration deleted its rows.
#[tokio::test]
async fn the_grant_writes_link_opens_garmins_authorization_beside_a_live_session() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server_with_garmin_oauth(&scraper).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
    hold_grant(&resources, user_id, tenant).await;
    let open = || async {
        let (link, _expires_at) = mint_connect_oauth_init_url(
            "",
            user_id,
            tenant.as_uuid(),
            oauth_providers::GARMIN,
            "whatsapp",
            &resources.auth.admin_jwt_secret,
        )
        .unwrap();
        let resp = AxumTestRequest::get(&link)
            .send(AuthRoutes::routes(resources.auth_routes_context()))
            .await;
        assert_eq!(resp.status(), 302, "{}", resp.text());
        resp.header("location").unwrap_or_default().to_owned()
    };

    kill_grant(&resources, user_id, tenant).await;
    let flagged = open().await;
    assert!(
        flagged.contains("/oauth2Confirm") && flagged.contains("client_id="),
        "bug: the flagged grant's link does not open Garmin's authorization: {flagged}"
    );

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    GrantWithdrawal::new(
        OAuthService::new(resources.data(), resources.common.config.clone()),
        runtime,
    )
    .disconnect(
        user_id,
        oauth_providers::GARMIN,
        tenant,
        DisconnectReason::ProviderRevoked,
    )
    .await
    .unwrap();
    let withdrawn = open().await;
    assert!(
        withdrawn.contains("/oauth2Confirm") && withdrawn.contains("client_id="),
        "bug: the withdrawn grant's link does not open Garmin's authorization: {withdrawn}"
    );
}

/// The hosted picker keeps a connected Garmin card whose grant is owed
/// pickable, naming the grant to authorize; a live grant owes nothing.
#[tokio::test]
async fn the_hosted_picker_offers_an_owed_garmin_grant_on_a_connected_card() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server_with_garmin_oauth(&scraper).await;
    let picker = |user_id: Uuid, tenant: TenantId| {
        let token = mint_connect_link_token(
            user_id,
            tenant.as_uuid(),
            "telegram",
            None,
            &resources.auth.admin_jwt_secret,
        )
        .unwrap();
        AxumTestRequest::get(&format!(
            "/providers/connect?token={}",
            urlencoding::encode(&token)
        ))
        .header("accept-language", "en")
        .send(AuthRoutes::routes(resources.auth_routes_context()))
    };

    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
    hold_grant(&resources, user_id, tenant).await;
    kill_grant(&resources, user_id, tenant).await;
    let page = picker(user_id, tenant).await.text();
    assert!(
        page.contains(r#""oauth_owed":"garmin""#),
        "bug: the connected Garmin card hides the grant it owes"
    );

    let (live, _user, live_tenant, _token) = athlete(&resources).await;
    hold_session(&resources, live, live_tenant, LIVE_SESSION).await;
    hold_grant(&resources, live, live_tenant).await;
    let page = picker(live, live_tenant).await.text();
    assert!(
        !page.contains(r#""oauth_owed":"#),
        "bug: a live grant is offered again on the picker"
    );
}

/// A workout write the Garmin grant alone makes, for an athlete whose grant
/// is dead beside a live session, is refused with Garmin's authorization link
/// (directive 8), never sent to the session.
#[tokio::test]
async fn a_write_for_a_dead_garmin_grant_is_refused_with_its_authorization_link() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server_with_garmin_oauth(&scraper).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
    hold_grant(&resources, user_id, tenant).await;
    kill_grant(&resources, user_id, tenant).await;

    let refused = backend_resolver::write_backend(
        &resources.common.repos,
        &resources.fitness.provider_registry,
        user_id,
        tenant,
        oauth_providers::GARMIN,
        ProviderCapabilities::CALENDAR_WRITE,
    )
    .await
    .expect_err("bug: a workout for a dead Garmin grant went to the session");
    let refused = with_grant_link(resources.as_ref(), user_id, tenant, refused).await;
    let details = refused
        .details
        .as_deref()
        .expect("the refusal carries details");
    let url = details["authorization_url"]
        .as_str()
        .unwrap_or_else(|| panic!("bug: no authorization link on the refusal: {details}"));
    assert!(
        url.contains("client_id=") && !url.contains(HOSTED_SIGNIN_PATH),
        "bug: the link is not Garmin's authorization: {url}"
    );
    assert!(
        refused.tool_message().contains(url),
        "bug: the model is never given the link: {}",
        refused.tool_message()
    );
}

/// A workout prescribed with no calendar named, for an athlete whose Garmin
/// grant is dead beside a live session, lands on that grant and is refused
/// with Garmin's authorization link, through the tool the model calls.
#[tokio::test]
async fn an_unnamed_prescription_for_a_dead_garmin_grant_gets_its_authorization_link() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server_with_garmin_oauth(&scraper).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
    hold_grant(&resources, user_id, tenant).await;
    kill_grant(&resources, user_id, tenant).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let executor = UniversalToolExecutor::new(runtime).with_scopes(OAuthScope::self_grant());
    let date = (Utc::now() + chrono::Duration::days(3))
        .date_naive()
        .to_string();
    let response = executor
        .execute_tool(UniversalRequest {
            tool_name: "prescribe_workout".to_owned(),
            parameters: serde_json::json!({ "template_slug": "long_run_z2", "date": date }),
            user_id: user_id.to_string(),
            protocol: "test".to_owned(),
            tenant_id: Some(tenant.to_string()),
        })
        .await;
    let rendered = match response {
        Ok(response) => {
            assert!(
                !response.success,
                "bug: the workout was pushed: {response:?}"
            );
            format!("{:?} {:?}", response.error, response.result)
        }
        Err(e) => format!("{e:?}"),
    };
    assert!(
        rendered.contains("client_id=") && !rendered.contains(HOSTED_SIGNIN_PATH),
        "bug: the refusal does not carry Garmin's authorization link: {rendered}"
    );
}

/// A workout prescribed for an athlete whose only Garmin connection is a dead
/// grant, with no session, is refused, and the refusal is never Garmin's bare
/// authorization: an athlete with no session reconnects through the hosted
/// sign-in first, which goes on to the grant (JF's directives 1 and 8).
#[tokio::test]
async fn an_unnamed_prescription_for_a_dead_garmin_grant_alone_is_refused() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server_with_garmin_oauth(&scraper).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    kill_grant(&resources, user_id, tenant).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let executor = UniversalToolExecutor::new(runtime).with_scopes(OAuthScope::self_grant());
    let date = (Utc::now() + chrono::Duration::days(3))
        .date_naive()
        .to_string();
    let response = executor
        .execute_tool(UniversalRequest {
            tool_name: "prescribe_workout".to_owned(),
            parameters: serde_json::json!({ "template_slug": "long_run_z2", "date": date }),
            user_id: user_id.to_string(),
            protocol: "test".to_owned(),
            tenant_id: Some(tenant.to_string()),
        })
        .await;
    let rendered = match response {
        Ok(response) => {
            assert!(
                !response.success,
                "bug: a workout was pushed through a dead grant: {response:?}"
            );
            format!("{:?} {:?}", response.error, response.result)
        }
        Err(e) => format!("{e:?}"),
    };
    assert!(
        !rendered.contains("grant_owed"),
        "bug: an athlete with no Garmin session was pointed at the grant alone: {rendered}"
    );
}

/// A Garmin sign-in repeated moments later, answered from the session it
/// stored, still goes on to the grant it owes: an authorization left
/// unfinished is offered again.
#[tokio::test]
async fn a_repeated_garmin_sign_in_still_goes_on_to_its_owed_grant() {
    let (scraper, calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server_with_garmin_oauth(&scraper).await;
    let (_user_id, _user, _tenant, token) = athlete(&resources).await;

    let (_, first) = sign_in(&resources, &token).await;
    assert_eq!(
        first["oauth_follow_on"]["provider"],
        oauth_providers::GARMIN,
        "{first}"
    );
    let (status, again) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "{again}");
    assert_eq!(
        again["short_circuit"], true,
        "the stored session answers: {again}"
    );
    assert_eq!(
        again["oauth_follow_on"]["provider"],
        oauth_providers::GARMIN,
        "bug: the repeated sign-in dropped the grant it owes: {again}"
    );
    assert_eq!(
        calls_matching(&calls, "login").len(),
        1,
        "the repeat never reaches the service"
    );
}
