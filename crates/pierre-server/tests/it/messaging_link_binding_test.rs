// ABOUTME: An OAuth channel link completes only for the Dravr account that started it (carnet#1580)
// ABOUTME: Forwarded links and foreign sessions are refused unspent; app-started links hand the code back to the app
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The link code in `state` names the account that started a Slack or Discord
//! link; the provider's code names whoever approved it. A link started on one
//! account and forwarded to someone else used to link that person's channel
//! identity to the sender, whose account then answered their DMs. These tests
//! hold the binding: the callback completes only in a browser signed in as
//! the starter, an app-started link hands the code back to the app, and the
//! app finishes it only with the starter's login. Every refusal leaves the
//! link code unspent and links nothing.
//!
//! The channels are left unconfigured in the callback tests, so a request
//! that gets past the binding fails at the configuration lookup, before any
//! provider is called: the tests stay offline, and "not refused as a foreign
//! session" is what they read.

use crate::common::{create_test_server_resources, create_test_tenant};
use crate::helpers::axum_test::AxumTestRequest;

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_auth::oauth2_server::first_party::MOBILE_CHANNEL_LINK_URI;
use pierre_auth::security::cookies::auth_cookie_name;
use pierre_core::models::{TenantId, User};
use pierre_database::backends::{CreateLinkStateParams, MessagingRepository};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::messaging::MessagingRoutes;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

/// The refusal a caller who did not start the link reads.
const NOT_THE_STARTER: &str = "This link was started from another Dravr account";

/// The provider's authorization code every callback here carries.
const PROVIDER_CODE: &str = "provider-auth-code";

struct Fixture {
    resources: Arc<ServerContext>,
    starter: User,
    starter_token: String,
    tenant_id: TenantId,
}

async fn fixture() -> Fixture {
    let resources = create_test_server_resources().await.unwrap();
    let (starter, starter_token) = create_test_tenant(&resources, "starter@example.com")
        .await
        .unwrap();
    let tenant_id = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(starter.id)
        .await
        .unwrap()
        .first()
        .expect("the starter has a tenant")
        .id;
    Fixture {
        resources,
        starter,
        starter_token,
        tenant_id,
    }
}

impl Fixture {
    /// A pending Slack link the starter began, as `init` would store it.
    async fn pending_slack_link(&self, app_return_uri: Option<&str>) -> String {
        let db: &dyn MessagingRepository = &*self.resources.common.repos.messaging;
        let code = Uuid::new_v4().to_string();
        let starter = self.starter.id.to_string();
        let expires_at = (Timestamp::now() + Elapsed::minutes(10)).to_rfc3339();
        db.create_link_state(&CreateLinkStateParams {
            id: &Uuid::new_v4().to_string(),
            tenant_id: self.tenant_id,
            user_id: Some(&starter),
            channel_type: "slack",
            code: &code,
            method: "oauth",
            channel_user_id: None,
            sender_name: None,
            app_return_uri,
            expires_at: &expires_at,
        })
        .await
        .unwrap();
        code
    }

    async fn another_account_token(&self) -> String {
        create_test_tenant(&self.resources, "someone-else@example.com")
            .await
            .unwrap()
            .1
    }

    fn router(&self) -> axum::Router {
        MessagingRoutes::routes(Arc::clone(&self.resources))
    }

    /// The link code is still live and nothing was linked.
    async fn assert_unspent(&self, code: &str) {
        let db: &dyn MessagingRepository = &*self.resources.common.repos.messaging;
        assert!(
            db.get_link_state(code).await.unwrap().is_some(),
            "a refused completion leaves the link code unspent"
        );
        assert!(
            db.list_channel_links_for_user(&self.starter.id.to_string())
                .await
                .unwrap()
                .is_empty(),
            "a refused completion links nothing"
        );
    }
}

fn callback_uri(state: &str) -> String {
    format!("/api/messaging/link/callback/slack?state={state}&code={PROVIDER_CODE}")
}

fn session_cookie(token: &str) -> String {
    format!("{}={token}", auth_cookie_name())
}

#[tokio::test]
async fn a_forwarded_link_is_refused_in_a_browser_with_no_session() {
    let f = fixture().await;
    let state = f.pending_slack_link(None).await;

    let resp = AxumTestRequest::get(&callback_uri(&state))
        .send(f.router())
        .await;

    assert_eq!(resp.status(), 403, "no session vouches for the starter");
    let body = resp.text();
    assert!(body.contains(NOT_THE_STARTER), "{body}");
    f.assert_unspent(&state).await;
}

