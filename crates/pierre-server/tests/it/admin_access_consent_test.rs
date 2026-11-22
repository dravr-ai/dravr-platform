// ABOUTME: Admin access at consent (carnet#1511, step 12): the unticked box a live admin alone is offered, and what ticking it grants
// ABOUTME: Through the real authorize, consent, token, /mcp and /a2a doors: admin tools only when ticked, a day-long family, audit fails closed

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! JF, Q20 (strict), Q25 and Q26: `admin` stays unpublished; an admin is
//! offered an unticked Admin access box on every app's consent screen, with
//! the host the access goes to in bold; an admin always sees the screen; a
//! refresh family holding `admin` lives a day; a failed audit write refuses
//! the admin call over MCP and A2A. A non-admin never sees the box, and no
//! request or form a non-admin can send makes a grant hold `admin`.
//!
//! The box's own lines are authored in dravr-contremaitre first and are not
//! in the pinned catalogue yet, and the box is never shown without them. So
//! these tests serve the screen from a catalogue carrying the proposed lines
//! ([`with_admin_access_lines`]), as a runtime sync would once they land, and
//! one test pins that the real catalogue shows no box at all.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::extract::connect_info::MockConnectInfo;
use axum::http::Request;
use base64::{engine::general_purpose, Engine as _};
use chrono::{Duration, Utc};
use jsonwebtoken::dangerous::insecure_decode;
use jsonwebtoken::{encode, Algorithm, Header};
use pierre_auth::auth::Claims;
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::endpoints::ADMIN_REFRESHES_AT_TOKEN_ENDPOINT;
use pierre_auth::oauth2_server::models::ClientRegistrationRequest;
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_contremaitre::MessagingStringsRegistry;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::models::User;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::{AuditEventFilter, AuditEventRecord};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_identity::oauth2::{OAuth2Context, OAuth2Routes};
use pierre_routes_identity::{ConsentHtmlParams, ConsentRole, ConsentText};
use secrecy::ExposeSecret;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use url::Url;
use uuid::Uuid;

use crate::common::{self, create_test_tenant};
use crate::helpers::axum_test::{AxumTestRequest, AxumTestResponse};

const REDIRECT: &str = "https://client.example.test/callback";
const STATE: &str = "admin-access-state";
const VERIFIER: &str = "admin-access-pkce-verifier-0123456789-abcdefghijklmnopqrstu";

/// The proposed English of the box's lines, not yet in the catalogue: what the
/// FINDING on carnet#1511 asks dravr-contremaitre to add.
const PROPOSED_LINES: [(&str, &str); 5] = [
    ("oauth.consent.adminAccessLabel", "Admin access"),
    (
        "oauth.consent.adminAccessHint",
        "Only admins see this choice. It is never remembered: you are asked each time you connect an app, and you can withdraw it in Connected apps.",
    ),
    ("oauth.consent.adminAccessGoesTo", "Admin access goes to {0}."),
    (
        "oauth.consent.adminAccessGoesToTwoHosts",
        "Admin access goes to {0}, for the app published at {1}.",
    ),
    (
        "oauth.consent.adminAccessThisComputer",
        "an app on this computer",
    ),
];

/// `registry` with the box's proposed English lines, as a runtime sync would
/// deliver them once dravr-contremaitre carries them.
pub(crate) fn with_admin_access_lines(
    registry: MessagingStringsRegistry,
) -> MessagingStringsRegistry {
    for (key, line) in PROPOSED_LINES {
        registry.update(key, "en", line.to_owned(), format!("proposed-{key}"));
    }
    registry
}

// ============================================================================
// Harness: the OAuth routes over a catalogue carrying the box's lines, and the
// server's own /mcp and /a2a doors over the same database
// ============================================================================

async fn resources() -> Arc<ServerContext> {
    common::init_server_config();
    common::init_test_http_clients();
    common::create_test_server_resources().await.unwrap()
}

/// The platform's MCP resource, which an admin grant may be bound to.
fn mcp_resource(resources: &Arc<ServerContext>) -> String {
    resources.common.config.oauth2_server.mcp_resources()[0].to_owned()
}

/// The platform's A2A resource, which an admin grant may be bound to.
fn a2a_resource(resources: &Arc<ServerContext>) -> String {
    resources.common.config.oauth2_server.a2a_resources()[0].clone()
}

/// The OAuth routes as the server mounts them, over `strings`.
fn oauth2_routes_with(
    resources: &Arc<ServerContext>,
    strings: MessagingStringsRegistry,
) -> axum::Router {
    let context = OAuth2Context {
        database: resources.agent.database.clone(),
        oauth2_server: resources.common.repos.oauth2_server.clone(),
        tenants: resources.common.repos.tenants.clone(),
        users: resources.common.repos.users.clone(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        config: Arc::new(resources.common.config.oauth2_server.clone()),
        rate_limiter: Arc::new(OAuth2RateLimiter::new(
            None,
            OAuth2RateLimiter::local_window_store(),
            &resources.common.config.rate_limiting,
        )),
        refresh_token_expiry_days: 30,
        csrf_manager: resources.auth.csrf_manager.clone(),
        accounts: resources.account_service(),
        google_sign_in: None,
        strings: Arc::new(strings),
        first_party_only_sources: Arc::default(),
    };
    OAuth2Routes::routes(context).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 41_511))))
}

