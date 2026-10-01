// ABOUTME: Integration tests for the hosted Intervals.icu API-key form a chat's connect picker opens
// ABOUTME: Pins the picker card, connect-token gating on GET and POST, live validation, storage, and key hygiene
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "provider-intervals-icu")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The hosted connect picker a chat's `/connect` opens offers Intervals.icu,
//! and its card leads to an athlete id + API key form authorized by the same
//! connect link-token. The POST validates the pair against Intervals.icu —
//! stubbed on loopback, as `intervals_icu_link_route_test` does — through the
//! same linking path the web Settings form uses, then redirects to the shared
//! success page.

mod common;
mod helpers;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use axum::http::StatusCode;
use axum::Router;
use helpers::axum_test::AxumTestRequest;
use helpers::notify_capture::capture_logs;
use pierre_auth::security::cookies::auth_cookie_name;
use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_core::models::{ConnectionType, TenantId, UserOAuthToken};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_middleware::provider_link_token::{
    mint_connect_link_token, mint_link_token, MintProviderLinkTokenArgs,
};
use pierre_providers::intervals_icu_provider::default_config;
use pierre_providers::ProviderRegistry;
use pierre_routes_auth::AuthRoutes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use uuid::Uuid;

/// A key no other string in a page or log line could contain by accident.
const SECRET_KEY: &str = "sk-intervals-7f3a9c2e41d8";

/// The form page and its POST target.
const FORM_PATH: &str = "/providers/connect/intervals_icu";

/// One-shot loopback stub standing in for Intervals.icu. Answers a single
/// request with `status_line` + `body` and returns the captured request head.
async fn stub_once(status_line: &'static str, body: &'static str) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let addr = listener.local_addr().expect("stub addr");
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut head = Vec::new();
        let mut buf = [0_u8; 1024];
        loop {
            let n = socket.read(&mut buf).await.expect("read request");
            if n == 0 {
                break;
            }
            head.extend_from_slice(&buf[..n]);
            if head.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let response = format!(
            "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write response");
        socket.flush().await.expect("flush response");
        String::from_utf8_lossy(&head).into_owned()
    });
    (format!("http://{addr}"), handle)
}

struct Fixture {
    resources: Arc<ServerContext>,
    router: Router,
    user_id: Uuid,
    tenant_id: TenantId,
}

/// The auth router with Intervals.icu pointed at `base_url`, and a user with
/// a tenant for the connect token to name.
async fn setup(base_url: &str) -> Fixture {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, _) = common::create_test_user(&resources.agent.database)
        .await
        .unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("list tenants")
        .first()
        .expect("user has a tenant")
        .id;
    // These tests pin the pages against their English text; an athlete's own
    // language is `connect_hosted_locale_test`'s subject.
    resources
        .common
        .repos
        .users
        .update_locale(user_id, "en")
        .await
        .expect("store the athlete's locale");

    let mut config = default_config();
    base_url.clone_into(&mut config.api_base_url);
    let mut registry = ProviderRegistry::new();
    registry.set_default_config(INTERVALS_ICU, config);
    let mut context = resources.auth_routes_context();
    context.provider_registry = Arc::new(registry);

    Fixture {
        router: AuthRoutes::routes(context),
        resources,
        user_id,
        tenant_id,
    }
}

fn connect_token(fixture: &Fixture) -> String {
    mint_connect_link_token(
        fixture.user_id,
        fixture.tenant_id.as_uuid(),
        "telegram",
        None,
        &fixture.resources.auth.admin_jwt_secret,
    )
    .expect("mint connect token")
}

fn form_url(token: &str) -> String {
    format!("{FORM_PATH}?token={}", urlencoding::encode(token))
}

/// The cards the picker page embeds, as the page's script reads them.
fn picker_cards(body: &str) -> Vec<serde_json::Value> {
    let start = body.find("providers: ").expect("the page embeds its cards") + "providers: ".len();
    let json = &body[start..];
    let end = json.find(",\n").expect("the card list ends its line");
    serde_json::from_str(&json[..end]).expect("the embedded cards are JSON")
}

