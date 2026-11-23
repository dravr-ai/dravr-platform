// ABOUTME: An /oauth2/token access token carries the metadata issuer URL; sessions and impersonation keep pierre-mcp-server
// ABOUTME: /mcp, A2A, validate-and-refresh and introspection admit the URL on OAuth tokens and the legacy name a prior binary minted
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The issuer by token kind (RFC 9068 §2.2), over a server whose auth manager
//! is built as the deployed binary builds it: with the OAuth issuer.

use crate::common;
use crate::helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use crate::helpers::consent_form::{authorize_query, consent_page, review_digest};

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use base64::{engine::general_purpose, Engine as _};
use dravr_tronc::dates::Timestamp;
use jsonwebtoken::dangerous::insecure_decode;
use jsonwebtoken::{encode, Algorithm, Header};
use pierre_auth::auth::Claims;
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::models::ClientRegistrationRequest;
use pierre_auth::security::cookies::auth_cookie_name;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::constants::service_names::PIERRE_MCP_SERVER;
use pierre_core::models::User;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_a2a::{A2ARoutes, A2ARoutesState};
use pierre_tool_runtime::runtime::ToolRuntime;
use secrecy::ExposeSecret;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

const REDIRECT: &str = "https://issuer.example.test/callback";
const VERIFIER: &str = "issuer-test-pkce-verifier-0123456789-abcdefghijklmnopqrstuvwxyz";

struct Client {
    id: String,
    secret: String,
}

struct World {
    resources: Arc<ServerContext>,
    user: User,
    /// The athlete's first-party session token
    session: String,
    /// A confidential client listed in `OAUTH2_INTROSPECTION_CLIENTS`, also
    /// able to mint its own client-credentials token
    machine: Client,
}

impl World {
    fn issuer(&self) -> &str {
        &self.resources.common.config.oauth2_server.issuer_url
    }
}

async fn world() -> World {
    common::init_server_config();
    common::init_test_http_clients();
    let machine_id = format!("issuer-machine-{}", Uuid::new_v4().simple());
    let mut config = common::test_server_config();
    config.oauth2_server.introspection_clients = vec![machine_id.clone()];
    let resources = common::create_test_server_resources_with_oauth_issuer(config)
        .await
        .unwrap();
    let secret = ClientRegistrationManager::new(resources.common.repos.oauth2_server.clone())
        .register_client_credentials_client(&machine_id, "Issuer machine")
        .await
        .unwrap();
    let email = format!("issuer_{}@example.test", Uuid::new_v4());
    let (user, session) = common::create_test_tenant(&resources, &email)
        .await
        .unwrap();
    World {
        resources,
        user,
        session,
        machine: Client {
            id: machine_id,
            secret,
        },
    }
}

fn app(resources: &Arc<ServerContext>) -> axum::Router {
    ProviderToolRouter::build_http_app(resources)
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_663))))
}

/// The A2A router over `resources`, as the server mounts it.
fn a2a_routes(resources: &Arc<ServerContext>) -> axum::Router {
    let tool_runtime: Arc<dyn ToolRuntime> = resources.clone();
    A2ARoutes::routes(A2ARoutesState {
        ctx: resources.clone(),
        client_manager: resources.a2a.a2a_client_manager.clone(),
        auth_middleware: resources.auth.auth_middleware.clone(),
        tool_runtime,
        coaching: None,
    })
}

