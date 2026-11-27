// ABOUTME: The consent grant an MCP connection rides on is named on its code, its refresh family and every access token
// ABOUTME: A re-consent to the same details keeps the grant's id, to other details or an expired row replaces it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `client_grant_id`, the `oauth_client_grants` row an authorization was
//! approved under, travels from the consent to the code, from the code to
//! the refresh token and its successors, and into every access token as a
//! claim. These tests drive the real routes and read each carrier.

use crate::common;
use crate::helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use crate::helpers::consent_form::{authorize_query, hidden_field};

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::Request;
use base64::{engine::general_purpose, Engine as _};
use dravr_tronc::dates::{Elapsed, Timestamp};
use jsonwebtoken::dangerous::insecure_decode;
use pierre_auth::auth::{Claims, DelegatedGrant, TokenLineage};
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::models::ClientRegistrationRequest;
use pierre_auth::security::cookies::auth_cookie_name;
use pierre_config::environment::ServerConfig;
use pierre_contremaitre::messaging_strings::format_template;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::html::escape_html_attribute;
use pierre_core::models::{GrantActor, OAuthClientGrant, User};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use secrecy::ExposeSecret;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::time::sleep;
use tower::ServiceExt;
use url::Url;
use uuid::Uuid;

pub(crate) const REDIRECT: &str = "https://grants.example.test/callback";
const VERIFIER: &str = "grant-details-pkce-verifier-0123456789-abcdefghijklmnopqrst";

/// The server's own HTTP composition: authorize, consent, token and `/mcp`.
pub(crate) fn app(resources: &Arc<ServerContext>) -> axum::Router {
    ProviderToolRouter::build_http_app(resources)
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_544))))
}

/// A registered confidential client.
pub(crate) struct Client {
    pub(crate) id: String,
    pub(crate) secret: String,
}

/// An athlete signed in to the web app, and the tenant their grants live in.
pub(crate) struct World {
    pub(crate) resources: Arc<ServerContext>,
    pub(crate) user: User,
    pub(crate) tenant_id: String,
    pub(crate) cookie: String,
}

pub(crate) async fn world() -> World {
    common::init_server_config();
    common::init_test_http_clients();
    world_on(common::create_test_server_resources().await.unwrap()).await
}

/// [`world`] on a server built from `config`.
pub(crate) async fn world_with(config: ServerConfig) -> World {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources_with_config(config)
        .await
        .unwrap();
    world_on(resources).await
}

/// An athlete with a web session on `resources`.
async fn world_on(resources: Arc<ServerContext>) -> World {
    let email = format!("grant_details_{}@example.test", Uuid::new_v4());
    let (user, session) = common::create_test_tenant(&resources, &email)
        .await
        .unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .first_for_user(user.id)
        .await
        .unwrap()
        .expect("the athlete has a tenant")
        .to_string();
    World {
        resources,
        user,
        tenant_id,
        cookie: format!("{}={session}", auth_cookie_name()),
    }
}

pub(crate) async fn register(world: &World) -> Client {
    let registered =
        ClientRegistrationManager::new(world.resources.common.repos.oauth2_server.clone())
            .register_client(
                ClientRegistrationRequest {
                    redirect_uris: vec![REDIRECT.to_owned()],
                    client_name: Some("Grant details client".to_owned()),
                    client_uri: None,
                    grant_types: None,
                    response_types: None,
                    scope: None,
                },
                MAX_PENDING_REGISTRATIONS,
            )
            .await
            .unwrap();
    Client {
        id: registered.client_id,
        secret: registered.client_secret.expose_secret().to_owned(),
    }
}

pub(crate) fn challenge() -> String {
    general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()))
}

/// A fresh `state` for each authorization: a spent one is refused.
pub(crate) fn fresh_state() -> String {
    format!("state-{}", Uuid::new_v4().simple())
}

pub(crate) fn code_of(response: &AxumTestResponse) -> String {
    let location = response
        .header("location")
        .unwrap_or_else(|| panic!("a redirect: {}", response.body_text()));
    assert!(
        location.starts_with(REDIRECT),
        "sent to the client: {location}"
    );
    Url::parse(location)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map_or_else(
            || panic!("the redirect carries a code: {location}"),
            |(_, value)| value.into_owned(),
        )
}

/// The query string of a fresh authorization request of `client` with
/// `extra` parameters.
pub(crate) fn request_query(client: &Client, state: &str, extra: &[(&str, &str)]) -> String {
    let challenge = challenge();
    let mut pairs = vec![
        ("response_type", "code"),
        ("client_id", client.id.as_str()),
        ("redirect_uri", REDIRECT),
        ("state", state),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ];
    pairs.extend_from_slice(extra);
    authorize_query(&pairs)
}

/// `GET /oauth2/authorize` for `query`, signed in when `signed_in`.
pub(crate) async fn authorize_query_as(
    world: &World,
    query: &str,
    signed_in: bool,
) -> AxumTestResponse {
    let mut request = AxumTestRequest::get(&format!("/oauth2/authorize?{query}"));
    if signed_in {
        request = request.header("cookie", &world.cookie);
    }
    request.send(app(&world.resources)).await
}

