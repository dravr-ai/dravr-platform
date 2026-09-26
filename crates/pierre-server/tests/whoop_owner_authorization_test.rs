// ABOUTME: carnet#539 point 2 — a WHOOP OAuth start is refused until the account accepts WHOOP's owner-authorization notice
// ABOUTME: Pins every start path (launch route, mobile init, hosted picker, connect_provider) per account and per notice version

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! WHOOP's API Terms of Use (§4, effective 2026-10-06) let Dravr store WHOOP
//! Data, compute from it and hand it to the athlete's agent only under the
//! express authorization of the data's owner. That authorization is the WHOOP
//! notice an account accepts before the WHOOP OAuth flow begins, recorded per
//! account and per notice version through the same store as the TrainingPeaks
//! and COROS notices.
//!
//! Every path that mints a WHOOP authorization URL is held to it: the launch
//! route the web app and the SDK bridge start from, the mobile init, the
//! hosted connect picker's init and the `connect_provider` tool (which also
//! serves the chat reconnect). A start without the acceptance mints nothing; a start that
//! carries it records the version and proceeds; an account that accepted is
//! not asked again; a changed notice is asked again; and an account the
//! `provider_exposure_notice` flag leaves off is never asked.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate, and the surviving crate doc keeps `missing_docs` quiet.
#![cfg(feature = "provider-whoop")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use axum::Router;
use chrono::Utc;
use common::{create_test_server_resources, create_test_user, create_test_user_with_email};
use dravr_tronc::mcp::schema::ToolResponse;
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use helpers::axum_test::AxumTestRequest;
use pierre_core::constants::oauth::providers::{provider_terms_version, WHOOP};
use pierre_core::feature_flags::FeatureKey;
use pierre_core::models::{TenantId, TenantOAuthCredentials, User};
use pierre_database::repositories::SyncCursorRow;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_middleware::provider_link_token::mint_connect_link_token;
use pierre_routes_auth::AuthRoutes;
use pierre_tool_runtime::implementations::connection::{
    is_notice_refusal, mint_oauth_authorize_url, ConnectProviderTool,
};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

/// Where WHOOP's authorization page lives; every minted URL starts here.
const WHOOP_AUTHORIZE: &str = "https://api.prod.whoop.com/oauth/oauth2/auth";

/// A notice version older than the current one, standing for an account that
/// accepted WHOOP's notice before its text changed.
const EARLIER_WHOOP_NOTICE: &str = "2026-01-01";

/// WHOOP's current owner-authorization notice version.
fn whoop_notice_version() -> &'static str {
    provider_terms_version(WHOOP).expect("WHOOP carries an owner-authorization notice")
}

async fn whoop_server() -> Arc<ServerContext> {
    create_test_server_resources().await.unwrap()
}

/// The user's personal tenant, carrying a WHOOP OAuth app so a start that
/// passes the notice precondition can mint a real URL on every path.
async fn primary_tenant(resources: &Arc<ServerContext>, user_id: Uuid) -> TenantId {
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
    resources
        .common
        .repos
        .tenants
        .store_oauth_credentials(&TenantOAuthCredentials {
            tenant_id,
            provider: WHOOP.to_owned(),
            client_id: "whoop-owner-auth-test-client".to_owned(),
            client_secret: "whoop-owner-auth-test-secret".to_owned(),
            redirect_uri: "http://localhost/api/oauth/callback/whoop".to_owned(),
            scopes: vec!["read:recovery".to_owned()],
            rate_limit_per_day: 1000,
        })
        .await
        .unwrap();
    tenant_id
}

/// A session bearer carrying the user's tenant, as the web and mobile
/// clients hold one: OAuth state is tenant-scoped.
fn bearer(resources: &Arc<ServerContext>, user: &User, tenant_id: TenantId) -> String {
    let token = resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            user,
            &resources.auth.jwks_manager,
            Some(tenant_id.to_string()),
        )
        .unwrap();
    format!("Bearer {token}")
}

/// Arm the `provider_exposure_notice` flag for `user_id`, as an operator
/// onboarding the athlete does; it is off by default.
async fn arm_notice(resources: &Arc<ServerContext>, user_id: Uuid) {
    resources
        .common
        .repos
        .feature_flags
        .set_user_override(user_id, FeatureKey::ProviderExposureNotice, true, None)
        .await
        .unwrap();
}

async fn accepted(resources: &Arc<ServerContext>, user_id: Uuid) -> Option<String> {
    resources
        .common
        .repos
        .users
        .provider_terms_version(user_id, WHOOP)
        .await
        .unwrap()
}

fn routes(resources: &Arc<ServerContext>) -> Router {
    AuthRoutes::routes(resources.auth_routes_context())
}

