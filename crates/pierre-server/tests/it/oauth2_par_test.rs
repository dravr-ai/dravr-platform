// ABOUTME: RFC 9126 pushed authorization requests end to end — push, authorize, login, consent, one code, a token
// ABOUTME: A request_uri resolves only for its client, until it expires or mints its one code; the query beside it is ignored
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `POST /oauth2/par` and every page a pushed request crosses, driven over
//! the server's own HTTP composition.

use crate::common;
use crate::helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use crate::helpers::consent_form::hidden_field;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use base64::{engine::general_purpose, Engine as _};
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::models::ClientRegistrationRequest;
use pierre_auth::oauth2_server::pushed::request_uri_hash;
use pierre_auth::security::cookies::auth_cookie_name;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::models::User;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use secrecy::ExposeSecret;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

const REDIRECT: &str = "https://par.example.test/callback";
const VERIFIER: &str = "pushed-request-pkce-verifier-0123456789-abcdefghijklmnopqrstu";
/// The fixture user's password (`create_test_user_with_email`).
const PASSWORD: &str = "password123";

fn app(resources: &Arc<ServerContext>) -> axum::Router {
    ProviderToolRouter::build_http_app(resources)
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_926))))
}

/// A registered client; `secret` is empty for a public one.
struct Client {
    id: String,
    secret: String,
}

struct World {
    resources: Arc<ServerContext>,
    user: User,
    cookie: String,
}

async fn world() -> World {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await.unwrap();
    let email = format!("par_{}@example.test", Uuid::new_v4());
    let (user, session) = common::create_test_tenant(&resources, &email)
        .await
        .unwrap();
    World {
        resources,
        user,
        cookie: format!("{}={session}", auth_cookie_name()),
    }
}

async fn register_as(world: &World, auth_method: Option<&str>) -> Client {
    let registered =
        ClientRegistrationManager::new(world.resources.common.repos.oauth2_server.clone())
            .register_client_with_auth_method(
                ClientRegistrationRequest {
                    redirect_uris: vec![REDIRECT.to_owned()],
                    client_name: Some("Pushed request client".to_owned()),
                    client_uri: None,
                    grant_types: None,
                    response_types: None,
                    scope: None,
                },
                auth_method.map(str::to_owned),
                MAX_PENDING_REGISTRATIONS,
            )
            .await
            .unwrap();
    Client {
        id: registered.client_id,
        secret: registered.client_secret.expose_secret().to_owned(),
    }
}

async fn register(world: &World) -> Client {
    register_as(world, None).await
}

fn challenge() -> String {
    general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()))
}

fn fresh_state() -> String {
    format!("state-{}", Uuid::new_v4().simple())
}

/// `POST /oauth2/par` carrying `extra` beside the client's credentials and a
/// complete code request.
async fn push_with(world: &World, client: &Client, extra: &[(&str, &str)]) -> AxumTestResponse {
    let challenge = challenge();
    let state = fresh_state();
    let mut form = vec![
        ("client_id", client.id.as_str()),
        ("response_type", "code"),
        ("redirect_uri", REDIRECT),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ];
    if !client.secret.is_empty() {
        form.push(("client_secret", client.secret.as_str()));
    }
    form.extend_from_slice(extra);
    AxumTestRequest::post("/oauth2/par")
        .form(&form)
        .send(app(&world.resources))
        .await
}

/// Push a request and return its `request_uri`.
async fn push(world: &World, client: &Client, extra: &[(&str, &str)]) -> String {
    let pushed = push_with(world, client, extra).await;
    assert_eq!(pushed.status(), 201, "{}", pushed.body_text());
    assert_eq!(pushed.header("cache-control"), Some("no-store"));
    let body: Value = serde_json::from_str(&pushed.body_text()).unwrap();
    assert_eq!(body["expires_in"], 90);
    let request_uri = body["request_uri"].as_str().unwrap().to_owned();
    // RFC 9126 §2.2's URN prefix, a spec constant.
    assert!(
        request_uri.starts_with("urn:ietf:params:oauth:request_uri:"),
        "{request_uri}"
    );
    request_uri
}

/// `GET /oauth2/authorize` naming the pushed request, with `query` beside it
/// and the browser's `cookie`, when it holds one.
async fn authorize(
    world: &World,
    client_id: &str,
    request_uri: &str,
    query: &str,
    cookie: Option<&str>,
) -> AxumTestResponse {
    let uri = format!(
        "/oauth2/authorize?client_id={}&request_uri={}{query}",
        urlencoding::encode(client_id),
        urlencoding::encode(request_uri),
    );
    let mut request = AxumTestRequest::get(&uri);
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    request.send(app(&world.resources)).await
}