/// The athlete approves the screen a fresh request of `client` with `extra`
/// parameters shows, choosing `grant` on the picker when given, with a form
/// token minted for them and the review digest the screen carries. A screen
/// skipped under a live grant is approved again under that grant's digest.
pub(crate) async fn approve_with(
    world: &World,
    client: &Client,
    extra: &[(&str, &str)],
    grant: Option<&str>,
) -> AxumTestResponse {
    let state = fresh_state();
    let screen = authorize_query_as(world, &request_query(client, &state, extra), true).await;
    // The field the screen carries the chosen digest in: a choice's own on
    // the picker (Everything when none is named), else `review_digest`.
    let wanted = grant.map_or_else(
        || {
            vec![
                "review_digest".to_owned(),
                "review_digest_everything".to_owned(),
            ]
        },
        |grant| vec![format!("review_digest_{grant}")],
    );
    let (digest_fields, digest) = if screen.status() == 200 {
        let page = screen.body_text();
        wanted
            .iter()
            .find_map(|field| {
                hidden_field(&page, field).map(|digest| (vec![field.clone()], digest))
            })
            .unwrap_or_else(|| panic!("the screen carries one of {wanted:?}: {page}"))
    } else {
        let digest = live_grants(world)
            .await
            .into_iter()
            .find(|held| held.client_id == client.id && held.is_live_at(Timestamp::now()))
            .and_then(|held| held.consent_version)
            .expect("a skipped screen was skipped under a live grant");
        (wanted, digest)
    };
    let token = world
        .resources
        .auth
        .csrf_manager
        .generate_token(world.user.id)
        .unwrap();
    let challenge = challenge();
    // A skipped screen spent its state on the code it minted.
    let state = fresh_state();
    let mut form: Vec<(&str, &str)> = vec![
        ("response_type", "code"),
        ("client_id", client.id.as_str()),
        ("redirect_uri", REDIRECT),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("decision", "approve"),
        ("csrf_token", token.as_str()),
    ];
    for field in &digest_fields {
        form.push((field.as_str(), digest.as_str()));
    }
    if let Some(grant) = grant {
        form.push(("grant", grant));
    }
    form.extend_from_slice(extra);
    AxumTestRequest::post("/oauth2/consent")
        .header("cookie", &world.cookie)
        .form(&form)
        .send(app(&world.resources))
        .await
}

/// The athlete approves `client` on the consent form: the code.
pub(crate) async fn approve(world: &World, client: &Client) -> String {
    let approved = approve_with(world, client, &[], None).await;
    assert_eq!(approved.status(), 303, "{}", approved.body_text());
    code_of(&approved)
}

/// A new authorization of a client the athlete approved: the screen is
/// skipped and the code comes straight back.
async fn authorize_remembered(world: &World, client: &Client) -> String {
    let uri = format!(
        "/oauth2/authorize?response_type=code&client_id={}&redirect_uri={}&state={}&code_challenge={}&code_challenge_method=S256",
        urlencoding::encode(&client.id),
        urlencoding::encode(REDIRECT),
        fresh_state(),
        challenge(),
    );
    let response = AxumTestRequest::get(&uri)
        .header("cookie", &world.cookie)
        .send(app(&world.resources))
        .await;
    assert_eq!(response.status(), 303, "a remembered approval asks nothing");
    code_of(&response)
}

/// The consent grant a code was minted under, read without spending it.
async fn grant_of_code(world: &World, client: &Client, code: &str) -> Option<String> {
    world
        .resources
        .common
        .repos
        .oauth2_server
        .peek_auth_code(code, &client.id, REDIRECT, Timestamp::now())
        .await
        .unwrap()
        .expect("a live code")
        .client_grant_id
}

/// The athlete's live grants.
pub(crate) async fn live_grants(world: &World) -> Vec<OAuthClientGrant> {
    world
        .resources
        .common
        .repos
        .oauth2_server
        .list_client_grants(&world.user.id.to_string(), &world.tenant_id)
        .await
        .unwrap()
}

/// `POST /oauth2/token`.
pub(crate) async fn token(world: &World, form: &Value) -> AxumTestResponse {
    AxumTestRequest::post("/oauth2/token")
        .form(form)
        .send(app(&world.resources))
        .await
}

pub(crate) async fn exchange(world: &World, client: &Client, code: &str) -> AxumTestResponse {
    token(
        world,
        &json!({
            "grant_type": "authorization_code",
            "code": code,
            "redirect_uri": REDIRECT,
            "client_id": client.id,
            "client_secret": client.secret,
            "code_verifier": VERIFIER,
        }),
    )
    .await
}

pub(crate) async fn refresh(
    world: &World,
    client: &Client,
    refresh_token: &str,
) -> AxumTestResponse {
    token(
        world,
        &json!({
            "grant_type": "refresh_token",
            "client_id": client.id,
            "client_secret": client.secret,
            "refresh_token": refresh_token,
        }),
    )
    .await
}

/// A token response's access and refresh tokens.
pub(crate) fn tokens(response: &AxumTestResponse) -> (String, String) {
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let body: Value = serde_json::from_str(&response.body_text()).unwrap();
    (
        body["access_token"].as_str().unwrap().to_owned(),
        body["refresh_token"].as_str().unwrap().to_owned(),
    )
}

fn assert_invalid_grant(response: &AxumTestResponse) {
    assert_eq!(response.status(), 400, "{}", response.body_text());
    let body: Value = serde_json::from_str(&response.body_text()).unwrap();
    assert_eq!(body["error"], "invalid_grant", "{body}");
}

pub(crate) fn claimed_grant(access_token: &str) -> Option<String> {
    insecure_decode::<Claims>(access_token)
        .unwrap()
        .claims
        .client_grant_id
}

async fn stored_grant_of_refresh(world: &World, refresh_token: &str) -> Option<String> {
    world
        .resources
        .common
        .repos
        .oauth2_server
        .get_refresh_token_by_value(refresh_token)
        .await
        .unwrap()
        .expect("a stored refresh token")
        .client_grant_id
}

/// `tools/call` on `/mcp` under `token`: the status it answers.
async fn mcp_status(world: &World, token: &str) -> u16 {
    let request = Request::post("/mcp")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "get_connection_status", "arguments": {} }
            })
            .to_string(),
        ))
        .unwrap();
    ProviderToolRouter::build_http_app(&world.resources)
        .oneshot(request)
        .await
        .unwrap()
        .status()
        .as_u16()
}

/// Drop the cached standing, as a change to the grant made through the
/// routes does.
pub(crate) async fn forget_standing(world: &World, client: &Client) {
    world
        .resources
        .account_service()
        .grant_standings()
        .forget(world.user.id, &world.tenant_id, &client.id)
        .await;
}