/// The web launch route: `(status, Location header, JSON body)`.
async fn launch(
    resources: &Arc<ServerContext>,
    auth: &str,
    tos_consent: bool,
) -> (u16, Option<String>, Value) {
    let path = if tos_consent {
        "/api/oauth/authorize/whoop?tos_consent=true"
    } else {
        "/api/oauth/authorize/whoop"
    };
    let resp = AxumTestRequest::get(path)
        .header("authorization", auth)
        .send(routes(resources))
        .await;
    let location = resp.header("location").map(ToOwned::to_owned);
    (
        resp.status(),
        location,
        serde_json::from_str(&resp.text()).unwrap_or(Value::Null),
    )
}

async fn whoop_card_consent_required(resources: &Arc<ServerContext>, auth: &str) -> bool {
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
        .unwrap_or_else(|| panic!("no WHOOP card: {body}"))["consent_required"]
        .as_bool()
        .unwrap()
}

/// Assert the refusal every start path answers while the notice is
/// outstanding: a 400 naming WHOOP and the notice, whose details tell a
/// client which notice to show.
fn assert_notice_refusal(status: u16, body: &Value, path: &str) {
    assert_eq!(
        status, 400,
        "{path}: a start without the notice is refused: {body}"
    );
    let message = body["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("WHOOP") && message.contains("notice"),
        "{path}: the refusal names the provider and what is missing: {body}"
    );
    assert_eq!(
        body["details"]["action"], "accept_provider_notice",
        "{path}: the refusal says which action clears it: {body}"
    );
    assert_eq!(body["details"]["provider"], WHOOP, "{path}: {body}");
}

#[tokio::test]
async fn the_web_launch_asks_for_whoops_notice_once_per_version() {
    let resources = whoop_server().await;
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = primary_tenant(&resources, user_id).await;
    let auth = bearer(&resources, &user, tenant_id);
    arm_notice(&resources, user_id).await;

    // Armed, before any acceptance: the card asks, and the launch mints no URL.
    assert!(whoop_card_consent_required(&resources, &auth).await);
    let (status, location, body) = launch(&resources, &auth, false).await;
    assert_notice_refusal(status, &body, "authorize");
    assert_eq!(location, None, "no authorization URL leaves the server");
    assert_eq!(accepted(&resources, user_id).await, None);

    // The start that carries the acceptance records the version shown and
    // redirects to WHOOP's authorization page.
    let (status, location, body) = launch(&resources, &auth, true).await;
    assert_eq!(
        status, 302,
        "an accepted notice lets the start through: {body}"
    );
    let location = location.expect("the redirect names WHOOP's page");
    assert!(
        location.starts_with(WHOOP_AUTHORIZE) && location.contains("state="),
        "the start redirects to WHOOP's authorization page: {location}"
    );
    assert_eq!(
        accepted(&resources, user_id).await.as_deref(),
        Some(whoop_notice_version())
    );
    assert!(!whoop_card_consent_required(&resources, &auth).await);

    // A later start is not asked again.
    let (status, _, body) = launch(&resources, &auth, false).await;
    assert_eq!(
        status, 302,
        "an accepted account starts without the notice: {body}"
    );

    // An answer to an earlier notice is no answer to this one.
    resources
        .common
        .repos
        .users
        .record_provider_terms(user_id, WHOOP, EARLIER_WHOOP_NOTICE)
        .await
        .unwrap();
    assert!(
        whoop_card_consent_required(&resources, &auth).await,
        "a changed notice is shown again"
    );
    let (status, location, body) = launch(&resources, &auth, false).await;
    assert_notice_refusal(status, &body, "authorize after a version bump");
    assert_eq!(location, None);
    assert_eq!(
        accepted(&resources, user_id).await.as_deref(),
        Some(EARLIER_WHOOP_NOTICE),
        "a refused start records nothing"
    );
}

#[tokio::test]
async fn the_mobile_start_is_held_to_the_same_notice_as_the_launch_route() {
    let resources = whoop_server().await;
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = primary_tenant(&resources, user_id).await;
    let auth = bearer(&resources, &user, tenant_id);
    arm_notice(&resources, user_id).await;

    // The mobile init, refused without the acceptance.
    let resp = AxumTestRequest::get("/api/oauth/mobile/init/whoop")
        .header("authorization", &auth)
        .send(routes(&resources))
        .await;
    let status = resp.status();
    let body: Value = serde_json::from_str(&resp.text()).unwrap_or(Value::Null);
    assert_notice_refusal(status, &body, "mobile init");
    assert_eq!(accepted(&resources, user_id).await, None);

    // Carrying it, the mobile init records the version and answers the URL.
    let resp = AxumTestRequest::get("/api/oauth/mobile/init/whoop?tos_consent=true")
        .header("authorization", &auth)
        .send(routes(&resources))
        .await;
    let status = resp.status();
    let body: Value = serde_json::from_str(&resp.text()).unwrap();
    assert_eq!(status, 200, "{body}");
    assert!(
        body["authorization_url"]
            .as_str()
            .unwrap_or_default()
            .starts_with(WHOOP_AUTHORIZE),
        "the mobile init answers WHOOP's authorization URL: {body}"
    );
    assert_eq!(
        accepted(&resources, user_id).await.as_deref(),
        Some(whoop_notice_version())
    );

    // Accepted there, the launch route redirects without being asked.
    let resp = AxumTestRequest::get("/api/oauth/authorize/whoop")
        .header("authorization", &auth)
        .send(routes(&resources))
        .await;
    assert_eq!(resp.status(), 302);
    assert!(resp
        .header("location")
        .unwrap_or_default()
        .starts_with(WHOOP_AUTHORIZE));
}