/// The OAuth routes over a catalogue carrying the box's lines.
fn oauth2_routes(resources: &Arc<ServerContext>) -> axum::Router {
    oauth2_routes_with(
        resources,
        with_admin_access_lines(MessagingStringsRegistry::new()),
    )
}

fn pkce_challenge() -> String {
    general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()))
}

/// A dynamically registered client; `(client_id, secret)`.
async fn register(resources: &Arc<ServerContext>) -> (String, String) {
    let registered = ClientRegistrationManager::new(resources.common.repos.oauth2_server.clone())
        .register_client(
            ClientRegistrationRequest {
                redirect_uris: vec![REDIRECT.to_owned()],
                client_name: Some("Claude".to_owned()),
                client_uri: None,
                grant_types: None,
                response_types: None,
                scope: None,
            },
            MAX_PENDING_REGISTRATIONS,
        )
        .await
        .unwrap();
    (
        registered.client_id,
        registered.client_secret.expose_secret().to_owned(),
    )
}

/// A tenant member signed in to the authorization server; with `admin`, an
/// admin. The `/oauth2/login` cookie's token.
async fn member(resources: &Arc<ServerContext>, admin: bool) -> (User, String) {
    let email = format!("admin-access-{}@example.test", Uuid::new_v4());
    let (user, _app_session) = create_test_tenant(resources, &email).await.unwrap();
    if admin {
        resources
            .common
            .repos
            .users
            .set_admin_status(user.id, true)
            .await
            .unwrap();
    }
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()
        .first()
        .map(|tenant| tenant.id.to_string());
    let session = resources
        .auth
        .auth_manager
        .generate_authorization_session_token(&user, &resources.auth.jwks_manager, tenant_id)
        .unwrap();
    (user, session)
}

/// The authorization server's sign-in cookie: `pierre_session` over the test
/// server's plain-HTTP issuer.
fn cookie(session: &str) -> String {
    format!("pierre_session={session}")
}

/// The authorization request `client_id` sends, asking for `scope` and
/// naming `resource` when given.
fn authorize_uri(client_id: &str, scope: Option<&str>, resource: Option<&str>) -> String {
    let mut uri = format!(
        "/oauth2/authorize?response_type=code&client_id={}&redirect_uri={}&state={STATE}-{}&code_challenge={}&code_challenge_method=S256",
        urlencoding::encode(client_id),
        urlencoding::encode(REDIRECT),
        Uuid::new_v4(),
        pkce_challenge(),
    );
    for (name, value) in [("scope", scope), ("resource", resource)] {
        if let Some(value) = value {
            uri.push('&');
            uri.push_str(name);
            uri.push('=');
            uri.push_str(&urlencoding::encode(value));
        }
    }
    uri
}

/// What the signed-in user is answered at `/oauth2/authorize`.
async fn authorize(
    resources: &Arc<ServerContext>,
    client_id: &str,
    session: &str,
    scope: Option<&str>,
    resource: Option<&str>,
) -> AxumTestResponse {
    AxumTestRequest::get(&authorize_uri(client_id, scope, resource))
        .header("cookie", &cookie(session))
        .send(oauth2_routes(resources))
        .await
}

/// The consent page the signed-in user is shown.
async fn consent_page(
    resources: &Arc<ServerContext>,
    client_id: &str,
    session: &str,
    scope: Option<&str>,
    resource: Option<&str>,
) -> String {
    let page = authorize(resources, client_id, session, scope, resource).await;
    assert_eq!(
        page.status(),
        200,
        "the screen is shown: {}",
        page.body_text()
    );
    page.body_text()
}

fn hidden(page: &str, name: &str) -> String {
    page.split(&format!("name=\"{name}\" value=\""))
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_else(|| panic!("the consent form carries {name}: {page}"))
        .to_owned()
}