/// Move a grant's expiry into the past. Nothing in this phase writes an
/// expiry through a route yet, so the test writes it as the row would hold it.
async fn expire(world: &World, grant_id: &str) {
    let past = Timestamp::now() - Elapsed::minutes(1);
    let sql = "UPDATE oauth_client_grants SET expires_at = $1 WHERE id = $2";
    match world.resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(sql)
                .bind(past)
                .bind(grant_id)
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(sql)
                .bind(past)
                .bind(grant_id)
                .execute(db.pool())
                .await
                .unwrap();
        }
    }
}

/// A re-consent keeps the grant the athlete already holds, and the code it
/// mints names that grant, not the id the approval offered.
#[tokio::test]
async fn a_re_consent_names_the_grant_already_held() {
    let world = world().await;
    let client = register(&world).await;

    let first = approve(&world, &client).await;
    let grants = live_grants(&world).await;
    assert_eq!(grants.len(), 1);
    let held = grants[0].id.clone();
    assert_eq!(
        grant_of_code(&world, &client, &first).await,
        Some(held.clone())
    );

    let again = approve(&world, &client).await;
    assert_eq!(
        live_grants(&world).await.len(),
        1,
        "a re-consent is not a second grant"
    );
    assert_eq!(
        grant_of_code(&world, &client, &again).await,
        Some(held.clone()),
        "bug: the code of a re-consent names an id no row holds"
    );

    let remembered = authorize_remembered(&world, &client).await;
    assert_eq!(
        grant_of_code(&world, &client, &remembered).await,
        Some(held),
        "the skipped screen's code names the grant it was skipped under"
    );
}

/// Code, refresh token, every successor and every access token name the
/// consent grant; the refresh family keeps its own `grant_id` claim.
#[tokio::test]
async fn a_code_token_refresh_chain_carries_the_consent_grant() {
    let world = world().await;
    let client = register(&world).await;
    let code = approve(&world, &client).await;
    let held = live_grants(&world).await[0].id.clone();

    let (access, refresh_token) = tokens(&exchange(&world, &client, &code).await);
    assert_eq!(claimed_grant(&access), Some(held.clone()));
    assert_eq!(
        stored_grant_of_refresh(&world, &refresh_token).await,
        Some(held.clone())
    );
    let family = insecure_decode::<Claims>(&access).unwrap().claims.grant_id;
    assert!(family.is_some());
    assert_ne!(
        family,
        Some(held.clone()),
        "the family is not the consent grant"
    );

    let (rotated_access, rotated_refresh) = tokens(&refresh(&world, &client, &refresh_token).await);
    assert_eq!(claimed_grant(&rotated_access), Some(held.clone()));
    assert_eq!(
        stored_grant_of_refresh(&world, &rotated_refresh).await,
        Some(held)
    );
    assert_eq!(
        insecure_decode::<Claims>(&rotated_access)
            .unwrap()
            .claims
            .grant_id,
        family,
        "a refresh keeps the family"
    );
    assert_eq!(mcp_status(&world, &rotated_access).await, 200);
}

/// A grant past its expiry mints no token from a code or a refresh token,
/// and its access token stops at `/mcp`.
#[tokio::test]
async fn an_expired_grant_is_refused_at_the_token_endpoint_and_at_mcp() {
    let world = world().await;
    let client = register(&world).await;
    let code = approve(&world, &client).await;
    let held = live_grants(&world).await[0].id.clone();
    let (access, refresh_token) = tokens(&exchange(&world, &client, &code).await);
    let pending = authorize_remembered(&world, &client).await;
    assert_eq!(
        mcp_status(&world, &access).await,
        200,
        "control: live grant"
    );

    expire(&world, &held).await;
    forget_standing(&world, &client).await;

    assert_invalid_grant(&refresh(&world, &client, &refresh_token).await);
    assert_invalid_grant(&exchange(&world, &client, &pending).await);
    assert_eq!(
        mcp_status(&world, &access).await,
        401,
        "an access token of an expired grant is refused"
    );
    assert_eq!(
        live_grants(&world).await.len(),
        1,
        "expiry revokes nothing: the row stays the athlete's to see"
    );
}

/// A token naming a revoked grant is refused even while the client holds a
/// newer grant the issued-at rule alone would admit it under.
#[tokio::test]
async fn a_token_naming_a_revoked_grant_is_refused_at_mcp() {
    let world = world().await;
    let client = register(&world).await;
    approve(&world, &client).await;
    let revoked = live_grants(&world).await[0].id.clone();
    let repository = &world.resources.common.repos.oauth2_server;
    assert!(repository
        .revoke_client_grant(
            &revoked,
            &world.user.id.to_string(),
            &world.tenant_id,
            GrantActor::User
        )
        .await
        .unwrap());
    // Past the second of the revocation, which `iat` cannot tell apart.
    sleep(StdDuration::from_millis(1_100)).await;
    approve(&world, &client).await;
    let live = live_grants(&world).await[0].id.clone();
    assert_ne!(live, revoked);
    forget_standing(&world, &client).await;

    let scopes = vec!["fitness:read".to_owned(), "profile:read".to_owned()];
    let mint = |client_grant_id: &str| {
        world
            .resources
            .auth
            .auth_manager
            .generate_oauth_access_token_in_grant(
                &world.resources.auth.jwks_manager,
                &DelegatedGrant {
                    client_id: &client.id,
                    scopes: &scopes,
                    audience: None,
                },
                &TokenLineage {
                    family_id: "family",
                    client_grant_id: Some(client_grant_id),
                    dpop_jkt: None,
                    agent_write: false,
                },
                &world.user.id,
                Some(world.tenant_id.clone()),
            )
            .unwrap()
    };
    assert_eq!(
        mcp_status(&world, &mint(&revoked)).await,
        401,
        "bug: a token naming a revoked grant was admitted under the client's newer one"
    );
    assert_eq!(
        mcp_status(&world, &mint(&live)).await,
        200,
        "the live grant's"
    );
}

