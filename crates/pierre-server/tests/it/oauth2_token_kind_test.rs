// ABOUTME: Token kinds (owl PDPP §5.1) derived from a token's claims, and the 15-minute lifetime of third-party tokens
// ABOUTME: Introspection names a delegated token's kind and its cnf; discovery lists the kinds and the DPoP algorithms

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The kinds are derived, never stored: `owner` for a session of Dravr's own
//! apps, `client` for a third party's delegated token. A third party's
//! access token lives 15 minutes, and its `exp`, the token response's
//! `expires_in` and introspection all say so; Dravr's own apps keep an hour.

use crate::common;
use crate::helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use crate::helpers::consent_form::{authorize_query, consent_page, review_digest};

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use axum::Router;
use base64::{engine::general_purpose, Engine as _};
use jsonwebtoken::dangerous::insecure_decode;
use pierre_auth::auth::{oauth_access_token_lifetime_secs, Claims, DelegatedGrant};
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::first_party::{MOBILE_CLIENT_ID, WEB_CLIENT_ID};
use pierre_auth::oauth2_server::models::ClientRegistrationRequest;
use pierre_auth::security::cookies::auth_cookie_name;
use pierre_core::constants::limits::{
    OAUTH_ACCESS_TOKEN_EXPIRY_HOURS, OAUTH_DELEGATED_ACCESS_TOKEN_EXPIRY_MINUTES,
};
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::models::User;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use secrecy::ExposeSecret;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

const REDIRECT: &str = "https://kinds.example.test/callback";
const VERIFIER: &str = "token-kind-pkce-verifier-0123456789-abcdefghijklmnopqrstuvwxyz";

/// Fifteen minutes, in seconds: a third-party access token's lifetime.
const DELEGATED_SECS: i64 = OAUTH_DELEGATED_ACCESS_TOKEN_EXPIRY_MINUTES * 60;

fn app(resources: &Arc<ServerContext>) -> Router {
    ProviderToolRouter::build_http_app(resources)
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_951))))
}

struct Client {
    id: String,
    secret: String,
}

struct World {
    resources: Arc<ServerContext>,
    user: User,
    session: String,
    cookie: String,
    introspector: Client,
}

async fn world() -> World {
    common::init_server_config();
    common::init_test_http_clients();
    let introspector_id = format!("resource-server-{}", Uuid::new_v4().simple());
    let mut config = common::test_server_config();
    config.oauth2_server.introspection_clients = vec![introspector_id.clone()];
    let resources = common::create_test_server_resources_with_config(config)
        .await
        .unwrap();
    let secret = ClientRegistrationManager::new(resources.common.repos.oauth2_server.clone())
        .register_client_credentials_client(&introspector_id, "Resource server")
        .await
        .unwrap();
    let email = format!("kinds_{}@example.test", Uuid::new_v4());
    let (user, session) = common::create_test_tenant(&resources, &email)
        .await
        .unwrap();
    World {
        resources,
        user,
        cookie: format!("{}={session}", auth_cookie_name()),
        session,
        introspector: Client {
            id: introspector_id,
            secret,
        },
    }
}