/// Approve the consent screen, ticking Admin access when `tick`; the code.
async fn approve(
    resources: &Arc<ServerContext>,
    client_id: &str,
    session: &str,
    scope: Option<&str>,
    resource: Option<&str>,
    tick: bool,
) -> String {
    let page = consent_page(resources, client_id, session, scope, resource).await;
    let csrf_token = hidden(&page, "csrf_token");
    let echoed_scope = hidden(&page, "scope");
    let state = hidden(&page, "state");
    let challenge = pkce_challenge();
    let mut form = vec![
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", REDIRECT),
        ("state", state.as_str()),
        ("scope", echoed_scope.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("resource", resource.unwrap_or_default()),
        ("csrf_token", csrf_token.as_str()),
        ("decision", "approve"),
    ];
    if tick {
        form.push(("admin_access", "granted"));
    }
    let approved = AxumTestRequest::post("/oauth2/consent")
        .header("cookie", &cookie(session))
        .form(&form)
        .send(oauth2_routes(resources))
        .await;
    code_of(&approved)
}

/// The code a redirect to the client carries.
fn code_of(response: &AxumTestResponse) -> String {
    assert_eq!(response.status(), 303, "{}", response.body_text());
    let location = response.header("location").expect("a redirect");
    assert!(location.starts_with(REDIRECT), "{location}");
    Url::parse(location)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map_or_else(
            || panic!("a code: {location}"),
            |(_, code)| code.into_owned(),
        )
}

async fn token_request(
    resources: &Arc<ServerContext>,
    fields: &[(&str, &str)],
) -> AxumTestResponse {
    AxumTestRequest::post("/oauth2/token")
        .form(&fields)
        .send(oauth2_routes(resources))
        .await
}

/// A token response's `(access_token, refresh_token, scope)`.
fn issued(response: &AxumTestResponse) -> (String, String, String) {
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let body: Value = serde_json::from_str(&response.body_text()).unwrap();
    (
        body["access_token"].as_str().unwrap().to_owned(),
        body["refresh_token"].as_str().unwrap().to_owned(),
        body["scope"].as_str().unwrap_or_default().to_owned(),
    )
}

async fn exchange(
    resources: &Arc<ServerContext>,
    (client_id, secret): (&str, &str),
    code: &str,
) -> (String, String, String) {
    issued(
        &token_request(
            resources,
            &[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", REDIRECT),
                ("client_id", client_id),
                ("client_secret", secret),
                ("code_verifier", VERIFIER),
            ],
        )
        .await,
    )
}

async fn refresh(
    resources: &Arc<ServerContext>,
    (client_id, secret): (&str, &str),
    refresh_token: &str,
) -> AxumTestResponse {
    token_request(
        resources,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
            ("client_secret", secret),
        ],
    )
    .await
}

/// A connection: the user approves `client` for the platform's MCP resource,
/// ticking Admin access when `tick`; the token response.
async fn connect(
    resources: &Arc<ServerContext>,
    client: (&str, &str),
    session: &str,
    resource: &str,
    tick: bool,
) -> (String, String, String) {
    let code = approve(resources, client.0, session, None, Some(resource), tick).await;
    exchange(resources, client, &code).await
}

fn scope_names(scope: &str) -> Vec<&str> {
    scope.split_whitespace().collect()
}

/// A JSON-RPC request to `/mcp` with `bearer`; the HTTP status and the body.
async fn post_mcp(resources: &Arc<ServerContext>, bearer: &str, body: Value) -> (u16, Value) {
    let request = Request::post("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("authorization", format!("Bearer {bearer}"))
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = ProviderToolRouter::build_http_app(resources)
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(&bytes) }));
    (status, body)
}

/// The tool names `/mcp` lists to `bearer`.
async fn listed(resources: &Arc<ServerContext>, bearer: &str) -> Vec<String> {
    let (status, body) = post_mcp(
        resources,
        bearer,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    body["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("a tool list: {body}"))
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect()
}

/// `tools/call` of `tool` at `/mcp` with `bearer`.
async fn call_mcp(resources: &Arc<ServerContext>, bearer: &str, tool: &str) -> (u16, Value) {
    post_mcp(
        resources,
        bearer,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": tool, "arguments": {} },
        }),
    )
    .await
}

/// Whether a `tools/call` answer is the tool's own success.
fn succeeded(status: u16, body: &Value) -> bool {
    status == 200 && body["error"].is_null() && body["result"]["isError"] != json!(true)
}

/// A2A `SendMessage` naming `tool` with `bearer`; the answer.
async fn call_a2a(resources: &Arc<ServerContext>, bearer: &str, tool: &str) -> Value {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "SendMessage",
        "params": { "message": {
            "messageId": format!("m-{}", Uuid::new_v4()),
            "role": "ROLE_USER",
            "parts": [{ "data": { "tool_name": tool, "parameters": {} } }],
        } },
    });
    let request = Request::post("/a2a/jsonrpc")
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .header("A2A-Version", "1.0")
        .header("authorization", format!("Bearer {bearer}"))
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = ProviderToolRouter::build_http_app(resources)
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| panic!("{status}: {}", String::from_utf8_lossy(&bytes)))
}

/// `actor`'s rows on the admin audit trail.
async fn trail(resources: &Arc<ServerContext>, actor: Uuid) -> Vec<AuditEventRecord> {
    resources
        .common
        .repos
        .admin_audit
        .list(
            &AuditEventFilter {
                actor_user_id: Some(actor),
                ..AuditEventFilter::default()
            },
            50,
        )
        .await
        .unwrap()
}