/// A token minted before grants were named carries no claim and is admitted
/// under a plain grant as before.
#[tokio::test]
async fn a_token_without_the_claim_still_works_under_a_plain_grant() {
    let world = world().await;
    let client = register(&world).await;
    approve(&world, &client).await;
    let grants = live_grants(&world).await;
    assert!(grants[0].authorization_details.is_none(), "a plain grant");
    assert!(
        grants[0].expires_at.is_none(),
        "a plain grant never expires"
    );

    let scopes = vec!["fitness:read".to_owned(), "profile:read".to_owned()];
    let legacy = world
        .resources
        .auth
        .auth_manager
        .generate_oauth_access_token(
            &world.resources.auth.jwks_manager,
            &DelegatedGrant {
                client_id: &client.id,
                scopes: &scopes,
                audience: None,
            },
            &world.user.id,
            Some(world.tenant_id.clone()),
        )
        .unwrap();
    assert_eq!(claimed_grant(&legacy), None);
    assert_eq!(mcp_status(&world, &legacy).await, 200);
}

/// A re-consent to other details is a new grant: the live row is revoked with
/// its refresh family, and the new code names the new row.
#[tokio::test]
async fn a_re_consent_with_other_details_replaces_the_grant() {
    let world = world().await;
    let client = register(&world).await;
    let code = approve(&world, &client).await;
    let old = live_grants(&world).await[0].id.clone();
    let (old_access, old_refresh) = tokens(&exchange(&world, &client, &code).await);

    // A request with details the plain grant does not hold shows the screen,
    // and its approval is a new grant.
    let details = training_coach_details(&world, "");
    let screen = authorize_query_as(
        &world,
        &request_query(
            &client,
            &fresh_state(),
            &[("authorization_details", &details)],
        ),
        true,
    )
    .await;
    assert_eq!(screen.status(), 200, "{}", screen.body_text());
    let approved = approve_with(
        &world,
        &client,
        &[("authorization_details", &details)],
        None,
    )
    .await;
    assert_eq!(approved.status(), 303, "{}", approved.body_text());
    let again = code_of(&approved);
    let live = live_grants(&world).await;
    assert_eq!(live.len(), 1, "one live grant: {live:?}");
    let new = live[0].id.clone();
    assert_ne!(
        new, old,
        "bug: other details were written onto the live row"
    );
    assert!(live[0]
        .authorization_details
        .as_deref()
        .is_some_and(|document| document.contains("\"training_coach\"")));
    let revoked = world
        .resources
        .common
        .repos
        .oauth2_server
        .get_client_grant(&old, &world.user.id.to_string(), &world.tenant_id)
        .await
        .unwrap()
        .expect("the replaced row stays");
    assert!(revoked.revoked_at.is_some(), "the replaced row is revoked");
    assert_eq!(grant_of_code(&world, &client, &again).await, Some(new));

    assert_invalid_grant(&refresh(&world, &client, &old_refresh).await);
    assert_eq!(
        mcp_status(&world, &old_access).await,
        401,
        "the old grant's access token stops with it"
    );
    let (new_access, _) = tokens(&exchange(&world, &client, &again).await);
    assert_eq!(mcp_status(&world, &new_access).await, 200);
}

/// The same details again keep the row: only the consent text moves.
#[tokio::test]
async fn a_re_consent_with_the_same_details_keeps_the_grant() {
    let world = world().await;
    let client = register(&world).await;
    let code = approve(&world, &client).await;
    let held = live_grants(&world).await[0].id.clone();
    let (_, refresh_token) = tokens(&exchange(&world, &client, &code).await);

    approve(&world, &client).await;
    assert_eq!(live_grants(&world).await[0].id, held);
    let (access, _) = tokens(&refresh(&world, &client, &refresh_token).await);
    assert_eq!(
        mcp_status(&world, &access).await,
        200,
        "nothing was revoked"
    );
}

/// A grant past its expiry can be consented to again: the screen is shown,
/// and the approval replaces the expired row with a live one that mints.
#[tokio::test]
async fn an_expired_grant_can_be_consented_to_again() {
    let world = world().await;
    let client = register(&world).await;
    approve(&world, &client).await;
    let expired = live_grants(&world).await[0].id.clone();
    expire(&world, &expired).await;
    forget_standing(&world, &client).await;

    let uri = format!(
        "/oauth2/authorize?response_type=code&client_id={}&redirect_uri={}&state={}&code_challenge={}&code_challenge_method=S256",
        urlencoding::encode(&client.id),
        urlencoding::encode(REDIRECT),
        fresh_state(),
        challenge(),
    );
    let screen = AxumTestRequest::get(&uri)
        .header("cookie", &world.cookie)
        .send(app(&world.resources))
        .await;
    assert_eq!(
        screen.status(),
        200,
        "an expired grant asks again: {}",
        screen.body_text()
    );

    let code = approve(&world, &client).await;
    let live = live_grants(&world).await;
    assert_eq!(live.len(), 1);
    assert_ne!(
        live[0].id, expired,
        "bug: the approval was written onto the expired row"
    );
    assert!(live[0].is_live_at(Timestamp::now()));
    assert_eq!(
        grant_of_code(&world, &client, &code).await,
        Some(live[0].id.clone())
    );
    let (access, _) = tokens(&exchange(&world, &client, &code).await);
    assert_eq!(mcp_status(&world, &access).await, 200);
}

// ============================================================================
// authorization_details (RFC 9396): refusals, the picker, the digest, the echo
// ============================================================================

const DATA_ACCESS: &str = "https://pdpp.dev/data-access";
const HEALTH_CONSTRAINTS: &str = "https://dravr.ai/pdpp/health-profile/v0/health-constraints";

/// The MCP resource this server mints for, which a detail's `source.id` names.
pub(crate) fn mcp_resource(world: &World) -> String {
    world.resources.common.config.oauth2_server.mcp_resources()[0].to_owned()
}