async fn register(world: &World) -> Client {
    let registered =
        ClientRegistrationManager::new(world.resources.common.repos.oauth2_server.clone())
            .register_client(
                ClientRegistrationRequest {
                    redirect_uris: vec![REDIRECT.to_owned()],
                    client_name: Some("Kind client".to_owned()),
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

/// The athlete approves `client` and the code is exchanged: the token
/// response.
async fn connect(world: &World, client: &Client) -> Value {
    let csrf = world
        .resources
        .auth
        .csrf_manager
        .generate_token(world.user.id)
        .unwrap();
    let challenge = general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()));
    let state = format!("state-{}", Uuid::new_v4().simple());
    let query = authorize_query(&[
        ("response_type", "code"),
        ("client_id", client.id.as_str()),
        ("redirect_uri", REDIRECT),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ]);
    let (digest_field, digest) =
        review_digest(&consent_page(app(&world.resources), &world.cookie, &query).await);
    let approved = AxumTestRequest::post("/oauth2/consent")
        .header("cookie", &world.cookie)
        .form(&[
            ("response_type", "code"),
            ("client_id", client.id.as_str()),
            ("redirect_uri", REDIRECT),
            ("state", state.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("decision", "approve"),
            ("csrf_token", csrf.as_str()),
            (digest_field, digest.as_str()),
        ])
        .send(app(&world.resources))
        .await;
    assert_eq!(approved.status(), 303, "{}", approved.body_text());
    let code = Url::parse(approved.header("location").unwrap())
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
    serde_json::from_str(&exchanged.body_text()).unwrap()
}

async fn introspect(world: &World, token: &str) -> Value {
    let basic = general_purpose::STANDARD.encode(format!(
        "{}:{}",
        world.introspector.id, world.introspector.secret
    ));
    let response: AxumTestResponse = AxumTestRequest::post("/oauth2/introspect")
        .header("authorization", &format!("Basic {basic}"))
        .form(&[("token", token)])
        .send(app(&world.resources))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    serde_json::from_str(&response.body_text()).unwrap()
}

/// A third party's token is the `client` kind, lives 15 minutes, and
/// introspection says both; it carries no `cnf`, so it is a Bearer token.
#[tokio::test]
async fn a_third_party_token_is_the_client_kind_and_lives_fifteen_minutes() {
    let world = world().await;
    let client = register(&world).await;
    let issued = connect(&world, &client).await;
    assert_eq!(issued["expires_in"], DELEGATED_SECS, "{issued}");
    let access = issued["access_token"].as_str().unwrap();
    let claims = insecure_decode::<Claims>(access).unwrap().claims;
    assert_eq!(
        claims.exp - claims.iat,
        DELEGATED_SECS,
        "bug: a third party's token outlives fifteen minutes"
    );
    assert_eq!(
        claims.pdpp_token_kind(),
        Some("client"),
        "bug: a third party's token is not the client kind"
    );

    let answer = introspect(&world, access).await;
    assert_eq!(answer["active"], true, "{answer}");
    assert_eq!(answer["pdpp_token_kind"], "client", "{answer}");
    assert_eq!(answer["token_type"], "Bearer", "{answer}");
    assert_eq!(
        answer["exp"], claims.exp,
        "introspection's exp is the token's"
    );
    assert!(
        answer.get("cnf").is_none(),
        "an unbound token has no cnf: {answer}"
    );

    // A refresh mints the same lifetime.
    let refreshed = AxumTestRequest::post("/oauth2/token")
        .form(&json!({
            "grant_type": "refresh_token",
            "refresh_token": issued["refresh_token"],
            "client_id": client.id,
            "client_secret": client.secret,
        }))
        .send(app(&world.resources))
        .await;
    assert_eq!(refreshed.status(), 200, "{}", refreshed.body_text());
    let refreshed: Value = serde_json::from_str(&refreshed.body_text()).unwrap();
    assert_eq!(
        refreshed["expires_in"], DELEGATED_SECS,
        "bug: a refresh changed the third party's lifetime: {refreshed}"
    );
}

/// Dravr's own session is the `owner` kind. Introspection describes no
/// session (no resource server holds one), so it answers inactive.
#[tokio::test]
async fn a_session_is_the_owner_kind_and_introspection_does_not_describe_it() {
    let world = world().await;
    let claims = insecure_decode::<Claims>(&world.session).unwrap().claims;
    assert!(claims.is_session(), "the fixture session is a session");
    assert_eq!(
        claims.pdpp_token_kind(),
        Some("owner"),
        "bug: Dravr's own session is not the owner kind"
    );
    let answer = introspect(&world, &world.session).await;
    assert_eq!(answer["active"], false, "{answer}");
    assert!(answer.get("pdpp_token_kind").is_none(), "{answer}");
}

/// Dravr's own apps keep the hour; a third party gets fifteen minutes,
/// whether the token is minted by the code grant or directly.
#[tokio::test]
async fn first_party_lifetimes_are_unchanged() {
    let hour = OAUTH_ACCESS_TOKEN_EXPIRY_HOURS * 3600;
    assert_eq!(
        oauth_access_token_lifetime_secs(WEB_CLIENT_ID),
        hour,
        "bug: the web app lost its hour"
    );
    assert_eq!(
        oauth_access_token_lifetime_secs(MOBILE_CLIENT_ID),
        hour,
        "bug: the mobile app lost its hour"
    );
    assert_eq!(
        oauth_access_token_lifetime_secs("some-registered-client"),
        DELEGATED_SECS,
        "bug: a registered client outlives fifteen minutes"
    );

    let world = world().await;
    let scopes = vec!["fitness:read".to_owned()];
    for (client_id, lifetime) in [
        (WEB_CLIENT_ID, hour),
        ("some-registered-client", DELEGATED_SECS),
    ] {
        let token = world
            .resources
            .auth
            .auth_manager
            .generate_oauth_access_token(
                &world.resources.auth.jwks_manager,
                &DelegatedGrant {
                    client_id,
                    scopes: &scopes,
                    audience: None,
                },
                &world.user.id,
                None,
            )
            .unwrap();
        let claims = insecure_decode::<Claims>(&token).unwrap().claims;
        assert_eq!(claims.exp - claims.iat, lifetime, "{client_id}");
        assert!(
            claims.cnf.is_none(),
            "a token minted with no proof is unbound"
        );
    }
}

/// The protected resource metadata lists the kinds this server serves and
/// the `DPoP` algorithms; the authorization server metadata lists the
/// algorithms too.
#[tokio::test]
async fn discovery_names_the_kinds_and_the_dpop_algorithms() {
    let world = world().await;
    let prm = AxumTestRequest::get("/.well-known/oauth-protected-resource")
        .send(app(&world.resources))
        .await;
    assert_eq!(prm.status(), 200, "{}", prm.body_text());
    let prm: Value = serde_json::from_str(&prm.body_text()).unwrap();
    assert_eq!(
        prm["pdpp_token_kinds_supported"],
        json!(["owner", "client", "agent_write"]),
        "bug: discovery does not list the three token kinds: {prm}"
    );
    assert!(
        prm["health_agent_write_base"]
            .as_str()
            .is_some_and(|base| base.ends_with("/api/agent-write/v0")),
        "bug: discovery names no agent-write base (owl PDPP §5.3): {prm}"
    );
    assert_eq!(
        prm["dpop_bound_access_tokens_required"], false,
        "bug: discovery requires DPoP of every client: {prm}"
    );
    assert!(
        prm["dpop_signing_alg_values_supported"]
            .as_array()
            .unwrap()
            .contains(&json!("ES256")),
        "{prm}"
    );

    let metadata = AxumTestRequest::get("/.well-known/oauth-authorization-server")
        .send(app(&world.resources))
        .await;
    assert_eq!(metadata.status(), 200, "{}", metadata.body_text());
    let metadata: Value = serde_json::from_str(&metadata.body_text()).unwrap();
    assert_eq!(
        metadata["dpop_signing_alg_values_supported"], prm["dpop_signing_alg_values_supported"],
        "one list of algorithms, published twice"
    );
}

/// A third party's token whose scope is exactly `pdpp:agent_write` is the `agent_write` kind
/// (owl PDPP §5.1, carnet#1544 slice 4c).
///
/// One holding it beside another scope stays `client`, and so does Dravr's own app.
#[tokio::test]
async fn an_agent_write_scope_alone_is_the_agent_write_kind() {
    let world = world().await;
    let kind_of = |client_id: &str, scopes: &[&str]| {
        let scopes: Vec<String> = scopes.iter().map(|s| (*s).to_owned()).collect();
        let token = world
            .resources
            .auth
            .auth_manager
            .generate_oauth_access_token(
                &world.resources.auth.jwks_manager,
                &DelegatedGrant {
                    client_id,
                    scopes: &scopes,
                    audience: Some("https://mcp.example.test/api/agent-write/v0"),
                },
                &world.user.id,
                None,
            )
            .unwrap();
        insecure_decode::<Claims>(&token)
            .unwrap()
            .claims
            .pdpp_token_kind()
    };
    assert_eq!(
        kind_of("some-registered-client", &["pdpp:agent_write"]),
        Some("agent_write"),
        "bug: an agent-write token is not its own kind"
    );
    assert_eq!(
        kind_of(
            "some-registered-client",
            &["fitness:read", "pdpp:agent_write"]
        ),
        Some("client"),
        "bug: a mixed scope was read as agent_write"
    );
    assert_eq!(
        kind_of("some-registered-client", &["fitness:read"]),
        Some("client")
    );
}