/// An admin tool, and a tool reached from outside only through the admin
/// exception: both are on the admin audit trail.
const AUDITED: [&str; 2] = ["admin_list_system_agents", "browse_agent_store"];

/// Drop the admin audit trail, so no row of it can be written.
async fn drop_audit_trail(resources: &Arc<ServerContext>) {
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query("DROP TABLE audit_events")
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query("DROP TABLE audit_events CASCADE")
                .execute(db.pool())
                .await
                .unwrap();
        }
    }
}

/// Move the start of `client_id`'s refresh families back to `started`, as
/// if their first token had been minted then.
async fn family_started_at(
    resources: &Arc<ServerContext>,
    client_id: &str,
    started: chrono::DateTime<Utc>,
) {
    const STATEMENT: &str =
        "UPDATE oauth2_refresh_tokens SET family_created_at = $1 WHERE client_id = $2";
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(STATEMENT)
                .bind(started)
                .bind(client_id)
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(STATEMENT)
                .bind(started)
                .bind(client_id)
                .execute(db.pool())
                .await
                .unwrap();
        }
    }
}

/// When each refresh token `client_id` holds stops exchanging.
async fn refresh_expiries(
    resources: &Arc<ServerContext>,
    client_id: &str,
) -> Vec<chrono::DateTime<Utc>> {
    const QUERY: &str = "SELECT expires_at FROM oauth2_refresh_tokens WHERE client_id = $1";
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => sqlx::query_scalar(QUERY)
            .bind(client_id)
            .fetch_all(db.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => sqlx::query_scalar(QUERY)
            .bind(client_id)
            .fetch_all(db.pool())
            .await
            .unwrap(),
    }
}

/// The active grant rows `user` holds for `client_id`, by scope.
async fn grant_scopes(resources: &Arc<ServerContext>, user: &User, client_id: &str) -> Vec<String> {
    let repos = &resources.common.repos;
    let tenant = repos
        .tenants
        .first_for_user(user.id)
        .await
        .unwrap()
        .unwrap();
    repos
        .oauth2_server
        .list_client_grants(&user.id.to_string(), &tenant.to_string())
        .await
        .unwrap()
        .into_iter()
        .filter(|grant| grant.client_id == client_id && grant.revoked_at.is_none())
        .map(|grant| grant.scope)
        .collect()
}

// ============================================================================
// The screen
// ============================================================================

#[tokio::test]
async fn an_admin_is_offered_an_unticked_box_naming_the_redirect_host_in_bold() {
    let resources = resources().await;
    let (client_id, _) = register(&resources).await;
    let (_admin, session) = member(&resources, true).await;

    let page = consent_page(
        &resources,
        &client_id,
        &session,
        None,
        Some(&mcp_resource(&resources)),
    )
    .await;

    let input = page
        .split("<input type=\"checkbox\" name=\"admin_access\"")
        .nth(1)
        .and_then(|rest| rest.split('>').next())
        .unwrap_or_else(|| panic!("bug: an admin is offered the Admin access box: {page}"));
    assert!(
        !input.contains("checked"),
        "bug: the Admin access box is offered unticked: {input}"
    );
    assert!(
        page.contains("Admin access goes to <strong>client.example.test</strong>."),
        "bug: the box names the redirect host in bold: {page}"
    );
    let callout = page
        .split("<div class=\"callout\">")
        .nth(1)
        .and_then(|rest| rest.split("</div>").next())
        .unwrap_or_default();
    assert!(
        callout.contains("<span class=\"client\">Claude</span>"),
        "bug: the admin line, naming the client, says what ticking the box gives, beside it: {page}"
    );
    let form = page.split("<form").nth(1).unwrap();
    assert!(
        form.contains("name=\"admin_access\""),
        "bug: the box is inside the form it is submitted with"
    );
}

#[tokio::test]
async fn an_admin_is_offered_no_box_on_a_request_not_bound_to_the_platform() {
    let resources = resources().await;
    let (client_id, _) = register(&resources).await;
    let (_admin, session) = member(&resources, true).await;

    // No resource named: the token would carry the platform audience by
    // default, and admin is minted only for a resource bound on purpose (M6).
    let page = consent_page(&resources, &client_id, &session, None, None).await;
    assert!(
        !page.contains("name=\"admin_access\""),
        "bug: a request naming no resource offers Admin access: {page}"
    );
    assert!(
        !page.contains("<div class=\"callout\">"),
        "bug: the admin line promises what this connection cannot get: {page}"
    );
}