/// A data-access detail naming `training_coach` on the MCP resource, with
/// `extra` members.
pub(crate) fn access_detail(world: &World, extra: &Value) -> Value {
    let mut detail = json!({
        "type": DATA_ACCESS,
        "source": {"id": mcp_resource(world)},
        "selection_preset": "training_coach",
    });
    if let (Value::Object(detail), Value::Object(extra)) = (&mut detail, extra) {
        detail.extend(extra.clone());
    }
    detail
}

/// A health-constraints detail applying to the first detail, with `extra`.
pub(crate) fn constraints_detail(extra: &Value) -> Value {
    let mut detail = json!({"type": HEALTH_CONSTRAINTS, "applies_to": 0, "critical": true});
    if let (Value::Object(detail), Value::Object(extra)) = (&mut detail, extra) {
        detail.extend(extra.clone());
    }
    detail
}

/// A training-coach request narrowed to 30 calls an hour, with `purpose` as
/// the client's own words when not empty.
fn training_coach_details(world: &World, purpose: &str) -> String {
    let extra = if purpose.is_empty() {
        json!({})
    } else {
        json!({"purpose_description": purpose, "client_claims": {"commitments": ["Used only for your plan"]}})
    };
    json!([
        access_detail(world, &extra),
        constraints_detail(&json!({"budgets": {"requests_per_hour": 30}}))
    ])
    .to_string()
}

/// The error and `state` a refusal redirected to the client carries.
fn refusal_to_client(response: &AxumTestResponse) -> (String, String) {
    assert_eq!(response.status(), 303, "{}", response.body_text());
    let location = response.header("location").unwrap();
    assert!(
        location.starts_with(REDIRECT),
        "refused to the client, never to the login page: {location}"
    );
    let url = Url::parse(location).unwrap();
    let param = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map_or_else(String::new, |(_, value)| value.into_owned())
    };
    (param("error"), param("state"))
}

#[tokio::test]
async fn every_refused_detail_reaches_the_client_before_sign_in() {
    let world = world().await;
    let client = register(&world).await;
    let access = |extra: Value| access_detail(&world, &extra);
    let constraints = |extra: Value| constraints_detail(&extra);
    let rows: Vec<(&str, Value)> = vec![
        (
            "an unknown type",
            json!([{"type": "https://example.test/other"}]),
        ),
        ("no data-access detail", json!([constraints(json!({}))])),
        (
            "two data-access details",
            json!([access(json!({})), access(json!({}))]),
        ),
        (
            "critical missing",
            json!([access(json!({})), {"type": HEALTH_CONSTRAINTS, "applies_to": 0}]),
        ),
        (
            "a bad applies_to",
            json!([access(json!({})), constraints(json!({"applies_to": 3}))]),
        ),
        (
            "an unknown preset",
            json!([access(json!({"selection_preset": "clinician_summary"}))]),
        ),
        (
            "explicit streams",
            json!([access(
                json!({"streams": [{"name": "health.vault.sleep.summary"}]})
            )]),
        ),
        (
            "another resource",
            json!([access(
                json!({"source": {"id": "https://other.example.test/mcp"}})
            )]),
        ),
        (
            "single_use",
            json!([access(json!({"access_mode": "single_use"}))]),
        ),
        (
            "time_range",
            json!([access(
                json!({"time_range": {"since": "2026-01-01T00:00:00Z"}})
            )]),
        ),
        (
            "max_tier T2",
            json!([access(json!({})), constraints(json!({"max_tier": "T2"}))]),
        ),
        (
            "max_tier T3",
            json!([access(json!({})), constraints(json!({"max_tier": "T3"}))]),
        ),
        (
            "raw_streams",
            json!([
                access(json!({})),
                constraints(json!({"raw_streams": ["health.vault.workout.raw"]}))
            ]),
        ),
        (
            "a calendar lifetime",
            json!([access(json!({})), constraints(json!({"lifetime": "P1M"}))]),
        ),
        (
            "total_rows",
            json!([
                access(json!({})),
                constraints(json!({"budgets": {"total_rows": 9}}))
            ]),
        ),
        (
            "code_filter",
            json!([
                access(json!({})),
                constraints(
                    json!({"code_filter": {"system": "http://loinc.org", "codes": ["4548-4"]}})
                )
            ]),
        ),
        (
            "a step-up",
            json!([
                access(json!({})),
                constraints(json!({"step_up": {"required": true}}))
            ]),
        ),
        (
            "a Host Profile",
            json!([
                access(json!({})),
                constraints(json!({"host_profile_required": "t2"}))
            ]),
        ),
        (
            "egress",
            json!([
                access(json!({})),
                constraints(json!({"egress": "typed_only"}))
            ]),
        ),
        (
            "an AS-set member",
            json!([
                access(json!({})),
                constraints(json!({"host_profile": {"basis": "x"}}))
            ]),
        ),
        (
            "standing_policy_ref",
            json!([access(json!({"standing_policy_ref": "p"}))]),
        ),
        (
            "an unknown member",
            json!([access(json!({"locations": ["x"]}))]),
        ),
        ("not a JSON array", json!({"type": DATA_ACCESS})),
    ];
    for (row, details) in rows {
        let state = fresh_state();
        let query = request_query(
            &client,
            &state,
            &[("authorization_details", &details.to_string())],
        );
        let (error, echoed) = refusal_to_client(&authorize_query_as(&world, &query, false).await);
        assert_eq!(error, "invalid_authorization_details", "row: {row}");
        assert_eq!(
            echoed, state,
            "row {row}: the refusal carries the client's state"
        );
    }
    assert!(live_grants(&world).await.is_empty(), "nothing was granted");
}

/// The pinned catalogue's `key` in the athlete's language, its slots filled
/// with `slots`, as the consent screen escapes it.
pub(crate) fn shown(world: &World, key: &str, slots: &[&str]) -> String {
    let line = world
        .resources
        .mcp
        .messaging_strings_registry
        .get(key, &world.user.locale);
    assert!(!line.is_empty(), "the pinned catalogue carries {key}");
    format_template(&escape_html_attribute(&line), slots)
}