async fn register(world: &World) -> Client {
    let registered =
        ClientRegistrationManager::new(world.resources.common.repos.oauth2_server.clone())
            .register_client(
                ClientRegistrationRequest {
                    redirect_uris: vec![REDIRECT.to_owned()],
                    client_name: Some("Issuer client".to_owned()),
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

/// The athlete approves `client` and the code is exchanged: the access and
/// refresh tokens.
async fn connect(world: &World, client: &Client) -> (String, String) {
    let csrf = world
        .resources
        .auth
        .csrf_manager
        .generate_token(world.user.id)
        .unwrap();
    let challenge = general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()));
    let state = format!("state-{}", Uuid::new_v4().simple());
    let cookie = format!("{}={}", auth_cookie_name(), world.session);
    let query = authorize_query(&[
        ("response_type", "code"),
        ("client_id", client.id.as_str()),
        ("redirect_uri", REDIRECT),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ]);
    let digest = review_digest(&consent_page(app(&world.resources), &cookie, &query).await);
    let approved = AxumTestRequest::post("/oauth2/consent")
        .header("cookie", &cookie)
        .form(&[
            ("response_type", "code"),
            ("client_id", client.id.as_str()),
            ("redirect_uri", REDIRECT),
            ("state", state.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("decision", "approve"),
            ("csrf_token", csrf.as_str()),
            ("review_digest", digest.as_str()),
        ])
        .send(app(&world.resources))
        .await;
    assert_eq!(approved.status(), 303, "{}", approved.body_text());
    let location = approved.header("location").unwrap();
    let code = Url::parse(location)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "code")
        .unwrap()
        .1
        .into_owned();
    let exchanged = AxumTestRequest::post("/oauth2/token")
        .form(&json!({
            "grant_type": "authorization_code",
            "code": code,
            "redirect_uri": REDIRECT,
            "client_id": client.id,
            "client_secret": client.secret,
            "code_verifier": VERIFIER,
        }))
        .send(app(&world.resources))
        .await;
    assert_eq!(exchanged.status(), 200, "{}", exchanged.body_text());
    let body: Value = serde_json::from_str(&exchanged.body_text()).unwrap();
    (
        body["access_token"].as_str().unwrap().to_owned(),
        body["refresh_token"].as_str().unwrap().to_owned(),
    )
}

/// An A2A client registered through `POST /a2a/clients` by the athlete, and
/// the client-credentials token it mints for itself.
async fn client_credentials(world: &World) -> String {
    let created: Value = AxumTestRequest::post("/a2a/clients")
        .header("Authorization", &format!("Bearer {}", world.session))
        .json(&json!({
            "name": "issuer-agent",
            "description": "mints its own client-credentials token",
            "capabilities": ["fitness-data-analysis"],
            "contact_email": "ops@example.com",
        }))
        .send(a2a_routes(&world.resources))
        .await
        .json();
    let client_id = created["client_id"].as_str().unwrap();
    let client_secret = created["client_secret"].as_str().unwrap();
    let response = AxumTestRequest::post("/oauth2/token")
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", client_id),
            ("client_secret", client_secret),
        ])
        .send(app(&world.resources))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let body: Value = serde_json::from_str(&response.body_text()).unwrap();
    body["access_token"].as_str().unwrap().to_owned()
}

fn iss(token: &str) -> String {
    insecure_decode::<Claims>(token).unwrap().claims.iss
}

/// `token` with its claims unchanged but `iss`, and `exp` when given,
/// re-signed with the server's active key.
fn resigned(world: &World, token: &str, issuer: &str, exp: Option<i64>) -> String {
    let mut claims = insecure_decode::<Claims>(token).unwrap().claims;
    issuer.clone_into(&mut claims.iss);
    if let Some(exp) = exp {
        claims.iat = exp - 3600;
        claims.exp = exp;
    }
    let key = world.resources.auth.jwks_manager.get_active_key().unwrap();
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(key.kid.clone());
    encode(&header, &claims, &key.encoding_key().unwrap()).unwrap()
}

/// `tools/list` at `/mcp` with `bearer`.
async fn tools_list(world: &World, bearer: &str) -> AxumTestResponse {
    AxumTestRequest::post("/mcp")
        .header("authorization", &format!("Bearer {bearer}"))
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}))
        .send(app(&world.resources))
        .await
}

fn assert_tools_listed(response: &AxumTestResponse, why: &str) {
    assert_eq!(response.status(), 200, "{why}: {}", response.body_text());
    let body: Value = serde_json::from_str(&response.body_text()).unwrap();
    assert!(
        body["result"]["tools"]
            .as_array()
            .is_some_and(|t| !t.is_empty()),
        "{why}: {body}"
    );
}

/// `GetExtendedAgentCard` over A2A's JSON-RPC binding with `bearer`.
async fn extended_card(world: &World, bearer: &str) -> Value {
    AxumTestRequest::post("/a2a/jsonrpc")
        .header("A2A-Version", "1.0")
        .header("Authorization", &format!("Bearer {bearer}"))
        .json(&json!({"jsonrpc": "2.0", "method": "GetExtendedAgentCard", "id": 1}))
        .send(a2a_routes(&world.resources))
        .await
        .json()
}

fn assert_a2a_admits(envelope: &Value, why: &str) {
    assert!(envelope["error"].is_null(), "{why}: {envelope}");
    assert_eq!(
        envelope["result"]["capabilities"]["extendedAgentCard"], true,
        "{why}: {envelope}"
    );
}

/// A delegated token and a client-credentials token both carry the metadata
/// `issuer`, and `/mcp` and A2A admit them.
#[tokio::test]
async fn an_oauth_token_carries_the_issuer_url_and_is_admitted() {
    let world = world().await;
    let issuer = world.issuer().to_owned();
    assert_ne!(issuer, PIERRE_MCP_SERVER, "the test config names a URL");

    let metadata: Value = AxumTestRequest::get("/.well-known/oauth-authorization-server")
        .send(app(&world.resources))
        .await
        .json();
    assert_eq!(metadata["issuer"], issuer.as_str(), "the metadata's issuer");

    let client = register(&world).await;
    let (access, _) = connect(&world, &client).await;
    assert_eq!(iss(&access), issuer);
    assert_tools_listed(&tools_list(&world, &access).await, "a delegated token");
    assert_a2a_admits(&extended_card(&world, &access).await, "a delegated token");

    let machine = client_credentials(&world).await;
    assert_eq!(iss(&machine), issuer);
    assert_a2a_admits(
        &extended_card(&world, &machine).await,
        "a client-credentials token",
    );

    // The session the athlete signed in with keeps the platform's name.
    assert_eq!(iss(&world.session), PIERRE_MCP_SERVER);
}