fn location(response: &AxumTestResponse) -> String {
    response
        .header("location")
        .unwrap_or_else(|| {
            panic!(
                "a redirect ({}): {}",
                response.status(),
                response.body_text()
            )
        })
        .to_owned()
}

fn code_of(response: &AxumTestResponse) -> String {
    let location = location(response);
    assert!(location.starts_with(REDIRECT), "to the client: {location}");
    Url::parse(&location)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map_or_else(
            || panic!("a code: {location}"),
            |(_, value)| value.into_owned(),
        )
}

/// The `name=value` pairs a response sets, as a browser would send them back.
fn cookies_set(response: &AxumTestResponse) -> String {
    response
        .header_all("set-cookie")
        .iter()
        .filter_map(|cookie| cookie.split(';').next())
        .collect::<Vec<_>>()
        .join("; ")
}

/// A refused `request_uri`: a page, never a redirect, saying why. The callers
/// send no session, so the page is in English (a signed-in athlete reads the
/// refusal in their own language).
fn assert_request_uri_refused(response: &AxumTestResponse) {
    assert_eq!(response.status(), 400, "{}", response.body_text());
    assert!(
        response.header("location").is_none(),
        "a refused request_uri is a page, never a redirect"
    );
    assert!(
        response.body_text().contains("expired or was already used"),
        "the invalid_request_uri refusal: {}",
        response.body_text()
    );
}

/// The athlete approves the consent form for the pushed request.
async fn approve(
    world: &World,
    client: &Client,
    request_uri: &str,
    cookie: &str,
) -> AxumTestResponse {
    let token = world
        .resources
        .auth
        .csrf_manager
        .generate_token(world.user.id)
        .unwrap();
    // The screen the athlete approves, resolved again from the request_uri;
    // a signed-out browser is sent to the login page and has none.
    let screen = AxumTestRequest::get(&format!(
        "/oauth2/authorize?client_id={}&request_uri={}",
        urlencoding::encode(&client.id),
        urlencoding::encode(request_uri)
    ))
    .header("cookie", cookie)
    .send(app(&world.resources))
    .await;
    let digest = hidden_field(&screen.body_text(), "review_digest").unwrap_or_default();
    let form = vec![
        ("client_id", client.id.as_str()),
        ("request_uri", request_uri),
        ("redirect_uri", "https://evil.example.test/cb"),
        ("decision", "approve"),
        ("csrf_token", token.as_str()),
        ("review_digest", digest.as_str()),
    ];
    AxumTestRequest::post("/oauth2/consent")
        .header("cookie", cookie)
        .form(&form)
        .send(app(&world.resources))
        .await
}

async fn exchange(world: &World, client: &Client, code: &str) -> AxumTestResponse {
    let mut form = json!({
        "grant_type": "authorization_code",
        "code": code,
        "redirect_uri": REDIRECT,
        "client_id": client.id,
        "code_verifier": VERIFIER,
    });
    if !client.secret.is_empty() {
        form["client_secret"] = json!(client.secret);
    }
    AxumTestRequest::post("/oauth2/token")
        .form(&form)
        .send(app(&world.resources))
        .await
}

/// Write a pushed request's expiry as the row would hold it.
async fn set_expiry(world: &World, request_uri: &str, expires_at: Timestamp) {
    let sql = "UPDATE oauth2_pushed_requests SET expires_at = $1 WHERE request_uri_hash = $2";
    let hash = request_uri_hash(request_uri);
    match world.resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(sql)
                .bind(expires_at)
                .bind(&hash)
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(sql)
                .bind(expires_at)
                .bind(&hash)
                .execute(db.pool())
                .await
                .unwrap();
        }
    }
}

