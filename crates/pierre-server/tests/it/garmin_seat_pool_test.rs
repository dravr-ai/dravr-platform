// ABOUTME: Garmin's OAuth seat pool: the no-seat return, the chat-minted URL, the daily grant check, the sign-in reconnect
// ABOUTME: Pins that every OAuth start weighs the seats, a dead grant flags, and every reconnect signs in whatever the seats

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin's OAuth seat pool (carnet#1155, carnet#1494).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// Every start of Garmin's OAuth weighs the Developer Program client's seats:
// an SPA start with none left returns to the app with the reason, and the
// chat's minted authorization URL is refused the same way. The daily grant
// check flags a grant Garmin will no longer renew, never a connection whose
// token is gone. A Garmin reconnect signs in through the hosted page whatever
// the seats, and a session's sign-in is never refused for them.
#![cfg(all(feature = "provider-garmin", feature = "health-sync"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::garmin_sign_in_test::{
    athlete, calls_matching, connect_garmin_oauth, connections, garmin_history, hold_grant,
    hold_session, server, server_with_garmin_oauth, session_token_held, sign_in,
    spawn_switchable_scraper, ScraperSwitch, LIVE_SESSION,
};
use crate::helpers::axum_test::AxumTestRequest;
use crate::reconnect_prompt_on_served_turn_test::{
    loop_result, offer_target, recover_in, token_after, RECOVERY_LINK_SECRET,
};

use std::sync::Arc;

use chrono::{Duration, Utc};
use pierre_config::environment::ServerConfig;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{ConnectionStatus, ConnectionType, UserOAuthToken};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::garmin_webhook::check_garmin_grants_in;
use pierre_middleware::provider_link_token::{verify_link_token, HOSTED_SIGNIN_PATH};
use pierre_routes_auth::AuthRoutes;
use pierre_tool_runtime::implementations::connection::{
    is_no_seat_refusal, mint_oauth_authorize_url,
};
use secrecy::SecretString;
use serde_json::Value;
use uuid::Uuid;

/// A config whose Garmin client holds `cap` seats, with `scraper` as the
/// usr/pwd sign-in service.
fn config_with_cap(scraper: &str, cap: u32) -> ServerConfig {
    let mut config = common::test_server_config_with_sciotte(scraper);
    config.oauth.garmin_seat_cap = Some(cap);
    config
}

/// Fill one seat of the Garmin pool with another athlete's live grant.
async fn fill_a_seat(resources: &Arc<ServerContext>) {
    let (holder, _user, tenant, _token) = athlete(resources).await;
    hold_grant(resources, holder, tenant).await;
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

/// With a seat free, a new usr/pwd Garmin sign-in from an athlete with no
/// grant goes through: the sign-in goes first by default, and a free seat
/// never refuses it (carnet#1494).
#[tokio::test]
async fn a_new_usr_pwd_garmin_sign_in_goes_through_with_a_seat_free() {
    let (scraper, calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(config_with_cap(&scraper, 5)).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "bug: the sign-in is refused: {body}");
    assert_eq!(body["status"], "connected", "{body}");
    assert_eq!(
        calls_matching(&calls, "login").len(),
        1,
        "bug: the sign-in reaches the service once"
    );
    assert!(
        resources
            .common
            .repos
            .oauth_tokens
            .get_token(user_id, tenant, oauth_providers::SCIOTTE_GARMIN)
            .await
            .unwrap()
            .is_some(),
        "bug: the session row is written"
    );
}

/// With the pool full the usr/pwd sign-in stays the way in: the same
/// athlete's sign-in goes through.
#[tokio::test]
async fn a_full_pool_keeps_the_usr_pwd_sign_in() {
    let (scraper, calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(config_with_cap(&scraper, 1)).await;
    fill_a_seat(&resources).await;
    let (_user_id, _user, _tenant, token) = athlete(&resources).await;

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(
        status, 200,
        "bug: a full pool falls back to usr/pwd: {body}"
    );
    assert_eq!(body["status"], "connected", "{body}");
    assert_eq!(
        calls_matching(&calls, "login").len(),
        1,
        "bug: the sign-in reaches the service once"
    );
}

/// An athlete whose usr/pwd Garmin session died re-signs it with a seat
/// free: they keep their way in until they move.
#[tokio::test]
async fn an_existing_usr_pwd_connection_re_signs_with_a_seat_free() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(config_with_cap(&scraper, 5)).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_session(&resources, user_id, tenant, "dead-session").await;
    resources
        .common
        .repos
        .provider_connections
        .mark_needs_reauth(
            user_id,
            tenant,
            oauth_providers::SCIOTTE_GARMIN,
            Some("session_expired"),
            Utc::now(),
        )
        .await
        .unwrap();

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "bug: an existing connection re-signs: {body}");
    assert_eq!(body["status"], "connected", "{body}");
}