#[tokio::test]
async fn a_link_completed_in_another_accounts_browser_is_refused() {
    let f = fixture().await;
    let state = f.pending_slack_link(None).await;
    let other = f.another_account_token().await;

    let resp = AxumTestRequest::get(&callback_uri(&state))
        .header("cookie", &session_cookie(&other))
        .send(f.router())
        .await;

    assert_eq!(
        resp.status(),
        403,
        "another account's session is not the starter's"
    );
    assert!(resp.text().contains(NOT_THE_STARTER));
    f.assert_unspent(&state).await;
}

#[tokio::test]
async fn the_starters_own_browser_gets_past_the_binding() {
    let f = fixture().await;
    let state = f.pending_slack_link(None).await;

    let resp = AxumTestRequest::get(&callback_uri(&state))
        .header("cookie", &session_cookie(&f.starter_token))
        .send(f.router())
        .await;

    let status = resp.status();
    let body = resp.text();
    // The starter's session vouches for the link, so it fails next at the
    // unconfigured channel (a 500), before the provider and before the code
    // is spent.
    assert_eq!(
        status, 500,
        "the starter's session vouches for the link: {body}"
    );
    assert!(!body.contains(NOT_THE_STARTER), "{body}");
    f.assert_unspent(&state).await;
}

#[tokio::test]
async fn an_app_started_link_hands_the_code_back_to_the_app() {
    let f = fixture().await;
    let state = f.pending_slack_link(Some(MOBILE_CHANNEL_LINK_URI)).await;

    let resp = AxumTestRequest::get(&callback_uri(&state))
        .send(f.router())
        .await;

    assert_eq!(resp.status(), 302, "the app finishes the link itself");
    assert_eq!(
        resp.header("location"),
        Some(
            format!("{MOBILE_CHANNEL_LINK_URI}?channel=slack&state={state}&code={PROVIDER_CODE}")
                .as_str()
        ),
    );
    f.assert_unspent(&state).await;
}

#[tokio::test]
async fn the_app_finishes_its_link_only_with_the_starters_login() {
    let f = fixture().await;
    let state = f.pending_slack_link(Some(MOBILE_CHANNEL_LINK_URI)).await;
    let body = json!({ "state": state, "code": PROVIDER_CODE });
    let other = f.another_account_token().await;

    let anonymous = AxumTestRequest::post("/api/messaging/link/complete/slack")
        .json(&body)
        .send(f.router())
        .await;
    assert_eq!(
        anonymous.status(),
        401,
        "no login, no link; a 401 lets the app renew a lapsed session and retry"
    );
    f.assert_unspent(&state).await;

    let foreign = AxumTestRequest::post("/api/messaging/link/complete/slack")
        .header("authorization", &format!("Bearer {other}"))
        .json(&body)
        .send(f.router())
        .await;
    assert_eq!(
        foreign.status(),
        403,
        "another account cannot finish the link"
    );
    assert!(foreign.text().contains(NOT_THE_STARTER));
    f.assert_unspent(&state).await;

    let own = AxumTestRequest::post("/api/messaging/link/complete/slack")
        .header("authorization", &format!("Bearer {}", f.starter_token))
        .json(&body)
        .send(f.router())
        .await;
    let status = own.status();
    let text = own.text();
    // Past the binding, it fails at the unconfigured channel (a 500).
    assert_eq!(
        status, 500,
        "the starter's login vouches for the link: {text}"
    );
    assert!(!text.contains(NOT_THE_STARTER), "{text}");
    f.assert_unspent(&state).await;
}

#[tokio::test]
async fn completion_is_refused_for_another_channel_before_the_binding() {
    let f = fixture().await;
    let state = f.pending_slack_link(Some(MOBILE_CHANNEL_LINK_URI)).await;

    let resp = AxumTestRequest::post("/api/messaging/link/complete/discord")
        .header("authorization", &format!("Bearer {}", f.starter_token))
        .json(&json!({ "state": state, "code": PROVIDER_CODE }))
        .send(f.router())
        .await;

    assert_eq!(resp.status(), 400);
    assert!(resp.text().contains("Channel mismatch"));
    f.assert_unspent(&state).await;
}