/// An OAuth token an older binary minted, `iss` the legacy name, still
/// validates on `/mcp` and A2A until it expires.
#[tokio::test]
async fn an_oauth_token_with_the_legacy_issuer_still_validates() {
    let world = world().await;
    let client = register(&world).await;
    let (access, _) = connect(&world, &client).await;
    let legacy = resigned(&world, &access, PIERRE_MCP_SERVER, None);

    assert_tools_listed(&tools_list(&world, &legacy).await, "a legacy-named token");
    assert_a2a_admits(
        &extended_card(&world, &legacy).await,
        "a legacy-named token",
    );

    let machine = resigned(
        &world,
        &client_credentials(&world).await,
        PIERRE_MCP_SERVER,
        None,
    );
    assert_a2a_admits(
        &extended_card(&world, &machine).await,
        "a legacy-named client-credentials token",
    );
}

/// A session keeps validating with the legacy name and is refused carrying
/// the issuer URL, by both pins (`/mcp` and A2A refuse a session whatever its
/// issuer, so the pins are read through the manager); so is an impersonation
/// token.
#[tokio::test]
async fn a_session_carrying_the_issuer_url_is_refused() {
    let world = world().await;
    let issuer = world.issuer().to_owned();
    let manager = &world.resources.auth.auth_manager;
    let jwks = &world.resources.auth.jwks_manager;

    // The control: the same re-signing with the legacy name is accepted, so
    // the refusal below is the issuer rule, not the re-signing.
    let control = resigned(&world, &world.session, PIERRE_MCP_SERVER, None);
    assert!(manager.validate_token(&control, jwks).is_ok());
    assert!(manager.validate_token_detailed(&control, jwks).is_ok());

    let forged = resigned(&world, &world.session, &issuer, None);
    assert!(manager.validate_token(&forged, jwks).is_err());
    assert!(manager.validate_token_detailed(&forged, jwks).is_err());
    assert!(manager
        .validate_resource_token_detailed(&forged, jwks, &[])
        .is_err());

    let impersonation = manager
        .generate_impersonation_token(&world.user, Uuid::new_v4(), "session-1", jwks, None)
        .unwrap();
    assert_eq!(iss(&impersonation), PIERRE_MCP_SERVER);
    assert!(manager
        .validate_token_detailed(&impersonation, jwks)
        .is_ok());
    let forged = resigned(&world, &impersonation, &issuer, None);
    assert!(manager.validate_token(&forged, jwks).is_err());
    assert!(manager.validate_token_detailed(&forged, jwks).is_err());
}

/// `/oauth2/validate-and-refresh` answers a new token valid, and refreshes it
/// once expired into another carrying the issuer URL.
#[tokio::test]
async fn validate_and_refresh_handles_a_new_token() {
    let world = world().await;
    let issuer = world.issuer().to_owned();
    let client = register(&world).await;
    let (access, refresh) = connect(&world, &client).await;

    let validate = |bearer: String, refresh_token: Option<String>| {
        let resources = Arc::clone(&world.resources);
        async move {
            let response = AxumTestRequest::post("/oauth2/validate-and-refresh")
                .header("authorization", &format!("Bearer {bearer}"))
                .json(&json!({ "refresh_token": refresh_token }))
                .send(app(&resources))
                .await;
            assert_eq!(response.status(), 200, "{}", response.body_text());
            serde_json::from_str::<Value>(&response.body_text()).unwrap()
        }
    };

    let live = validate(access.clone(), None).await;
    assert_eq!(live["status"], "valid", "{live}");

    let expired = resigned(
        &world,
        &access,
        &issuer,
        Some(Timestamp::now().epoch_seconds() - 3600),
    );
    let refreshed = validate(expired, Some(refresh)).await;
    assert_eq!(refreshed["status"], "refreshed", "{refreshed}");
    let renewed = refreshed["access_token"].as_str().unwrap();
    assert_eq!(iss(renewed), issuer);
}

/// Introspection reports the issuer URL a new token carries.
#[tokio::test]
async fn introspection_reports_the_issuer_url() {
    let world = world().await;
    let client = register(&world).await;
    let (access, _) = connect(&world, &client).await;

    let basic =
        general_purpose::STANDARD.encode(format!("{}:{}", world.machine.id, world.machine.secret));
    let response = AxumTestRequest::post("/oauth2/introspect")
        .header("authorization", &format!("Basic {basic}"))
        .form(&[
            ("token", access.as_str()),
            ("token_type_hint", "access_token"),
        ])
        .send(app(&world.resources))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let answer: Value = serde_json::from_str(&response.body_text()).unwrap();
    assert_eq!(answer["active"], true, "{answer}");
    assert_eq!(answer["iss"], world.issuer(), "{answer}");
}