#[tokio::test]
async fn details_are_advertised_and_shown_from_the_pinned_catalogue() {
    let world = world().await;
    let body = AxumTestRequest::get("/.well-known/oauth-authorization-server")
        .send(app(&world.resources))
        .await
        .body_text();
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["authorization_details_types_supported"],
        json!([DATA_ACCESS, HEALTH_CONSTRAINTS])
    );

    // A request's details are shown before they are granted, never refused
    // for lack of lines.
    let client = register(&world).await;
    let details = training_coach_details(&world, "");
    let query = request_query(
        &client,
        &fresh_state(),
        &[("authorization_details", &details)],
    );
    let screen = authorize_query_as(&world, &query, true).await;
    assert_eq!(screen.status(), 200, "{}", screen.body_text());
    let page = screen.body_text();
    assert!(
        page.contains(&shown(&world, "oauth.consent.grantTrainingCoach", &[])),
        "{page}"
    );
    assert!(hidden_field(&page, "review_digest").is_some());
}

/// A preset grant's screen lists the data it shares, so it reads the
/// preset's disclosure and scope lines, which name no injuries and no
/// activity places, in place of the lines every grant reads. The plain
/// screen never reads them.
#[tokio::test]
async fn a_preset_screen_reads_its_preset_lines_and_the_plain_screen_never_does() {
    let world = world().await;
    let client = register(&world).await;
    let details = training_coach_details(&world, "");
    let details_page = || async {
        let query = request_query(
            &client,
            &fresh_state(),
            &[("authorization_details", &details)],
        );
        let screen = authorize_query_as(&world, &query, true).await;
        assert_eq!(screen.status(), 200, "{}", screen.body_text());
        screen.body_text()
    };
    let generic_scope = shown(&world, "oauth.consent.scope.fitnessRead", &[]);

    // The pinned catalogue's own preset line, in the athlete's language.
    let pinned = details_page().await;
    let preset_scope = shown(&world, "oauth.consent.scope.fitnessReadPreset", &[]);
    assert!(
        pinned.contains(&preset_scope) && !pinned.contains(&generic_scope),
        "bug: a preset screen reads the pinned preset scope line: {pinned}"
    );
    let registry = &world.resources.mcp.messaging_strings_registry;
    for key in ["oauth.consent.healthPreset", "oauth.consent.expressPreset"] {
        let line = registry.get(key, &world.user.locale);
        assert!(
            !line.is_empty(),
            "fixture: the pinned catalogue carries {key}"
        );
        // The text between the slots, which the page shows as written.
        for part in line.split(['{', '}']).filter(|part| part.len() > 3) {
            let escaped = escape_html_attribute(part.trim());
            assert!(
                pinned.contains(&escaped),
                "bug: a preset screen reads {key}'s {escaped:?}: {pinned}"
            );
        }
    }

    let locale = world.user.locale.as_str();
    for (key, line) in [
        (
            "oauth.consent.healthPreset",
            "PRESET HEALTH shared with {0}.",
        ),
        (
            "oauth.consent.expressPreset",
            "PRESET EXPRESS: by choosing {0} you consent to {1}.",
        ),
        (
            "oauth.consent.scope.fitnessReadPreset",
            "PRESET SCOPE read your activities",
        ),
    ] {
        registry.update(key, locale, line.to_owned(), "proposed".to_owned());
    }

    let after = details_page().await;
    for preset in ["PRESET HEALTH", "PRESET EXPRESS", "PRESET SCOPE"] {
        assert!(after.contains(preset), "bug: {preset} is shown: {after}");
    }
    assert!(
        !after.contains(&generic_scope),
        "bug: the preset screen still reads the generic scope line: {after}"
    );
    let plain = authorize_query_as(&world, &request_query(&client, &fresh_state(), &[]), true)
        .await
        .body_text();
    assert!(
        !plain.contains("PRESET") && plain.contains(&generic_scope),
        "bug: the plain screen reads a preset line: {plain}"
    );
}