#[tokio::test]
async fn a_non_admin_never_sees_the_box_and_no_crafted_request_or_form_grants_admin() {
    let resources = resources().await;
    let client = register(&resources).await;
    let (athlete, session) = member(&resources, false).await;
    let resource = mcp_resource(&resources);

    // The client asks for admin outright: dropped, never refused (JF, Q20),
    // never listed as a scope, never echoed in the form.
    let page = consent_page(
        &resources,
        &client.0,
        &session,
        Some("fitness:read admin"),
        Some(&resource),
    )
    .await;
    assert!(
        !page.contains("name=\"admin_access\""),
        "bug: a non-admin is offered Admin access: {page}"
    );
    assert!(
        !page.contains("<li>admin"),
        "bug: admin listed as a scope: {page}"
    );
    assert_eq!(
        hidden(&page, "scope"),
        "fitness:read",
        "bug: the form echoes admin back"
    );

    // A form that names admin in its scope and ticks the box anyway.
    let csrf_token = hidden(&page, "csrf_token");
    let state = hidden(&page, "state");
    let challenge = pkce_challenge();
    let forged = AxumTestRequest::post("/oauth2/consent")
        .header("cookie", &cookie(&session))
        .form(&[
            ("response_type", "code"),
            ("client_id", client.0.as_str()),
            ("redirect_uri", REDIRECT),
            ("state", state.as_str()),
            ("scope", "fitness:read admin"),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("resource", resource.as_str()),
            ("csrf_token", csrf_token.as_str()),
            ("decision", "approve"),
            ("admin_access", "granted"),
        ])
        .send(oauth2_routes(&resources))
        .await;
    let code = code_of(&forged);
    let (access_token, _refresh, scope) = exchange(&resources, (&client.0, &client.1), &code).await;

    assert!(
        !scope_names(&scope).contains(&"admin"),
        "bug: a non-admin's token response names admin: {scope}"
    );
    let claims = insecure_decode::<Claims>(&access_token).unwrap().claims;
    assert!(
        !claims.scope.split_whitespace().any(|name| name == "admin"),
        "bug: a non-admin's token holds admin: {}",
        claims.scope
    );
    for granted in grant_scopes(&resources, &athlete, &client.0).await {
        assert!(
            !scope_names(&granted).contains(&"admin"),
            "bug: a non-admin's grant row holds admin: {granted}"
        );
    }
    assert!(
        !listed(&resources, &access_token)
            .await
            .iter()
            .any(|tool| tool == "admin_list_system_agents"),
        "bug: an admin tool listed to a non-admin"
    );
}

#[tokio::test]
async fn an_admin_always_sees_the_screen_and_an_athlete_with_a_remembered_grant_does_not() {
    let resources = resources().await;
    let client = register(&resources).await;
    let resource = mcp_resource(&resources);

    let (_athlete, athlete_session) = member(&resources, false).await;
    approve(
        &resources,
        &client.0,
        &athlete_session,
        None,
        Some(&resource),
        false,
    )
    .await;
    let again = authorize(
        &resources,
        &client.0,
        &athlete_session,
        None,
        Some(&resource),
    )
    .await;
    assert_eq!(
        again.status(),
        303,
        "an athlete's remembered grant skips the screen: {}",
        again.body_text()
    );

    let (_admin, admin_session) = member(&resources, true).await;
    approve(
        &resources,
        &client.0,
        &admin_session,
        None,
        Some(&resource),
        false,
    )
    .await;
    let again = authorize(&resources, &client.0, &admin_session, None, Some(&resource)).await;
    assert_eq!(
        again.status(),
        200,
        "bug: an admin's grant was renewed without the screen (Q20)"
    );
    approve(
        &resources,
        &client.0,
        &admin_session,
        None,
        Some(&resource),
        true,
    )
    .await;
    let again = authorize(&resources, &client.0, &admin_session, None, Some(&resource)).await;
    assert_eq!(
        again.status(),
        200,
        "bug: an admin-access grant was renewed without the screen (Q20)"
    );
}

#[test]
fn the_box_names_a_differing_document_host_and_says_this_computer_for_a_loopback() {
    let strings = with_admin_access_lines(MessagingStringsRegistry::new());
    let text = ConsentText::new(&strings, "en");
    let render = |client_id: &str, redirect_uri: &str| {
        OAuth2Routes::generate_consent_html(ConsentHtmlParams {
            client_id,
            redirect_uri,
            response_type: "code",
            state: "s",
            scope: "fitness:read",
            code_challenge: "c",
            code_challenge_method: "S256",
            resource: "",
            csrf_token: "t",
            client_label: "Claude",
            account_email: "admin@example.test",
            switch_account_url: "/oauth2/login",
            first_party_only_sources: &[],
            text: &text,
            role: ConsentRole {
                coach: false,
                admin: true,
            },
        })
    };

    let same = render(
        "https://claude.ai/oauth/mcp-oauth-client-metadata",
        "https://claude.ai/api/mcp/auth_callback",
    );
    assert!(
        same.contains("Admin access goes to <strong>claude.ai</strong>."),
        "one host when the document is where the code goes: {same}"
    );

    let two = render(
        "https://tool.example.test/client.json",
        "https://elsewhere.example.test/callback",
    );
    assert!(
        two.contains(
            "Admin access goes to <strong>elsewhere.example.test</strong>, for the app published at <strong>tool.example.test</strong>."
        ),
        "bug: a differing client_id host is named in bold too: {two}"
    );

    let loopback = render(
        "https://claude.ai/oauth/claude-code-client-metadata",
        "http://127.0.0.1:53124/callback",
    );
    assert!(
        loopback.contains(
            "Admin access goes to <strong>an app on this computer</strong>, for the app published at <strong>claude.ai</strong>."
        ),
        "bug: a loopback redirect reads as an app on this computer: {loopback}"
    );
    let registered = render("3f1c6b2e-dcr-client", "http://localhost:6274/callback");
    assert!(
        registered.contains("Admin access goes to <strong>an app on this computer</strong>."),
        "bug: a loopback redirect reads as an app on this computer: {registered}"
    );
}