/// Configure Slack as an OAuth channel for the starter's tenant and start a
/// link with `body`; returns the response status and, on success, the stored
/// link state.
async fn start_slack_link(f: &Fixture, body: Option<Value>) -> (u16, Option<Value>) {
    let auth = format!("Bearer {}", f.starter_token);
    let configured = AxumTestRequest::put("/api/messaging/channels/slack")
        .header("authorization", &auth)
        .json(&json!({
            "enabled": true,
            "credentials": {
                "api_key": "slack-client-id",
                "api_secret": "slack-client-secret",
                "webhook_secret": "slack-signing-secret"
            }
        }))
        .send(f.router())
        .await;
    assert!(configured.status() < 300, "{}", configured.text());

    let mut request =
        AxumTestRequest::post("/api/messaging/link/init/slack").header("authorization", &auth);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let resp = request.send(f.router()).await;
    let status = resp.status();
    if status != 200 {
        return (status, None);
    }
    let init: Value = resp.json();
    let linking_url = init["linking_url"].as_str().unwrap();
    let state = url::Url::parse(linking_url)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .expect("the authorize URL carries the link code as state");
    let db: &dyn MessagingRepository = &*f.resources.common.repos.messaging;
    (status, db.get_link_state(&state).await.unwrap())
}

#[tokio::test]
async fn the_app_names_its_own_callback_when_it_starts_a_link() {
    let f = fixture().await;

    let (status, stored) = start_slack_link(
        &f,
        Some(json!({ "app_return_uri": MOBILE_CHANNEL_LINK_URI })),
    )
    .await;

    assert_eq!(status, 200);
    assert_eq!(stored.unwrap()["app_return_uri"], MOBILE_CHANNEL_LINK_URI);
}

#[tokio::test]
async fn a_link_started_on_the_web_has_no_app_to_return_to() {
    let f = fixture().await;

    let (status, stored) = start_slack_link(&f, None).await;

    assert_eq!(status, 200);
    assert!(stored.unwrap()["app_return_uri"].is_null());
}

#[tokio::test]
async fn a_return_address_other_than_the_apps_is_refused() {
    let f = fixture().await;

    for foreign in [
        "https://evil.example/messaging/link-callback",
        "dravr://messaging/link-callback?next=https://evil.example",
        "exp://192.168.1.20:8082/--/messaging/link-callback",
    ] {
        let (status, _) = start_slack_link(&f, Some(json!({ "app_return_uri": foreign }))).await;
        assert_eq!(
            status, 400,
            "{foreign} must not receive a link's provider code"
        );
    }
}

/// Pressing Cancel on Slack or Discord sends the browser back with `error` and
/// no code. An app-started link hands that back too, so the app's sheet closes
/// rather than stranding the athlete on a refusal page.
#[tokio::test]
async fn a_declined_app_link_is_handed_back_so_the_sheet_closes() {
    let f = fixture().await;
    let state = f.pending_slack_link(Some(MOBILE_CHANNEL_LINK_URI)).await;

    let resp = AxumTestRequest::get(&format!(
        "/api/messaging/link/callback/slack?state={state}&error=access_denied&error_description=%3Cscript%3E"
    ))
    .send(f.router())
    .await;

    assert_eq!(resp.status(), 302);
    assert_eq!(
        resp.header("location"),
        Some(
            format!("{MOBILE_CHANNEL_LINK_URI}?channel=slack&state={state}&error=access_denied")
                .as_str()
        ),
        "the provider's own error text is never reflected"
    );
    f.assert_unspent(&state).await;
}

/// A link code minted for another linking method is not finished by a
/// provider's code.
#[tokio::test]
async fn a_code_not_started_as_an_oauth_link_is_refused() {
    let f = fixture().await;
    let db: &dyn MessagingRepository = &*f.resources.common.repos.messaging;
    let code = Uuid::new_v4().to_string();
    let starter = f.starter.id.to_string();
    let expires_at = (Timestamp::now() + Elapsed::minutes(10)).to_rfc3339();
    db.create_link_state(&CreateLinkStateParams {
        id: &Uuid::new_v4().to_string(),
        tenant_id: f.tenant_id,
        user_id: Some(&starter),
        channel_type: "slack",
        code: &code,
        method: "deep_link",
        channel_user_id: None,
        sender_name: None,
        app_return_uri: None,
        expires_at: &expires_at,
    })
    .await
    .unwrap();

    let resp = AxumTestRequest::get(&callback_uri(&code))
        .header("cookie", &session_cookie(&f.starter_token))
        .send(f.router())
        .await;

    assert_eq!(resp.status(), 400);
    assert!(resp.text().contains("not started as an OAuth link"));
    f.assert_unspent(&code).await;
}