/// A grant holder's sign-in is never refused for the seats: the session it
/// stores becomes the provider's primary, beside the grant (carnet#1494).
#[tokio::test]
async fn a_grant_holders_sign_in_is_never_refused() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(config_with_cap(&scraper, 5)).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(
        status, 200,
        "bug: the grant holder's sign-in goes through: {body}"
    );
    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: the session is stored beside the grant"
    );
}

/// A usr/pwd-only athlete who connects Garmin's OAuth keeps their session
/// as the card's primary: the grant lands beside it as its fallback, nothing
/// retires the session, and the card keeps reading it (carnet#1494).
#[tokio::test]
async fn connecting_the_grant_keeps_the_session_serving_the_card() {
    let (scraper, calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server_with_garmin_oauth(&scraper).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
    connect_garmin_oauth(&resources, user_id, tenant).await;

    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: a new grant never retires the session"
    );
    assert!(
        connections(&resources, user_id, tenant)
            .await
            .iter()
            .any(|(provider, _)| provider == oauth_providers::SCIOTTE_GARMIN),
        "bug: the session's connection stays"
    );
    assert!(
        calls_matching(&calls, "drop").is_empty(),
        "bug: nothing drops the session"
    );
    let after = garmin_card(&resources, &token).await;
    assert_eq!(
        after["connected_backend"],
        oauth_providers::SCIOTTE_GARMIN,
        "bug: the session serves the card first: {after}"
    );
    assert_eq!(
        garmin_history(&resources, &token).await["state"],
        "not_applicable",
        "bug: the session reads its own history"
    );
}

/// An SPA tab sent to Garmin's consent page after the pool filled lands back
/// on the app's OAuth return with the reason, never on a JSON body.
#[tokio::test]
async fn a_spa_start_with_no_seat_returns_to_the_app_with_the_reason() {
    let mut config = common::test_server_config();
    config.oauth.garmin_seat_cap = Some(1);
    config.frontend_url = Some("https://app.example.test".to_owned());
    let resources = server(config).await;
    fill_a_seat(&resources).await;
    let (_user_id, _user, _tenant, token) = athlete(&resources).await;

    let resp = AxumTestRequest::get("/api/oauth/authorize/garmin")
        .header("authorization", &format!("Bearer {token}"))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    assert_eq!(resp.status(), 302, "{}", resp.text());
    let location = resp.header("location").expect("a redirect");
    assert!(
        location.starts_with("https://app.example.test/oauth-callback?")
            && location.contains("provider=garmin")
            && location.contains("success=false")
            && location.contains("error=no_seat"),
        "bug: the return names the missing seat: {location}"
    );
}

/// The daily grant check renews a live grant and flags one Garmin will no
/// longer renew, so its card reads "Reconnect" with nothing recorded.
#[tokio::test]
async fn the_daily_grant_check_flags_a_grant_garmin_will_not_renew() {
    let resources = server(common::test_server_config()).await;
    let (live, _u, live_tenant, _t) = athlete(&resources).await;
    hold_grant(&resources, live, live_tenant).await;

    let (dead, _u, dead_tenant, _t) = athlete(&resources).await;
    hold_grant(&resources, dead, dead_tenant).await;
    // Expired, with nothing to renew it with: the token path hands back none.
    let now = Utc::now();
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id: dead,
            tenant_id: dead_tenant.to_string(),
            provider: oauth_providers::GARMIN.to_owned(),
            access_token: SecretString::from("garmin-access-do-not-log".to_owned()),
            refresh_token: None,
            token_type: "Bearer".to_owned(),
            expires_at: Some(now - Duration::hours(1)),
            scope: None,
            provider_user_id: Some(format!("garmin-{dead}")),
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();

    let check = check_garmin_grants_in(&resources, &[live_tenant, dead_tenant])
        .await
        .unwrap();
    assert_eq!(
        (check.live, check.flagged, check.unread),
        (1, 1, 0),
        "{check:?}"
    );
    assert_eq!(
        connections(&resources, live, live_tenant).await,
        vec![(oauth_providers::GARMIN.to_owned(), ConnectionStatus::Active)],
        "bug: a live grant stays connected"
    );
    assert_eq!(
        connections(&resources, dead, dead_tenant).await,
        vec![(
            oauth_providers::GARMIN.to_owned(),
            ConnectionStatus::NeedsReauth
        )],
        "bug: a grant Garmin will not renew reads Reconnect"
    );
}