#[test]
fn the_box_is_not_offered_while_the_catalogue_has_no_lines_for_it() {
    // Whatever the pinned catalogue carries, a catalogue with the box's lines
    // read empty shows no box: they are supplied first, then blanked.
    let strings = with_admin_access_lines(MessagingStringsRegistry::new());
    for (key, _) in PROPOSED_LINES {
        strings.update(key, "en", String::new(), format!("blank-{key}"));
    }
    let html = OAuth2Routes::generate_consent_html(ConsentHtmlParams {
        client_id: "mcp_client_1",
        redirect_uri: "https://claude.ai/api/mcp/auth_callback",
        response_type: "code",
        state: "s",
        scope: "fitness:read",
        code_challenge: "c",
        code_challenge_method: "S256",
        resource: "",
        csrf_token: "t",
        client_label: "Claude",
        account_email: "admin@example.test",
        switch_account_url: "/oauth2/login",
        first_party_only_sources: &[],
        text: &ConsentText::new(&strings, "en"),
        role: ConsentRole {
            coach: false,
            admin: true,
        },
    });
    assert!(
        !html.contains("admin_access"),
        "bug: an unlabelled box that grants admin was offered: {html}"
    );
}

// ============================================================================
// What ticking grants
// ============================================================================

#[tokio::test]
async fn an_unticked_box_grants_no_admin_and_the_admin_tools_stay_withheld() {
    let resources = resources().await;
    let client = register(&resources).await;
    let (admin, session) = member(&resources, true).await;

    let (access_token, _refresh, scope) = connect(
        &resources,
        (&client.0, &client.1),
        &session,
        &mcp_resource(&resources),
        false,
    )
    .await;

    assert!(!scope_names(&scope).contains(&"admin"), "{scope}");
    let tools = listed(&resources, &access_token).await;
    for withheld in ["admin_list_system_agents", "browse_agent_store"] {
        assert!(
            !tools.iter().any(|tool| tool == withheld),
            "bug: {withheld} listed on a connection without Admin access"
        );
    }
    let (status, body) = call_mcp(&resources, &access_token, "admin_list_system_agents").await;
    assert!(
        !succeeded(status, &body),
        "bug: an admin tool ran without Admin access: {status} {body}"
    );
    for granted in grant_scopes(&resources, &admin, &client.0).await {
        assert!(!scope_names(&granted).contains(&"admin"), "{granted}");
    }
}

#[tokio::test]
async fn a_ticked_box_lists_and_runs_every_admin_tool_over_mcp_each_call_on_the_trail() {
    let resources = resources().await;
    let client = register(&resources).await;
    let (admin, session) = member(&resources, true).await;

    let (access_token, _refresh, scope) = connect(
        &resources,
        (&client.0, &client.1),
        &session,
        &mcp_resource(&resources),
        true,
    )
    .await;

    assert!(
        scope_names(&scope).contains(&"admin"),
        "bug: the token response names the admin it issued: {scope}"
    );
    let claims = insecure_decode::<Claims>(&access_token).unwrap().claims;
    assert!(
        claims.scope.split_whitespace().any(|name| name == "admin"),
        "bug: the access token holds admin: {}",
        claims.scope
    );
    assert!(
        grant_scopes(&resources, &admin, &client.0)
            .await
            .iter()
            .any(|granted| scope_names(granted).contains(&"admin")),
        "bug: the recorded grant is the one the code carries"
    );

    let tools = listed(&resources, &access_token).await;
    for offered in ["admin_list_system_agents", "browse_agent_store"] {
        assert!(
            tools.iter().any(|tool| tool == offered),
            "bug: {offered} not listed on a connection with Admin access: {tools:?}"
        );
    }

    let (status, body) = call_mcp(&resources, &access_token, "admin_list_system_agents").await;
    assert!(
        succeeded(status, &body),
        "bug: the admin tool runs: {status} {body}"
    );
    let (status, body) = call_mcp(&resources, &access_token, "browse_agent_store").await;
    assert!(
        succeeded(status, &body),
        "bug: a first-party-only tool runs through the admin exception: {status} {body}"
    );

    let actions: Vec<String> = trail(&resources, admin.id)
        .await
        .into_iter()
        .map(|row| row.action)
        .collect();
    for action in ["tool.admin_list_system_agents", "tool.browse_agent_store"] {
        assert!(
            actions.iter().any(|recorded| recorded == action),
            "bug: {action} is on the admin audit trail: {actions:?}"
        );
    }
}