#[tokio::test]
async fn the_picker_offers_everything_preselected_then_each_preset() {
    let world = world().await;
    let client = register(&world).await;
    let screen =
        authorize_query_as(&world, &request_query(&client, &fresh_state(), &[]), true).await;
    assert_eq!(screen.status(), 200);
    let page = screen.body_text();
    assert!(
        page.contains(r#"name="grant" value="everything" checked"#),
        "Everything, as today, preselected: {page}"
    );
    for choice in ["training_coach", "sleep_coach"] {
        assert!(
            page.contains(&format!(r#"name="grant" value="{choice}">"#)),
            "{choice}: {page}"
        );
    }
    for choice in ["everything", "training_coach", "sleep_coach"] {
        assert!(
            hidden_field(&page, &format!("review_digest_{choice}")).is_some(),
            "each choice carries its digest"
        );
    }
    // The choices sit in the scope list's neutral frame, each radio on one
    // line with its label and its hint under it, never the centred fine print.
    assert!(
        page.contains(r#"<fieldset class="scopes grant-choice"><legend class="scopes-title">"#),
        "{page}"
    );
    assert_eq!(
        page.matches(r#"<div class="grant-option"><label class="check"><input type="radio""#)
            .count(),
        3,
        "{page}"
    );
    assert_eq!(
        page.matches(r#"</span></label><p class="grant-hint">"#)
            .count(),
        3,
        "{page}"
    );
    assert!(
        !page[page.find("<fieldset").unwrap()..page.find("</fieldset>").unwrap()]
            .contains("fineprint"),
        "a hint is not centred fine print: {page}"
    );
    for (key, slots) in [
        ("oauth.consent.grantEverything", &[][..]),
        ("oauth.consent.grantTrainingCoach", &[]),
        ("oauth.consent.grantTrainingCoachHint", &["90"]),
        ("oauth.consent.grantSleepCoach", &[]),
        ("oauth.consent.grantSleepCoachHint", &["90"]),
        ("oauth.consent.grantLastsDays", &["30"]),
    ] {
        let line = shown(&world, key, slots);
        assert!(page.contains(&line), "{key} reads {line:?}: {page}");
    }

    // Everything is the plain grant, as today.
    approve(&world, &client).await;
    let plain = live_grants(&world).await;
    assert!(plain[0].authorization_details.is_none() && plain[0].expires_at.is_none());

    // A request naming no details is then answered by that grant, so the
    // picker is shown only while no grant answers it: another client.
    let skipped =
        authorize_query_as(&world, &request_query(&client, &fresh_state(), &[]), true).await;
    assert_eq!(
        skipped.status(),
        303,
        "the plain grant answers a scope-only request"
    );

    // Choosing Sleep coach is a preset grant the token echoes.
    let sleepy = register(&world).await;
    let approved = approve_with(&world, &sleepy, &[], Some("sleep_coach")).await;
    let code = code_of(&approved);
    let sleep_grant = live_grants(&world)
        .await
        .into_iter()
        .find(|grant| grant.client_id == sleepy.id)
        .expect("the sleep grant");
    assert!(sleep_grant
        .authorization_details
        .is_some_and(|document| document.contains("\"sleep_coach\"")));
    let body: Value =
        serde_json::from_str(&exchange(&world, &sleepy, &code).await.body_text()).unwrap();
    assert_eq!(
        body["authorization_details"][0]["selection_preset"],
        "sleep_coach"
    );
    assert_eq!(
        body["authorization_details"][1]["representation"],
        "summary"
    );
}

#[tokio::test]
async fn a_training_coach_request_resolves_end_to_end_to_a_token_echoing_its_details() {
    let world = world().await;
    let client = register(&world).await;
    let details = training_coach_details(&world, "Plan <b>your</b> week {{CSRF_TOKEN}}");

    // Pushed (RFC 9126) with its details.
    let challenge = challenge();
    let state = fresh_state();
    let pushed = AxumTestRequest::post("/oauth2/par")
        .form(&[
            ("client_id", client.id.as_str()),
            ("client_secret", client.secret.as_str()),
            ("response_type", "code"),
            ("redirect_uri", REDIRECT),
            ("state", state.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("authorization_details", details.as_str()),
        ])
        .send(app(&world.resources))
        .await;
    assert_eq!(pushed.status(), 201, "{}", pushed.body_text());
    let request_uri = serde_json::from_str::<Value>(&pushed.body_text()).unwrap()["request_uri"]
        .as_str()
        .unwrap()
        .to_owned();
    let query = authorize_query(&[("client_id", &client.id), ("request_uri", &request_uri)]);

    // The screen shows what the details resolved to, the purpose quoted and
    // escaped, and neither the picker nor Admin access.
    let screen = authorize_query_as(&world, &query, true).await;
    assert_eq!(screen.status(), 200, "{}", screen.body_text());
    let page = screen.body_text();
    assert!(
        page.contains(&shown(&world, "oauth.consent.grantTrainingCoach", &[])),
        "{page}"
    );
    assert!(
        page.contains("Plan &lt;b&gt;your&lt;/b&gt; week"),
        "the client's words, escaped: {page}"
    );
    assert!(
        !page.contains("<b>your</b>"),
        "bug: the client's markup reached the page"
    );
    assert!(page.contains("<q>Used only for your plan</q>"), "{page}");
    assert!(
        page.contains(r#"<div class="scopes grant-requested"><p class="grant-label">"#),
        "the details read as information in the neutral frame, not a warning: {page}"
    );
    assert!(
        page.contains("<ul><li><q>Used only for your plan</q></li></ul>"),
        "{page}"
    );
    assert!(
        !page.contains(r#"name="grant""#),
        "no picker for a request's own details"
    );
    assert!(!page.contains(r#"name="admin_access""#));

    let token = world
        .resources
        .auth
        .csrf_manager
        .generate_token(world.user.id)
        .unwrap();
    let digest = hidden_field(&page, "review_digest").unwrap();
    let approved = AxumTestRequest::post("/oauth2/consent")
        .header("cookie", &world.cookie)
        .form(&[
            ("client_id", client.id.as_str()),
            ("request_uri", request_uri.as_str()),
            ("decision", "approve"),
            ("csrf_token", token.as_str()),
            ("review_digest", digest.as_str()),
        ])
        .send(app(&world.resources))
        .await;
    let code = code_of(&approved);

    // The grant stores the resolved document, its window frozen, and expires
    // with the preset's 30 days.
    let granted = live_grants(&world).await;
    assert_eq!(granted.len(), 1);
    let document = granted[0]
        .authorization_details
        .clone()
        .expect("a preset grant");
    assert!(
        document.contains("\"training_coach\"") && document.contains("\"since\""),
        "{document}"
    );
    assert!(
        !document.contains("health.vault"),
        "no stream names: {document}"
    );
    let expires_at = granted[0].expires_at.expect("a preset grant expires");
    let thirty_days = Timestamp::now() + Elapsed::days(30);
    assert!(expires_at <= thirty_days && expires_at > thirty_days - Elapsed::minutes(5));

    // The token response echoes it (RFC 9396 §7), on the exchange and a refresh.
    let exchanged = exchange(&world, &client, &code).await;
    let (access, refresh_token) = tokens(&exchanged);
    let body: Value = serde_json::from_str(&exchanged.body_text()).unwrap();
    let echoed = &body["authorization_details"];
    assert_eq!(echoed[0]["type"], DATA_ACCESS);
    assert_eq!(echoed[0]["selection_preset"], "training_coach");
    assert_eq!(
        echoed[0]["purpose_description"],
        "Plan <b>your</b> week {{CSRF_TOKEN}}"
    );
    assert!(echoed[0]["window"]["since"].is_string());
    assert_eq!(echoed[1]["type"], HEALTH_CONSTRAINTS);
    assert_eq!(echoed[1]["max_tier"], "T1");
    assert_eq!(echoed[1]["representation"], "summary_and_raw");
    assert_eq!(echoed[1]["budgets"]["requests_per_hour"], 30);
    assert_eq!(claimed_grant(&access), Some(granted[0].id.clone()));
    let refreshed = refresh(&world, &client, &refresh_token).await;
    assert_eq!(refreshed.status(), 200, "{}", refreshed.body_text());
    let refreshed: Value = serde_json::from_str(&refreshed.body_text()).unwrap();
    assert_eq!(&refreshed["authorization_details"], echoed);

    // A plain grant's tokens echo nothing.
    let plain_client = register(&world).await;
    let plain_code = approve(&world, &plain_client).await;
    let plain: Value = serde_json::from_str(
        &exchange(&world, &plain_client, &plain_code)
            .await
            .body_text(),
    )
    .unwrap();
    assert!(plain.get("authorization_details").is_none(), "{plain}");
}

/// A request sent in the query keeps its details across the login page.
#[tokio::test]
async fn a_signed_out_details_request_carries_its_details_to_the_login_page() {
    let world = world().await;
    let client = register(&world).await;
    let details = training_coach_details(&world, "");
    let query = request_query(
        &client,
        &fresh_state(),
        &[("authorization_details", &details)],
    );
    let response = authorize_query_as(&world, &query, false).await;
    let location = response.header("location").unwrap_or_default().to_owned();
    assert!(location.starts_with("/oauth2/login?"), "{location}");
    let carried = Url::parse(&format!("https://issuer.test{location}"))
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "authorization_details")
        .map(|(_, value)| value.into_owned());
    assert_eq!(carried.as_deref(), Some(details.as_str()));
}

#[tokio::test]
async fn an_approval_under_a_stale_review_digest_is_refused() {
    let world = world().await;
    let client = register(&world).await;
    let details = training_coach_details(&world, "");
    let state = fresh_state();
    let challenge = challenge();
    let token = world
        .resources
        .auth
        .csrf_manager
        .generate_token(world.user.id)
        .unwrap();
    let screen = authorize_query_as(
        &world,
        &request_query(&client, &state, &[("authorization_details", &details)]),
        true,
    )
    .await;
    let digest = hidden_field(&screen.body_text(), "review_digest").unwrap();
    let plain_screen = authorize_query_as(&world, &request_query(&client, &state, &[]), true).await;
    let plain_digest = hidden_field(&plain_screen.body_text(), "review_digest_everything").unwrap();

    let submit = |digest: String| {
        let world = &world;
        let form: Vec<(String, String)> = [
            ("response_type", "code"),
            ("client_id", client.id.as_str()),
            ("redirect_uri", REDIRECT),
            ("state", state.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("authorization_details", details.as_str()),
            ("decision", "approve"),
            ("csrf_token", token.as_str()),
            ("review_digest", digest.as_str()),
        ]
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
        async move {
            AxumTestRequest::post("/oauth2/consent")
                .header("cookie", &world.cookie)
                .form(&form)
                .send(app(&world.resources))
                .await
        }
    };
    for stale in ["0".repeat(64), plain_digest, String::new()] {
        let refused = submit(stale).await;
        // Shown to the athlete, in their language; nothing reaches the client.
        assert_eq!(refused.status(), 400, "{}", refused.body_text());
        assert!(refused.header("location").is_none());
    }
    assert!(
        live_grants(&world).await.is_empty(),
        "a stale screen grants nothing"
    );

    // The screen's own digest approves.
    let approved = submit(digest).await;
    assert_eq!(approved.status(), 303, "{}", approved.body_text());
    assert_eq!(live_grants(&world).await.len(), 1);
}

#[tokio::test]
async fn a_remembered_preset_grant_answers_its_own_details_and_a_plain_request_only() {
    let world = world().await;
    let client = register(&world).await;
    let details = training_coach_details(&world, "");
    let approved = approve_with(
        &world,
        &client,
        &[("authorization_details", &details)],
        None,
    )
    .await;
    code_of(&approved);
    let held = live_grants(&world).await[0].id.clone();

    // The same details, and a request naming none, skip the screen.
    for extra in [vec![("authorization_details", details.as_str())], vec![]] {
        let skipped = authorize_query_as(
            &world,
            &request_query(&client, &fresh_state(), &extra),
            true,
        )
        .await;
        assert_eq!(skipped.status(), 303, "{}", skipped.body_text());
        assert_eq!(
            grant_of_code(&world, &client, &code_of(&skipped)).await,
            Some(held.clone())
        );
    }

    // Other details ask again.
    let sleep = json!([access_detail(
        &world,
        &json!({"selection_preset": "sleep_coach"})
    )])
    .to_string();
    let asked = authorize_query_as(
        &world,
        &request_query(
            &client,
            &fresh_state(),
            &[("authorization_details", &sleep)],
        ),
        true,
    )
    .await;
    assert_eq!(asked.status(), 200, "other details are asked again");
}

#[tokio::test]
async fn an_expired_preset_grant_is_asked_again_and_replaced() {
    let world = world().await;
    let client = register(&world).await;
    let details = training_coach_details(&world, "");
    let extra = [("authorization_details", details.as_str())];
    let code = code_of(&approve_with(&world, &client, &extra, None).await);
    let (_, refresh_token) = tokens(&exchange(&world, &client, &code).await);
    let expired = live_grants(&world).await[0].id.clone();
    expire(&world, &expired).await;
    forget_standing(&world, &client).await;

    assert_invalid_grant(&refresh(&world, &client, &refresh_token).await);
    let screen = authorize_query_as(
        &world,
        &request_query(&client, &fresh_state(), &extra),
        true,
    )
    .await;
    assert_eq!(screen.status(), 200, "an expired grant asks again");

    let code = code_of(&approve_with(&world, &client, &extra, None).await);
    let live = live_grants(&world).await;
    assert_eq!(live.len(), 1);
    assert_ne!(live[0].id, expired, "the expired row is replaced");
    let (access, _) = tokens(&exchange(&world, &client, &code).await);
    assert_eq!(mcp_status(&world, &access).await, 200);
}