/// A pushed request crosses the login page and the consent screen by its
/// `request_uri` alone, mints one code, and that code is a token. The query
/// parameters beside the `request_uri` are ignored, and once the code is
/// minted the `request_uri` resolves no more.
#[tokio::test]
async fn a_pushed_request_signs_in_consents_and_mints_one_code() {
    let world = world().await;
    let client = register(&world).await;
    let request_uri = push(&world, &client, &[]).await;

    // No session: to the login page, carrying only the reference. The
    // query's own redirect_uri and scope are not read.
    let unsigned = authorize(
        &world,
        &client.id,
        &request_uri,
        "&redirect_uri=https%3A%2F%2Fevil.example.test%2Fcb&scope=admin",
        None,
    )
    .await;
    let login_url = location(&unsigned);
    assert!(login_url.starts_with("/oauth2/login?"), "{login_url}");
    assert!(
        login_url.contains(&format!(
            "request_uri={}",
            urlencoding::encode(&request_uri)
        )),
        "{login_url}"
    );
    assert!(!login_url.contains("redirect_uri"), "{login_url}");

    let login_page = AxumTestRequest::get(&login_url)
        .send(app(&world.resources))
        .await;
    assert_eq!(login_page.status(), 200);
    assert!(
        login_page
            .body_text()
            .contains(&format!("name=\"request_uri\" value=\"{request_uri}\"")),
        "the login form carries the request_uri"
    );

    let signed_in = AxumTestRequest::post("/oauth2/login")
        .form(&[
            ("client_id", client.id.as_str()),
            ("request_uri", request_uri.as_str()),
            ("redirect_uri", ""),
            ("email", world.user.email.as_str()),
            ("password", PASSWORD),
        ])
        .send(app(&world.resources))
        .await;
    assert_eq!(signed_in.status(), 302, "{}", signed_in.body_text());
    let back = location(&signed_in);
    assert_eq!(
        back,
        format!(
            "/oauth2/authorize?client_id={}&request_uri={}",
            urlencoding::encode(&client.id),
            urlencoding::encode(&request_uri)
        )
    );
    let session = cookies_set(&signed_in);

    let screen = AxumTestRequest::get(&back)
        .header("cookie", &session)
        .send(app(&world.resources))
        .await;
    assert_eq!(screen.status(), 200, "{}", screen.body_text());
    let page = screen.body_text();
    assert!(
        page.contains(&format!("name=\"request_uri\" value=\"{request_uri}\"")),
        "the consent form carries the request_uri"
    );

    // The form's echoed redirect_uri is ignored: the stored one is used.
    let approved = approve(&world, &client, &request_uri, &session).await;
    assert_eq!(approved.status(), 303, "{}", approved.body_text());
    let code = code_of(&approved);
    let token = exchange(&world, &client, &code).await;
    assert_eq!(token.status(), 200, "{}", token.body_text());

    assert_request_uri_refused(&authorize(&world, &client.id, &request_uri, "", None).await);
    let signed_in_refusal = authorize(&world, &client.id, &request_uri, "", Some(&session)).await;
    assert_eq!(
        signed_in_refusal.status(),
        400,
        "refused in the athlete's own language too"
    );
    assert!(signed_in_refusal.header("location").is_none());
    assert_request_uri_refused(&approve(&world, &client, &request_uri, "").await);
}

/// A remembered approval mints at once, and that spends the request too.
#[tokio::test]
async fn a_skipped_screen_spends_the_pushed_request() {
    let world = world().await;
    let client = register(&world).await;
    let first = push(&world, &client, &[]).await;
    authorize(&world, &client.id, &first, "", Some(&world.cookie)).await;
    code_of(&approve(&world, &client, &first, &world.cookie).await);

    let second = push(&world, &client, &[]).await;
    let remembered = authorize(&world, &client.id, &second, "", Some(&world.cookie)).await;
    assert_eq!(remembered.status(), 303, "{}", remembered.body_text());
    code_of(&remembered);
    assert_request_uri_refused(&authorize(&world, &client.id, &second, "", None).await);
}

/// A public client authenticates its push by its `client_id` alone, and its
/// code is redeemed with PKCE.
#[tokio::test]
async fn a_public_client_pushes_with_its_client_id_alone() {
    let world = world().await;
    let client = register_as(&world, Some("none")).await;
    assert_eq!(client.secret, "");
    let request_uri = push(&world, &client, &[]).await;

    let screen = authorize(&world, &client.id, &request_uri, "", Some(&world.cookie)).await;
    assert_eq!(screen.status(), 200, "{}", screen.body_text());
    let code = code_of(&approve(&world, &client, &request_uri, &world.cookie).await);
    let token = exchange(&world, &client, &code).await;
    assert_eq!(token.status(), 200, "{}", token.body_text());
}

/// An unused request past its expiry no longer resolves.
#[tokio::test]
async fn an_expired_pushed_request_is_refused() {
    let world = world().await;
    let client = register(&world).await;
    let request_uri = push(&world, &client, &[]).await;
    set_expiry(&world, &request_uri, Timestamp::now() - Elapsed::seconds(1)).await;

    assert_request_uri_refused(&authorize(&world, &client.id, &request_uri, "", None).await);
}

/// The first resolve extends the request past its 90 seconds, so the athlete
/// has time to sign in and consent.
#[tokio::test]
async fn the_first_resolve_extends_the_pushed_request() {
    let world = world().await;
    let client = register(&world).await;
    let request_uri = push(&world, &client, &[]).await;
    let stored = |at| {
        let repository = world.resources.common.repos.oauth2_server.clone();
        let hash = request_uri_hash(&request_uri);
        let client_id = client.id.clone();
        async move {
            repository
                .find_pushed_request(&hash, &client_id, at)
                .await
                .unwrap()
        }
    };
    let pushed = stored(Timestamp::now()).await.expect("a stored request");
    assert!(pushed.expires_at <= Timestamp::now() + Elapsed::seconds(90));
    assert!(!pushed.parameters.contains("client_secret"));

    authorize(&world, &client.id, &request_uri, "", None).await;
    let resolved = stored(Timestamp::now()).await.expect("still unused");
    assert!(
        resolved.expires_at > Timestamp::now() + Elapsed::minutes(29),
        "the first resolve gives the flow 30 minutes"
    );
}