#[tokio::test]
async fn a_failed_audit_write_refuses_the_admin_call_over_mcp_and_a2a() {
    let resources = resources().await;
    let mcp_client = register(&resources).await;
    let a2a_client = register(&resources).await;
    let (_admin, session) = member(&resources, true).await;
    let (mcp_token, _, mcp_scope) = connect(
        &resources,
        (&mcp_client.0, &mcp_client.1),
        &session,
        &mcp_resource(&resources),
        true,
    )
    .await;
    let (a2a_token, _, a2a_scope) = connect(
        &resources,
        (&a2a_client.0, &a2a_client.1),
        &session,
        &a2a_resource(&resources),
        true,
    )
    .await;
    assert!(scope_names(&mcp_scope).contains(&"admin"), "{mcp_scope}");
    assert!(
        scope_names(&a2a_scope).contains(&"admin"),
        "bug: an A2A-bound grant may hold admin too: {a2a_scope}"
    );

    // An admin tool and a tool reached only through the admin exception: both
    // run on these connections while their rows can be written.
    for tool in AUDITED {
        let (status, body) = call_mcp(&resources, &mcp_token, tool).await;
        assert!(succeeded(status, &body), "{tool} over MCP: {status} {body}");
        let answer = call_a2a(&resources, &a2a_token, tool).await;
        assert_eq!(
            answer["result"]["task"]["status"]["state"], "TASK_STATE_COMPLETED",
            "{tool} over A2A: {answer}"
        );
    }

    drop_audit_trail(&resources).await;

    for tool in AUDITED {
        // `/mcp` answers a failed call with a generic text; that it ran a
        // moment ago is what says this refusal is the missing row.
        let (status, body) = call_mcp(&resources, &mcp_token, tool).await;
        assert!(
            !succeeded(status, &body),
            "bug: {tool} over MCP ran with no audit row: {status} {body}"
        );
        let answer = call_a2a(&resources, &a2a_token, tool).await;
        assert_eq!(
            answer["result"]["task"]["status"]["state"], "TASK_STATE_FAILED",
            "bug: {tool} over A2A ran with no audit row: {answer}"
        );
        assert!(
            answer.to_string().contains("audit trail"),
            "bug: the A2A refusal says why: {answer}"
        );
    }
}

// ============================================================================
// The refresh chain
// ============================================================================

#[tokio::test]
async fn a_refresh_family_holding_admin_stops_a_day_after_its_first_token() {
    let resources = resources().await;
    let client = register(&resources).await;
    let (_admin, session) = member(&resources, true).await;
    let (_access, first_refresh, _scope) = connect(
        &resources,
        (&client.0, &client.1),
        &session,
        &mcp_resource(&resources),
        true,
    )
    .await;

    // Within the day the family rotates, its successor cut at the day's end.
    family_started_at(&resources, &client.0, Utc::now() - Duration::hours(23)).await;
    let (_access, second_refresh, scope) =
        issued(&refresh(&resources, (&client.0, &client.1), &first_refresh).await);
    assert!(scope_names(&scope).contains(&"admin"), "{scope}");

    // Past the day, however recent the last rotation, the family is done.
    family_started_at(&resources, &client.0, Utc::now() - Duration::hours(25)).await;
    let refused = refresh(&resources, (&client.0, &client.1), &second_refresh).await;
    assert_eq!(
        refused.status(),
        400,
        "bug: an admin family past its day refreshed: {}",
        refused.body_text()
    );
    let error: Value = serde_json::from_str(&refused.body_text()).unwrap();
    assert_eq!(error["error"], "invalid_grant", "{error}");
}

#[tokio::test]
async fn an_admin_familys_first_refresh_token_expires_within_the_day() {
    let resources = resources().await;
    let client = register(&resources).await;
    let (_admin, session) = member(&resources, true).await;
    connect(
        &resources,
        (&client.0, &client.1),
        &session,
        &mcp_resource(&resources),
        true,
    )
    .await;

    let expiries = refresh_expiries(&resources, &client.0).await;
    assert_eq!(expiries.len(), 1, "one refresh token was stored");
    assert!(
        expiries[0] <= Utc::now() + Duration::hours(24),
        "bug: an admin family's first token outlives the day: {}",
        expiries[0]
    );
}

