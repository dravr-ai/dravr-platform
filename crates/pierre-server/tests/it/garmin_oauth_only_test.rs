// ABOUTME: Garmin connects through its OAuth while a seat is free: the move asked of usr/pwd-only athletes, the refused new sign-in
// ABOUTME: Pins the migration finder, the seat rule's fallback, the re-sign kept, the no-seat return, the callback and the grant check

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin OAuth-only connect (carnet#1454).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// While a seat is free on Garmin's OAuth client, an athlete with no Garmin
// grant is moved there: a usr/pwd-only card asks for the move, a new usr/pwd
// sign-in is refused (whatever client sent it), and a stale sign-in link
// says so. An athlete who already holds a usr/pwd connection re-signs it
// freely, and with the pool full the usr/pwd connection stays the way in.
// The finder counts who has yet to move, and the daily grant check flags a
// grant Garmin will no longer renew, never a connection whose token is gone.
// The chat's minted Garmin authorization URL weighs the same seat pool.
#![cfg(all(feature = "provider-garmin", feature = "health-sync"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::garmin_history_import_test::{
    athlete, calls_matching, connect_garmin_oauth, connections, garmin_history, hold_grant,
    hold_session, import_ended, rows_under, server, server_with_garmin_oauth, sign_in,
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
use pierre_core::models::{ConnectionStatus, ConnectionType, TenantId, UserOAuthToken};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::garmin_webhook::check_garmin_grants_in;
use pierre_middleware::provider_link_token::{
    mint_link_token, verify_link_token, MintProviderLinkTokenArgs, HOSTED_SIGNIN_PATH,
};
use pierre_routes_auth::AuthRoutes;
use pierre_tool_runtime::implementations::connection::{
    is_no_seat_refusal, mint_oauth_authorize_url,
};
use pierre_tool_runtime::protocol::reauth_notice::reconnects_through_oauth;
use pierre_tool_runtime::runtime::ToolRuntime;
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

/// The finder lists the usr/pwd-only Garmin athletes of one tenant: never a
/// grant holder, a delegated row, or another tenant's athlete.
#[tokio::test]
async fn the_migration_finder_lists_usr_pwd_only_athletes_of_its_tenant() {
    let resources = server(common::test_server_config()).await;
    let (only, _u, tenant, _t) = athlete(&resources).await;
    hold_session(&resources, only, tenant, LIVE_SESSION).await;

    let (both, _u, both_tenant, _t) = athlete(&resources).await;
    hold_session(&resources, both, both_tenant, LIVE_SESSION).await;
    hold_grant(&resources, both, both_tenant).await;

    let (coached, _u, coached_tenant, _t) = athlete(&resources).await;
    resources
        .common
        .repos
        .provider_connections
        .register_connection(
            coached,
            coached_tenant,
            oauth_providers::SCIOTTE_GARMIN,
            &ConnectionType::Delegated,
            None,
        )
        .await
        .unwrap();

    let repos = &resources.common.repos.provider_connections;
    let listed = |tenant: TenantId| async move {
        repos
            .list_mirror_only_users(
                tenant,
                oauth_providers::SCIOTTE_GARMIN,
                oauth_providers::GARMIN,
            )
            .await
            .unwrap()
    };
    assert_eq!(
        listed(tenant).await,
        vec![only],
        "bug: the usr/pwd-only athlete is listed"
    );
    assert!(
        listed(both_tenant).await.is_empty(),
        "bug: a grant holder has moved already"
    );
    assert!(
        listed(coached_tenant).await.is_empty(),
        "bug: a delegated row is someone else's account"
    );
    let other = TenantId::from_uuid(Uuid::new_v4());
    assert!(
        listed(other).await.is_empty(),
        "bug: the finder never reads another tenant"
    );
}

/// With a seat free, a new usr/pwd Garmin sign-in from an athlete with no
/// grant and no usr/pwd connection is refused before the scraper is called,
/// and nothing is stored.
#[tokio::test]
async fn a_new_usr_pwd_garmin_sign_in_is_refused_while_a_seat_is_free() {
    let (scraper, calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(config_with_cap(&scraper, 5)).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 403, "bug: the sign-in is refused: {body}");
    assert_eq!(body["details"]["reason"], "oauth_required", "{body}");
    assert!(
        calls_matching(&calls, "login").is_empty(),
        "bug: the scraper is never called for a refused sign-in"
    );
    assert!(
        resources
            .common
            .repos
            .oauth_tokens
            .get_token(user_id, tenant, oauth_providers::SCIOTTE_GARMIN)
            .await
            .unwrap()
            .is_none(),
        "bug: no session row is written"
    );
    assert_eq!(
        connections(&resources, user_id, tenant).await,
        vec![],
        "bug: no connection row is written"
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

/// A grant holder's sign-in is the one-time history copy, never refused.
#[tokio::test]
async fn a_grant_holders_sign_in_is_never_refused() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(config_with_cap(&scraper, 5)).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "bug: the history sign-in goes through: {body}");
    import_ended(&resources, user_id, tenant).await;
}

/// COROS has no OAuth to move to: its sign-in is never refused.
#[tokio::test]
async fn a_coros_sign_in_is_unaffected() {
    let (scraper, calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(config_with_cap(&scraper, 5)).await;
    let (_user_id, _user, _tenant, token) = athlete(&resources).await;
    let resp = AxumTestRequest::post("/api/providers/sciotte/login")
        .header("authorization", &format!("Bearer {token}"))
        .json(&serde_json::json!({
            "email": "athlete@example.com",
            "password": "not-a-real-password",
            "target": "coros",
            "tos_consent": true,
        }))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    let body: Value = serde_json::from_str(&resp.text()).unwrap_or(Value::Null);
    assert_ne!(
        body["details"]["reason"], "oauth_required",
        "bug: COROS is never sent to an OAuth: {body}"
    );
    assert_eq!(calls_matching(&calls, "login").len(), 1, "{body}");
}

/// A sign-in link minted before the move opens the refusal its login would
/// get, worded in the athlete's language, never a form that cannot succeed.
#[tokio::test]
async fn a_stale_garmin_sign_in_link_says_garmin_connects_officially() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(config_with_cap(&scraper, 5)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    resources
        .common
        .repos
        .users
        .update_locale(user_id, "en")
        .await
        .unwrap();
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
    let page = AxumTestRequest::get(&format!(
        "/providers/signin?token={}",
        urlencoding::encode(&link)
    ))
    .header("accept-language", "en")
    .send(AuthRoutes::routes(resources.auth_routes_context()))
    .await
    .text();
    assert!(
        page.contains("now connects through its official sign-in"),
        "bug: the stale link says Garmin connects officially: {page}"
    );
    assert!(
        !page.contains("id=\"login-form\""),
        "bug: no sign-in form is offered: {page}"
    );
}

/// A usr/pwd-only Garmin card is asked to move while a seat is free, and
/// asked nothing with the pool full.
#[tokio::test]
async fn a_usr_pwd_only_card_is_asked_to_move_only_while_a_seat_is_free() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(config_with_cap(&scraper, 2)).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let card = garmin_card(&resources, &token).await;
    assert_eq!(
        card["connected_backend"],
        oauth_providers::SCIOTTE_GARMIN,
        "{card}"
    );
    assert_eq!(
        card["migration"], "oauth_required",
        "bug: a seat is free: {card}"
    );
    assert_eq!(card["recommended_backend"], "oauth", "{card}");
    assert!(
        reconnects_through_oauth(&runtime, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        "bug: its reconnect points to Garmin's OAuth"
    );

    fill_a_seat(&resources).await;
    fill_a_seat(&resources).await;
    let full = garmin_card(&resources, &token).await;
    assert!(
        full.get("migration").is_none(),
        "bug: with no seat the card asks no move: {full}"
    );
    assert_eq!(full["recommended_backend"], "mirror", "{full}");
    assert!(
        !reconnects_through_oauth(&runtime, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        "bug: with no seat the reconnect is the ordinary sign-in"
    );
    assert!(
        !reconnects_through_oauth(&runtime, user_id, tenant, oauth_providers::SCIOTTE_COROS).await,
        "bug: COROS has no OAuth to move to"
    );
}

/// A usr/pwd-only athlete who moves: the callback copies through their live
/// session once and retires it, keeping every row it copied; the card then
/// reads imported, asks no move, and recommends the OAuth.
#[tokio::test]
async fn moving_to_the_grant_retires_the_session_and_reads_imported() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server_with_garmin_oauth(&scraper).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
    let before = garmin_card(&resources, &token).await;
    assert_eq!(before["migration"], "oauth_required", "{before}");

    connect_garmin_oauth(&resources, user_id, tenant).await;
    import_ended(&resources, user_id, tenant).await;

    assert!(
        rows_under(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await > 0,
        "bug: the rows the session copied are kept"
    );
    assert!(
        !connections(&resources, user_id, tenant)
            .await
            .iter()
            .any(|(provider, _)| provider == oauth_providers::SCIOTTE_GARMIN),
        "bug: the session's connection is retired"
    );
    let after = garmin_card(&resources, &token).await;
    assert_eq!(
        after["connected_backend"],
        oauth_providers::GARMIN,
        "{after}"
    );
    assert!(after.get("migration").is_none(), "{after}");
    assert_eq!(after["recommended_backend"], "oauth", "{after}");
    assert_eq!(
        garmin_history(&resources, &token).await["state"],
        "imported",
        "bug: what the session copied is the history"
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

/// Only the athlete's own usr/pwd Garmin session is moved to the OAuth: an
/// athlete with no session, or whose Garmin is read through a coach's
/// delegated session, reconnects the ordinary way even with a seat free.
#[tokio::test]
async fn only_an_own_usr_pwd_session_reconnects_through_the_oauth() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(config_with_cap(&scraper, 5)).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let moves = |user_id: Uuid, tenant: TenantId| {
        let runtime = Arc::clone(&runtime);
        async move {
            reconnects_through_oauth(&runtime, user_id, tenant, oauth_providers::SCIOTTE_GARMIN)
                .await
        }
    };

    let (none, _u, none_tenant, _t) = athlete(&resources).await;
    assert!(
        !moves(none, none_tenant).await,
        "bug: an athlete with no usr/pwd session has nothing to move"
    );

    let (coached, _u, coached_tenant, _t) = athlete(&resources).await;
    resources
        .common
        .repos
        .provider_connections
        .register_connection(
            coached,
            coached_tenant,
            oauth_providers::SCIOTTE_GARMIN,
            &ConnectionType::Delegated,
            None,
        )
        .await
        .unwrap();
    assert!(
        !moves(coached, coached_tenant).await,
        "bug: a coach's delegated session is not the athlete's to move"
    );

    let (own, _u, own_tenant, _t) = athlete(&resources).await;
    hold_session(&resources, own, own_tenant, LIVE_SESSION).await;
    assert!(
        moves(own, own_tenant).await,
        "bug: the athlete's own usr/pwd session moves while a seat is free"
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

/// A Garmin OAuth reconnect the pool has no seat for is offered the usr/pwd
/// sign-in instead, never an authorization URL that cannot take a seat.
#[tokio::test]
async fn a_garmin_reconnect_with_no_seat_offers_the_usr_pwd_sign_in() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(config_with_cap(&scraper, 1)).await;
    fill_a_seat(&resources).await;
    let (user_id, _u, tenant, _t) = athlete(&resources).await;

    let mut result = loop_result(Some(oauth_providers::GARMIN), None);
    let prompt = recover_in(&resources, user_id, tenant, "en", &mut result)
        .await
        .prompt
        .expect("the athlete still gets a reconnect control");
    let target = offer_target(&resources, &prompt.url).await;
    let token = token_after(&target, HOSTED_SIGNIN_PATH);
    let claims =
        verify_link_token(&token, RECOVERY_LINK_SECRET, "sciotte").expect("a hosted-login token");
    assert_eq!(claims.tgt, "garmin", "bug: the Garmin usr/pwd sign-in");
    assert_eq!(
        claims.sub,
        user_id.to_string(),
        "bug: the link signs in this athlete"
    );
}