/// The daily grant check skips a Garmin connection whose token row is gone
/// (a disconnect since the list was read, a token never stored): there is no
/// grant Garmin refused, so the athlete is never flagged for it.
#[tokio::test]
async fn the_daily_grant_check_skips_a_connection_with_no_token_row() {
    let resources = server(common::test_server_config()).await;
    let (user_id, _u, tenant, _t) = athlete(&resources).await;
    resources
        .common
        .repos
        .provider_connections
        .register_connection(
            user_id,
            tenant,
            oauth_providers::GARMIN,
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();

    let check = check_garmin_grants_in(&resources, &[tenant]).await.unwrap();
    assert_eq!(
        (check.live, check.flagged, check.unread),
        (0, 0, 1),
        "bug: a missing token row is skipped, never flagged: {check:?}"
    );
    assert_eq!(
        connections(&resources, user_id, tenant).await,
        vec![(oauth_providers::GARMIN.to_owned(), ConnectionStatus::Active)],
        "bug: the connection is left as it is"
    );
}

/// The chat's minted Garmin authorization URL takes a seat like every other
/// OAuth start: with the pool full an athlete with no grant gets the seat
/// refusal (the caller offers the usr/pwd sign-in), a grant holder still
/// re-signs the seat they hold, and with a seat free the URL is minted.
#[tokio::test]
async fn the_chat_minted_garmin_authorization_url_honours_the_seat_pool() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(config_with_cap(&scraper, 1)).await;
    let (user_id, _u, tenant, _t) = athlete(&resources).await;

    let minted = mint_oauth_authorize_url(
        resources.as_ref(),
        user_id,
        tenant,
        oauth_providers::GARMIN,
        None,
    )
    .await;
    let (url, _state) = minted.expect("bug: a free seat mints Garmin's authorization URL");
    assert!(
        url.contains("client_id="),
        "bug: the URL is Garmin's authorization: {url}"
    );

    fill_a_seat(&resources).await;
    let refused = mint_oauth_authorize_url(
        resources.as_ref(),
        user_id,
        tenant,
        oauth_providers::GARMIN,
        None,
    )
    .await
    .expect_err("bug: a full pool mints no Garmin authorization URL");
    assert!(
        is_no_seat_refusal(&refused),
        "bug: the refusal names the missing seat: {refused:?}"
    );

    let (holder, _u, holder_tenant, _t) = athlete(&resources).await;
    hold_grant(&resources, holder, holder_tenant).await;
    mint_oauth_authorize_url(
        resources.as_ref(),
        holder,
        holder_tenant,
        oauth_providers::GARMIN,
        None,
    )
    .await
    .expect("bug: a grant holder re-signs the seat they hold");
}

/// A Garmin reconnect is offered the hosted sign-in whatever the seats: with
/// the pool full, and with a seat free, never an authorization URL
/// (carnet#1494).
#[tokio::test]
async fn a_garmin_reconnect_offers_the_sign_in_whatever_the_seats() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(config_with_cap(&scraper, 2)).await;
    let (user_id, _u, tenant, _t) = athlete(&resources).await;
    for seats in ["free", "full"] {
        if seats == "full" {
            fill_a_seat(&resources).await;
            fill_a_seat(&resources).await;
        }
        let mut result = loop_result(Some(oauth_providers::GARMIN), None);
        let prompt = recover_in(&resources, user_id, tenant, "en", &mut result)
            .await
            .prompt
            .expect("the athlete gets a reconnect control");
        let target = offer_target(&resources, &prompt.url).await;
        let token = token_after(&target, HOSTED_SIGNIN_PATH);
        let claims = verify_link_token(&token, RECOVERY_LINK_SECRET, "sciotte")
            .expect("a hosted-login token");
        assert_eq!(
            claims.tgt, "garmin",
            "bug: the Garmin sign-in with the pool {seats}"
        );
        assert_eq!(
            claims.sub,
            user_id.to_string(),
            "bug: the link signs in this athlete"
        );
    }
}

/// A sign-in over the athlete's own dead Garmin grant goes on to Garmin's
/// authorization even with the pool full: the dead grant's seat, freed when
/// the sign-in deletes it, is the one the athlete authorizes again
/// (carnet#1494, JF's directive 8).
#[tokio::test]
async fn a_sign_in_over_a_dead_grant_goes_on_to_it_with_the_pool_full() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(config_with_cap(&scraper, 1)).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
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

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["oauth_follow_on"]["provider"],
        oauth_providers::GARMIN,
        "bug: reconnecting lost the athlete's own Garmin seat: {body}"
    );
}

/// With the pool full, a sign-in of an athlete who never held a Garmin grant
/// is not sent on to an authorization that cannot finish.
#[tokio::test]
async fn a_full_pool_sends_no_sign_in_on_to_the_grant() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(config_with_cap(&scraper, 1)).await;
    fill_a_seat(&resources).await;
    let (_user_id, _user, _tenant, token) = athlete(&resources).await;

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body["oauth_follow_on"].is_null(),
        "bug: a full pool sends the sign-in into Garmin's authorization: {body}"
    );
}