/// Another client cannot resolve a request it did not push.
#[tokio::test]
async fn another_clients_request_uri_is_refused() {
    let world = world().await;
    let owner = register(&world).await;
    let other = register(&world).await;
    let request_uri = push(&world, &owner, &[]).await;

    assert_request_uri_refused(&authorize(&world, &other.id, &request_uri, "", None).await);
    // Still the owner's, untouched by the refusal.
    let screen = authorize(&world, &owner.id, &request_uri, "", Some(&world.cookie)).await;
    assert_eq!(screen.status(), 200, "{}", screen.body_text());
}

/// A push is authenticated as at the token endpoint, and carries the request
/// itself, never a reference to one.
#[tokio::test]
async fn a_push_is_refused_without_its_secret_or_with_a_request_uri() {
    let world = world().await;
    let client = register(&world).await;

    let wrong = Client {
        id: client.id.clone(),
        secret: "not-the-secret".to_owned(),
    };
    let refused = push_with(&world, &wrong, &[]).await;
    assert_eq!(refused.status(), 400, "{}", refused.body_text());
    let body: Value = serde_json::from_str(&refused.body_text()).unwrap();
    assert_eq!(body["error"], "invalid_client");

    let basic = general_purpose::STANDARD.encode(format!("{}:not-the-secret", client.id));
    let challenged = AxumTestRequest::post("/oauth2/par")
        .header("authorization", &format!("Basic {basic}"))
        .form(&[("response_type", "code"), ("redirect_uri", REDIRECT)])
        .send(app(&world.resources))
        .await;
    assert_eq!(challenged.status(), 401, "{}", challenged.body_text());

    let nested = push_with(&world, &client, &[("request_uri", "urn:x")]).await;
    assert_eq!(nested.status(), 400);
    let body: Value = serde_json::from_str(&nested.body_text()).unwrap();
    assert_eq!(body["error"], "invalid_request");

    let pushed = push_with(&world, &client, &[]).await;
    assert_eq!(pushed.status(), 201, "control: the client pushes");
}

/// A request the authorization endpoint would refuse is refused at the push.
#[tokio::test]
async fn a_push_is_checked_as_an_authorization_request() {
    let world = world().await;
    let client = register(&world).await;
    let form = [
        ("client_id", client.id.as_str()),
        ("client_secret", client.secret.as_str()),
        ("response_type", "code"),
        ("redirect_uri", "https://unregistered.example.test/cb"),
    ];
    let refused = AxumTestRequest::post("/oauth2/par")
        .form(&form)
        .send(app(&world.resources))
        .await;
    assert_eq!(refused.status(), 400, "{}", refused.body_text());
    let body: Value = serde_json::from_str(&refused.body_text()).unwrap();
    assert_eq!(body["error"], "invalid_request");
}

/// A pushed `prompt=login` asks for the password once: the return from the
/// login page resolves the same request and goes on to consent.
#[tokio::test]
async fn a_pushed_prompt_login_asks_for_the_password_once() {
    let world = world().await;
    let client = register(&world).await;
    let request_uri = push(&world, &client, &[("prompt", "login")]).await;

    let first = authorize(&world, &client.id, &request_uri, "", Some(&world.cookie)).await;
    assert!(
        location(&first).starts_with("/oauth2/login?"),
        "prompt=login asks again whatever the session"
    );
    let again = authorize(&world, &client.id, &request_uri, "", Some(&world.cookie)).await;
    assert_eq!(
        again.status(),
        200,
        "bug: the return from login was sent to the login page again: {}",
        again.body_text()
    );
}

/// The hourly client sweep deletes the pushed requests past their expiry, and
/// keeps the ones still resolving.
#[tokio::test]
async fn the_client_sweep_deletes_expired_pushed_requests() {
    let world = world().await;
    let client = register(&world).await;
    let expired = push(&world, &client, &[]).await;
    let live = push(&world, &client, &[]).await;
    set_expiry(&world, &expired, Timestamp::now() - Elapsed::seconds(1)).await;

    let sweep = ClientRegistrationManager::new(world.resources.common.repos.oauth2_server.clone())
        .sweep_stale_clients(
            &world.resources.common.config.oauth2_server.client_retention,
            Timestamp::now(),
        )
        .await
        .unwrap();
    assert_eq!(sweep.expired_pushed_requests, 1);
    let screen = authorize(&world, &client.id, &live, "", Some(&world.cookie)).await;
    assert_eq!(screen.status(), 200, "the live request still resolves");
}