#[tokio::test]
async fn a_demoted_admins_refresh_drops_admin_for_good() {
    let resources = resources().await;
    // The last active admin cannot be demoted: another one stays.
    member(&resources, true).await;
    let client = register(&resources).await;
    let (admin, session) = member(&resources, true).await;
    let (_access, refresh_token, scope) = connect(
        &resources,
        (&client.0, &client.1),
        &session,
        &mcp_resource(&resources),
        true,
    )
    .await;
    assert!(
        scope_names(&scope).contains(&"admin"),
        "the admin ticked the box: {scope}"
    );

    resources
        .common
        .repos
        .users
        .set_admin_status(admin.id, false)
        .await
        .unwrap();
    let (access_token, refresh_token, scope) =
        issued(&refresh(&resources, (&client.0, &client.1), &refresh_token).await);
    assert!(
        !scope_names(&scope).contains(&"admin"),
        "bug: a demoted admin's refresh kept admin: {scope}"
    );
    assert!(
        !listed(&resources, &access_token)
            .await
            .iter()
            .any(|tool| tool == "admin_list_system_agents"),
        "bug: an admin tool listed to a demoted admin"
    );

    resources
        .common
        .repos
        .users
        .set_admin_status(admin.id, true)
        .await
        .unwrap();
    let (_access, _refresh, scope) =
        issued(&refresh(&resources, (&client.0, &client.1), &refresh_token).await);
    assert!(
        !scope_names(&scope).contains(&"admin"),
        "bug: a re-promoted admin regained admin without the screen: {scope}"
    );
}

/// `token` re-signed with an expiry an hour past.
fn expired(resources: &Arc<ServerContext>, token: &str) -> String {
    let mut claims = insecure_decode::<Claims>(token).unwrap().claims;
    claims.iat -= 7200;
    claims.exp = Utc::now().timestamp() - 3600;
    let key = resources.auth.jwks_manager.get_active_key().unwrap();
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(key.kid.clone());
    encode(&header, &claims, &key.encoding_key().unwrap()).unwrap()
}

#[tokio::test]
async fn validate_and_refresh_refuses_an_admin_grant_and_rotates_nothing() {
    let resources = resources().await;
    let client = register(&resources).await;
    let (_admin, session) = member(&resources, true).await;
    let (access_token, refresh_token, _scope) = connect(
        &resources,
        (&client.0, &client.1),
        &session,
        &mcp_resource(&resources),
        true,
    )
    .await;

    // A leaked refresh token and any expired token of the admin's: this
    // route authenticates no client, so it mints nothing for admin (M3).
    let response = AxumTestRequest::post("/oauth2/validate-and-refresh")
        .header(
            "authorization",
            &format!("Bearer {}", expired(&resources, &access_token)),
        )
        .json(&json!({ "refresh_token": refresh_token }))
        .send(oauth2_routes(&resources))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let verdict: Value = response.json();
    assert_eq!(verdict["status"], "invalid", "{verdict}");
    assert_eq!(
        verdict["reason"], ADMIN_REFRESHES_AT_TOKEN_ENDPOINT,
        "{verdict}"
    );
    assert!(
        verdict["access_token"].is_null() && verdict["refresh_token"].is_null(),
        "bug: validate-and-refresh minted for an admin grant: {verdict}"
    );

    // Nothing was rotated: the client refreshes where it authenticates.
    let (_access, _refresh, scope) =
        issued(&refresh(&resources, (&client.0, &client.1), &refresh_token).await);
    assert!(scope_names(&scope).contains(&"admin"), "{scope}");
}

#[tokio::test]
async fn registration_and_client_credentials_still_refuse_admin() {
    let resources = resources().await;
    let refused = ClientRegistrationManager::new(resources.common.repos.oauth2_server.clone())
        .register_client(
            ClientRegistrationRequest {
                redirect_uris: vec![REDIRECT.to_owned()],
                client_name: Some("Asks for admin".to_owned()),
                client_uri: None,
                grant_types: None,
                response_types: None,
                scope: Some("fitness:read admin".to_owned()),
            },
            MAX_PENDING_REGISTRATIONS,
        )
        .await;
    assert!(
        refused.is_err(),
        "bug: a registration asking for admin was accepted"
    );

    let client = register(&resources).await;
    let response = token_request(
        &resources,
        &[
            ("grant_type", "client_credentials"),
            ("client_id", client.0.as_str()),
            ("client_secret", client.1.as_str()),
            ("scope", "admin"),
        ],
    )
    .await;
    assert_eq!(response.status(), 400, "{}", response.body_text());
    let error: Value = serde_json::from_str(&response.body_text()).unwrap();
    assert!(
        error["error"] == "invalid_scope" || error["error"] == "unauthorized_client",
        "bug: client credentials never get admin: {error}"
    );
}