async fn stored_token(fixture: &Fixture) -> Option<UserOAuthToken> {
    fixture
        .resources
        .common
        .repos
        .oauth_tokens
        .get_token(fixture.user_id, fixture.tenant_id, INTERVALS_ICU)
        .await
        .expect("read the stored token")
}

async fn intervals_connections(fixture: &Fixture) -> Vec<ConnectionType> {
    fixture
        .resources
        .common
        .repos
        .provider_connections
        .get_for_user(fixture.user_id, Some(fixture.tenant_id))
        .await
        .expect("read connections")
        .into_iter()
        .filter(|c| c.provider == INTERVALS_ICU)
        .map(|c| c.connection_type)
        .collect()
}

// ============================================================================
// The picker offers Intervals.icu
// ============================================================================

#[tokio::test]
async fn picker_offers_intervals_icu_as_an_api_key_card() {
    let (base_url, stub) = stub_once("HTTP/1.1 200 OK", "{}").await;
    let fixture = setup(&base_url).await;
    let token = connect_token(&fixture);

    let resp = AxumTestRequest::get(&format!(
        "/providers/connect?token={}",
        urlencoding::encode(&token)
    ))
    .send(fixture.router.clone())
    .await;
    assert_eq!(resp.status(), 200);
    let body = resp.text();

    let cards = picker_cards(&body);
    let card = cards
        .iter()
        .find(|c| c["provider"] == INTERVALS_ICU)
        .unwrap_or_else(|| panic!("the picker offers Intervals.icu: {cards:?}"));
    assert_eq!(card["kind"], "api_key");
    assert_eq!(card["target"], INTERVALS_ICU);
    assert_eq!(card["connected"], false);
    assert_eq!(card["display_name"], "Intervals.icu");
    // The card carries its own destination, and that destination is the form.
    let destination = card["form_url"]
        .as_str()
        .unwrap_or_else(|| panic!("an api_key card names its hosted form: {card}"));
    assert_eq!(destination, form_url(&token));
    let form = AxumTestRequest::get(destination)
        .send(fixture.router.clone())
        .await;
    assert_eq!(form.status(), 200);
    let form = form.text();
    assert!(
        form.contains(r#"name="athlete_id""#) && form.contains(r#"name="api_key""#),
        "the card's destination is the athlete id + API key form: {form}"
    );
    // Only a card with a hosted form of its own carries one.
    for other in cards.iter().filter(|c| c["provider"] != INTERVALS_ICU) {
        assert!(other.get("form_url").is_none(), "{other}");
    }
    stub.abort();
}

#[tokio::test]
async fn picker_shows_intervals_icu_connected_once_linked() {
    let (base_url, stub) = stub_once("HTTP/1.1 200 OK", "{}").await;
    let fixture = setup(&base_url).await;
    fixture
        .resources
        .common
        .repos
        .provider_connections
        .register_connection(
            fixture.user_id,
            fixture.tenant_id,
            INTERVALS_ICU,
            &ConnectionType::Manual,
            None,
        )
        .await
        .expect("register connection");
    let token = connect_token(&fixture);

    let body = AxumTestRequest::get(&format!(
        "/providers/connect?token={}",
        urlencoding::encode(&token)
    ))
    .send(fixture.router.clone())
    .await
    .text();
    let cards = picker_cards(&body);
    let card = cards
        .iter()
        .find(|c| c["provider"] == INTERVALS_ICU)
        .expect("the picker offers Intervals.icu");
    assert_eq!(card["connected"], true);
    stub.abort();
}

// ============================================================================
// GET /providers/connect/intervals_icu
// ============================================================================

#[tokio::test]
async fn form_page_renders_the_fields_for_a_valid_connect_token() {
    let (base_url, stub) = stub_once("HTTP/1.1 200 OK", "{}").await;
    let fixture = setup(&base_url).await;
    let token = connect_token(&fixture);

    let resp = AxumTestRequest::get(&form_url(&token))
        .send(fixture.router.clone())
        .await;
    assert_eq!(resp.status(), 200);
    let body = resp.text();

    assert!(body.contains(
        r#"<form id="api-key-form" method="post" action="/providers/connect/intervals_icu">"#
    ));
    assert!(body.contains(r#"name="athlete_id""#), "athlete id field");
    assert!(
        body.contains(r#"<input type="password" id="api_key" name="api_key""#),
        "the API key is a password field"
    );
    assert!(
        body.contains(&format!(
            r#"<input type="hidden" name="token" value="{token}">"#
        )),
        "the form carries the connect token it was opened with"
    );
    assert!(
        body.contains("Linked from Telegram"),
        "the page names the chat"
    );
    assert!(
        body.contains(r#"class="alert alert-error" role="alert" hidden>"#),
        "no error banner shows on a fresh form"
    );
    assert!(
        body.contains("@media (prefers-color-scheme: dark)"),
        "the page draws with the shared Boreal sheet"
    );
    assert!(!body.contains("{{"), "no placeholder survives the render");
    stub.abort();
}

#[tokio::test]
async fn form_page_refuses_a_missing_token() {
    let (base_url, stub) = stub_once("HTTP/1.1 200 OK", "{}").await;
    let fixture = setup(&base_url).await;

    let body = AxumTestRequest::get(FORM_PATH)
        .header("accept-language", "en")
        .send(fixture.router.clone())
        .await
        .text();
    assert!(body.contains("This link is incomplete."), "{body}");
    assert!(
        !body.contains(r#"name="api_key""#),
        "no form without a token"
    );
    stub.abort();
}

#[tokio::test]
async fn form_page_refuses_an_invalid_or_narrow_token() {
    let (base_url, stub) = stub_once("HTTP/1.1 200 OK", "{}").await;
    let fixture = setup(&base_url).await;
    // A narrow per-provider token is refused as a forged one is: only the
    // connect scope opens this form.
    let narrow = mint_link_token(
        &MintProviderLinkTokenArgs {
            user_id: fixture.user_id,
            tenant_id: fixture.tenant_id.as_uuid(),
            provider: "sciotte",
            target: "strava",
            channel: "telegram",
            channel_thread: None,
        },
        &fixture.resources.auth.admin_jwt_secret,
    )
    .unwrap();

    for token in ["not-a-jwt", narrow.as_str()] {
        let body = AxumTestRequest::get(&form_url(token))
            .header("accept-language", "en")
            .send(fixture.router.clone())
            .await
            .text();
        assert!(body.contains("invalid or has expired"), "{token}: {body}");
        assert!(!body.contains(r#"name="api_key""#), "{token}: no form");
    }
    stub.abort();
}

// ============================================================================
// POST /providers/connect/intervals_icu
// ============================================================================

#[tokio::test]
async fn posting_valid_credentials_links_the_account_and_redirects_to_success() {
    let (base_url, stub) = stub_once(
        "HTTP/1.1 200 OK",
        r#"{"id":"i123456","name":"Test Athlete"}"#,
    )
    .await;
    let fixture = setup(&base_url).await;
    let token = connect_token(&fixture);

    let (logs, guard) = capture_logs();
    let resp = AxumTestRequest::post(FORM_PATH)
        .form(&[
            ("token", token.as_str()),
            ("athlete_id", " i123456 "),
            ("api_key", SECRET_KEY),
        ])
        .send(fixture.router.clone())
        .await;
    drop(guard);

    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    // The token rides along, so the success page is written in the athlete's
    // locale too.
    let location = format!(
        "/providers/connect/success?channel=telegram&target=intervals_icu&token={}",
        urlencoding::encode(&token)
    );
    assert_eq!(resp.header("location"), Some(location.as_str()));
    let success = AxumTestRequest::get(&location)
        .send(fixture.router.clone())
        .await
        .text();
    assert!(
        success.contains("<h1>Intervals.icu connected</h1>"),
        "{success}"
    );

    // Validated live, with the athlete id in the path only.
    let head = stub.await.expect("stub task joins");
    assert!(
        head.starts_with("GET /api/v1/athlete/i123456 "),
        "athlete id addresses the path; got head: {head}"
    );

    // Stored under the token's own user and tenant, as the Settings form does.
    let stored = stored_token(&fixture).await.expect("the key is stored");
    assert_eq!(stored.access_token, SECRET_KEY);
    assert_eq!(stored.provider_user_id.as_deref(), Some("i123456"));
    assert_eq!(stored.token_type, "api_key");
    assert_eq!(stored.tenant_id, fixture.tenant_id.to_string());
    assert_eq!(
        intervals_connections(&fixture).await,
        vec![ConnectionType::Manual]
    );

    let lines = logs.lock().unwrap();
    assert!(
        lines
            .iter()
            .any(|l| l.event == "Hosted connect: Intervals.icu linked"),
        "the link is logged"
    );
    for line in lines.iter() {
        assert!(
            !line.event.contains(SECRET_KEY)
                && line.fields.values().all(|v| !v.contains(SECRET_KEY)),
            "the API key reached a log line: {line:?}"
        );
    }
}

#[tokio::test]
async fn the_success_page_names_intervals_icu() {
    let (base_url, stub) = stub_once("HTTP/1.1 200 OK", "{}").await;
    let fixture = setup(&base_url).await;

    let body =
        AxumTestRequest::get("/providers/connect/success?channel=telegram&target=intervals_icu")
            .header("accept-language", "en")
            .send(fixture.router.clone())
            .await
            .text();
    assert!(body.contains("Intervals.icu connected"), "{body}");
    assert!(body.contains("return to Telegram"), "{body}");
    stub.abort();
}

#[tokio::test]
async fn posting_rejected_credentials_reshows_the_form_and_stores_nothing() {
    let (base_url, stub) = stub_once("HTTP/1.1 401 Unauthorized", "{}").await;
    let fixture = setup(&base_url).await;
    let token = connect_token(&fixture);

    let (logs, guard) = capture_logs();
    let resp = AxumTestRequest::post(FORM_PATH)
        .form(&[
            ("token", token.as_str()),
            ("athlete_id", "i123456"),
            ("api_key", SECRET_KEY),
        ])
        .send(fixture.router.clone())
        .await;
    drop(guard);

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = resp.text();
    assert!(
        body.contains("Intervals.icu rejected those credentials"),
        "the page names the problem: {body}"
    );
    assert!(
        body.contains(r#"class="alert alert-error" role="alert">"#),
        "the error banner is shown"
    );
    assert!(
        body.contains(r#"name="athlete_id" value="i123456""#),
        "the athlete id is refilled"
    );
    assert!(
        body.contains(&format!(r#"name="token" value="{token}""#)),
        "the retry keeps the connect token"
    );
    assert!(
        !body.contains(SECRET_KEY),
        "the API key is never rendered back"
    );

    assert!(stored_token(&fixture).await.is_none(), "nothing is stored");
    assert!(intervals_connections(&fixture).await.is_empty());

    stub.await.expect("stub task joins");
    let lines = logs.lock().unwrap();
    assert!(
        lines
            .iter()
            .any(|l| l.event == "Hosted connect: Intervals.icu credentials refused"),
        "the refusal is logged"
    );
    for line in lines.iter() {
        assert!(
            !line.event.contains(SECRET_KEY)
                && line.fields.values().all(|v| !v.contains(SECRET_KEY)),
            "the API key reached a log line: {line:?}"
        );
    }
}

#[tokio::test]
async fn posting_an_empty_api_key_is_refused_before_intervals_icu_is_called() {
    let (base_url, stub) = stub_once("HTTP/1.1 200 OK", r#"{"id":"i123456"}"#).await;
    let fixture = setup(&base_url).await;
    let token = connect_token(&fixture);

    let resp = AxumTestRequest::post(FORM_PATH)
        .form(&[
            ("token", token.as_str()),
            ("athlete_id", "i123456"),
            ("api_key", "   "),
        ])
        .send(fixture.router.clone())
        .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = resp.text();
    assert!(body.contains("Enter your API key."), "{body}");
    assert!(
        !body.contains("api_key is required"),
        "the JSON route's error text is not the page's: {body}"
    );
    assert!(stored_token(&fixture).await.is_none());
    assert!(!stub.is_finished(), "Intervals.icu is never called");
    stub.abort();
}

#[tokio::test]
async fn posting_with_a_bad_token_is_rejected_and_stores_nothing() {
    let (base_url, stub) = stub_once("HTTP/1.1 200 OK", r#"{"id":"i123456"}"#).await;
    let fixture = setup(&base_url).await;

    for token in ["", "not-a-jwt"] {
        let resp = AxumTestRequest::post(FORM_PATH)
            .form(&[
                ("token", token),
                ("athlete_id", "i123456"),
                ("api_key", SECRET_KEY),
            ])
            .header("accept-language", "en")
            .send(fixture.router.clone())
            .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "token {token:?}");
        let body = resp.text();
        assert!(
            body.contains("This link is incomplete.") || body.contains("invalid or has expired"),
            "token {token:?}: {body}"
        );
        assert!(!body.contains(SECRET_KEY));
    }

    assert!(stored_token(&fixture).await.is_none(), "nothing is stored");
    assert!(intervals_connections(&fixture).await.is_empty());
    assert!(!stub.is_finished(), "Intervals.icu is never called");
    stub.abort();
}

// ============================================================================
// Through the server's own router, CSRF layer included
// ============================================================================

/// An athlete signed in to the web app carries its session cookie to the
/// hosted form, and an HTML form cannot send `X-CSRF-Token`. The CSRF layer
/// lets the POST through — the signed connect token in its body is what
/// authorizes it — so the form's own handler answers: here it refuses an
/// empty key with the form again, where the CSRF layer would answer 401.
#[tokio::test]
async fn a_web_app_session_cookie_does_not_refuse_the_form_post() {
    common::init_server_config();
    let resources = common::create_test_server_resources().await.unwrap();
    let (user, session) = common::create_test_tenant(&resources, "intervals-csrf@example.test")
        .await
        .unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .expect("list tenants")
        .first()
        .expect("user has a tenant")
        .id;
    resources
        .common
        .repos
        .users
        .update_locale(user.id, "en")
        .await
        .expect("store the athlete's locale");
    let token = mint_connect_link_token(
        user.id,
        tenant_id.as_uuid(),
        "telegram",
        None,
        &resources.auth.admin_jwt_secret,
    )
    .expect("mint connect token");
    // The web app's cookie: the one the CSRF layer demands a header for.
    let cookie = format!("{}={session}", auth_cookie_name());
    let app = ProviderToolRouter::build_http_app(&resources)
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_645))));

    let resp = AxumTestRequest::post(FORM_PATH)
        .header("cookie", &cookie)
        .form(&[
            ("token", token.as_str()),
            ("athlete_id", "i123456"),
            ("api_key", "   "),
        ])
        .send(app)
        .await;

    let status = resp.status();
    let body = resp.text();
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "the form handler answers, not the CSRF layer: {body}"
    );
    assert!(body.contains("Enter your API key."), "{body}");
    assert!(
        body.contains(r#"name="athlete_id" value="i123456""#),
        "the form is shown again with the athlete id refilled: {body}"
    );
}