#[tokio::test]
async fn the_hosted_picker_shows_whoops_notice_and_carries_its_acceptance() {
    let resources = whoop_server().await;
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = primary_tenant(&resources, user_id).await;
    arm_notice(&resources, user_id).await;
    let token = mint_connect_link_token(
        user_id,
        tenant_id.as_uuid(),
        "telegram",
        None,
        &resources.auth.admin_jwt_secret,
    )
    .unwrap();

    // The picker's WHOOP card carries the notice's English text to show.
    let page = AxumTestRequest::get(&format!("/providers/connect?token={token}"))
        .send(routes(&resources))
        .await
        .text();
    assert!(
        page.contains(
            "As the owner of this data, I authorize Dravr to store my WHOOP measurements"
        ),
        "the picker embeds WHOOP's owner authorization for its card"
    );
    assert!(page.contains("phase-oauth-consent"));

    // An init without the acceptance goes back to the picker, not to WHOOP.
    let init = format!("/api/providers/connect/oauth-init/whoop?token={token}");
    let resp = AxumTestRequest::get(&init).send(routes(&resources)).await;
    assert_eq!(resp.status(), 200, "the refusal is the picker's error page");
    assert_eq!(resp.header("location"), None);
    assert!(resp.text().contains("notice accepted first"));
    assert_eq!(accepted(&resources, user_id).await, None);

    // With it, the init records the version and redirects to WHOOP.
    let resp = AxumTestRequest::get(&format!("{init}&tos_consent=true"))
        .send(routes(&resources))
        .await;
    assert_eq!(resp.status(), 302);
    assert!(resp
        .header("location")
        .unwrap_or_default()
        .starts_with(WHOOP_AUTHORIZE));
    assert_eq!(
        accepted(&resources, user_id).await.as_deref(),
        Some(whoop_notice_version())
    );
}

fn tool_context(user_id: Uuid, tenant_id: TenantId) -> ToolContext {
    ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant_id.to_string())
        .with_auth_method("jwt_bearer")
}

fn structured(response: &ToolResponse) -> &Value {
    response
        .structured_content
        .as_ref()
        .expect("connect_provider carries structured content")
}

#[tokio::test]
async fn connect_provider_mints_no_whoop_url_until_the_notice_is_accepted() {
    let resources = whoop_server().await;
    let (user_id, _) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = primary_tenant(&resources, user_id).await;
    arm_notice(&resources, user_id).await;
    let state: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = tool_context(user_id, tenant_id);

    // A chat or MCP start carries no acceptance: the tool says where the
    // athlete gives it, and mints nothing.
    let refused = ConnectProviderTool
        .execute(&state, &ctx, json!({ "provider": "whoop" }))
        .await;
    assert!(refused.is_error, "{:?}", refused.structured_content);
    let data = structured(&refused);
    assert_eq!(data["error_type"], "provider_notice_required", "{data}");
    assert!(
        data.get("authorization_url").is_none(),
        "no authorization URL is handed out: {data}"
    );
    assert!(
        data["error"]
            .as_str()
            .unwrap_or_default()
            .contains("Connections"),
        "the refusal points at the surface that shows the notice: {data}"
    );

    // The chat reconnect mints through the same function, and reads the
    // refusal as a notice to accept rather than a broken provider.
    let err = mint_oauth_authorize_url(state.as_ref(), user_id, tenant_id, WHOOP, None)
        .await
        .expect_err("the reconnect mints nothing while the notice is outstanding");
    assert!(is_notice_refusal(&err), "{err}");
    assert_eq!(accepted(&resources, user_id).await, None);

    // Once the account has accepted it on a connect surface, the tool mints.
    resources
        .common
        .repos
        .users
        .record_provider_terms(user_id, WHOOP, whoop_notice_version())
        .await
        .unwrap();
    let minted = ConnectProviderTool
        .execute(&state, &ctx, json!({ "provider": "whoop" }))
        .await;
    let data = structured(&minted);
    assert!(!minted.is_error, "{data}");
    assert!(
        data["authorization_url"]
            .as_str()
            .unwrap_or_default()
            .starts_with(WHOOP_AUTHORIZE),
        "{data}"
    );
}

#[tokio::test]
async fn an_account_the_flag_leaves_off_is_never_asked() {
    let resources = whoop_server().await;
    let (user_id, user) =
        create_test_user_with_email(&resources.agent.database, "whoop-unarmed@example.test")
            .await
            .unwrap();
    let tenant_id = primary_tenant(&resources, user_id).await;
    let auth = bearer(&resources, &user, tenant_id);

    assert!(!whoop_card_consent_required(&resources, &auth).await);
    let (status, location, body) = launch(&resources, &auth, false).await;
    assert_eq!(
        status, 302,
        "an unarmed account starts WHOOP without a notice: {body}"
    );
    assert!(location.unwrap_or_default().starts_with(WHOOP_AUTHORIZE));
    assert_eq!(
        accepted(&resources, user_id).await,
        None,
        "nothing was asked, so nothing is recorded"
    );

    let state: Arc<dyn ToolRuntime> = resources.clone();
    let minted = ConnectProviderTool
        .execute(
            &state,
            &tool_context(user_id, tenant_id),
            json!({ "provider": "whoop" }),
        )
        .await;
    assert!(!minted.is_error, "{:?}", minted.structured_content);
}

/// A mid-walk sync cursor for `provider`'s `data_type`, as health sync leaves
/// one after reading a page.
async fn seed_cursor(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    data_type: &str,
) {
    resources
        .common
        .repos
        .sync_cursors
        .upsert_sync_cursor(&SyncCursorRow {
            id: format!("{user_id}:{tenant_id}:{provider}:{data_type}"),
            user_id: user_id.to_string(),
            tenant_id: tenant_id.to_string(),
            provider: provider.to_owned(),
            data_type: data_type.to_owned(),
            cursor_value: Some("page-7-token".to_owned()),
            last_sync_at: Some(Utc::now()),
            last_sync_status: "completed".to_owned(),
            records_synced: 25,
            error_message: None,
            retry_count: 0,
            next_retry_at: None,
        })
        .await
        .unwrap();
}

async fn cursor_value(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    data_type: &str,
) -> Option<String> {
    resources
        .common
        .repos
        .sync_cursors
        .get_sync_cursor(&user_id.to_string(), &tenant_id, provider, data_type)
        .await
        .unwrap()
        .and_then(|row| row.cursor_value)
}

/// While the notice was owed, health sync skipped every WHOOP record while
/// its cursor walked on past them. Accepting the notice drops the WHOOP
/// cursors, so the reconnect's sync starts from WHOOP's newest page and reads
/// that history again; a refused start drops nothing, and other providers'
/// cursors are left where they are.
#[tokio::test]
async fn accepting_the_notice_rewinds_the_whoop_sync() {
    let resources = whoop_server().await;
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = primary_tenant(&resources, user_id).await;
    let auth = bearer(&resources, &user, tenant_id);
    arm_notice(&resources, user_id).await;
    for data_type in ["sleep", "recovery", "health"] {
        seed_cursor(&resources, user_id, tenant_id, WHOOP, data_type).await;
    }
    seed_cursor(&resources, user_id, tenant_id, "garmin", "sleep").await;

    let (status, _, body) = launch(&resources, &auth, false).await;
    assert_notice_refusal(status, &body, "authorize");
    assert_eq!(
        cursor_value(&resources, user_id, tenant_id, WHOOP, "sleep")
            .await
            .as_deref(),
        Some("page-7-token"),
        "a refused start leaves the WHOOP sync where it was"
    );

    let (status, _, body) = launch(&resources, &auth, true).await;
    assert_eq!(status, 302, "{body}");
    for data_type in ["sleep", "recovery", "health"] {
        assert_eq!(
            resources
                .common
                .repos
                .sync_cursors
                .get_sync_cursor(&user_id.to_string(), &tenant_id, WHOOP, data_type)
                .await
                .unwrap()
                .map(|row| row.cursor_value),
            None,
            "the WHOOP {data_type} cursor is gone, so the next sync starts from the newest page"
        );
    }
    assert_eq!(
        cursor_value(&resources, user_id, tenant_id, "garmin", "sleep")
            .await
            .as_deref(),
        Some("page-7-token"),
        "another provider's sync is not rewound"
    );

    // Once accepted, a later start records nothing and rewinds nothing.
    seed_cursor(&resources, user_id, tenant_id, WHOOP, "sleep").await;
    let (status, _, _) = launch(&resources, &auth, false).await;
    assert_eq!(status, 302);
    assert_eq!(
        cursor_value(&resources, user_id, tenant_id, WHOOP, "sleep")
            .await
            .as_deref(),
        Some("page-7-token")
    );
}
