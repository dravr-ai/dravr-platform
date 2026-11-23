// ABOUTME: RFC 7662 token introspection — only a listed confidential client may ask, and the answer follows the grant
// ABOUTME: Active members name the consent grant; a revoked or expired grant, or a refresh token, answers inactive with its cause
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `POST /oauth2/introspect` over the server's own HTTP composition, for a
//! deployment naming one resource server in `OAUTH2_INTROSPECTION_CLIENTS`.

use crate::common;
use crate::helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use crate::helpers::consent_form::{
    authorize_query, consent_page, hidden_field, review_digest, with_grant_lines,
};

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use base64::{engine::general_purpose, Engine as _};
use dravr_tronc::dates::{Elapsed, Timestamp};
use jsonwebtoken::dangerous::insecure_decode;
use pierre_auth::auth::Claims;
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::models::ClientRegistrationRequest;
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

const REDIRECT: &str = "https://introspect.example.test/callback";
const VERIFIER: &str = "introspection-pkce-verifier-0123456789-abcdefghijklmnopqrstuv";

fn app(resources: &Arc<ServerContext>) -> axum::Router {
    ProviderToolRouter::build_http_app(resources)
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_662))))
}

struct Client {
    id: String,
    secret: String,
}

struct World {
    resources: Arc<ServerContext>,
    user: User,
    tenant_id: String,
    /// The athlete's web session, as a cookie
    cookie: String,
    /// The resource server named in `OAUTH2_INTROSPECTION_CLIENTS`
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
    let email = format!("introspect_{}@example.test", Uuid::new_v4());
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
        .unwrap()
        .to_string();
    World {
        resources,
        user,
        tenant_id,
        cookie: format!("{}={session}", auth_cookie_name()),
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
                    client_name: Some("Introspected client".to_owned()),
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

/// The athlete approves `client`, and the code is exchanged: the access and
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
    let query = authorize_query(&[
        ("response_type", "code"),
        ("client_id", client.id.as_str()),
        ("redirect_uri", REDIRECT),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ]);
    let digest = review_digest(&consent_page(app(&world.resources), &world.cookie, &query).await);
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

/// `POST /oauth2/introspect` for `token`, the caller authenticating with
/// Basic as `caller`.
async fn introspect_as(world: &World, caller: &Client, token: &str) -> AxumTestResponse {
    let basic = general_purpose::STANDARD.encode(format!("{}:{}", caller.id, caller.secret));
    AxumTestRequest::post("/oauth2/introspect")
        .header("authorization", &format!("Basic {basic}"))
        .form(&[("token", token), ("token_type_hint", "access_token")])
        .send(app(&world.resources))
        .await
}

async fn introspect(world: &World, token: &str) -> Value {
    let response = introspect_as(world, &world.introspector, token).await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    assert_eq!(response.header("cache-control"), Some("no-store"));
    serde_json::from_str(&response.body_text()).unwrap()
}

fn assert_inactive(answer: &Value, cause: &str) {
    assert_eq!(
        answer,
        &json!({"active": false, "health_inactive_cause": cause}),
        "an inactive answer says nothing more"
    );
}

async fn forget_standing(world: &World, client: &Client) {
    world
        .resources
        .account_service()
        .grant_standings()
        .forget(world.user.id, &world.tenant_id, &client.id)
        .await;
}

async fn the_grant(world: &World) -> String {
    world
        .resources
        .common
        .repos
        .oauth2_server
        .list_client_grants(&world.user.id.to_string(), &world.tenant_id)
        .await
        .unwrap()[0]
        .id
        .clone()
}

/// A client not named in `OAUTH2_INTROSPECTION_CLIENTS`, one without its
/// secret, and a caller presenting nothing are all refused 401.
#[tokio::test]
async fn an_unlisted_client_is_refused_401() {
    let world = world().await;
    let client = register(&world).await;
    let (access, _) = connect(&world, &client).await;

    let unlisted = introspect_as(&world, &client, &access).await;
    assert_eq!(unlisted.status(), 401, "{}", unlisted.body_text());
    let body: Value = serde_json::from_str(&unlisted.body_text()).unwrap();
    assert_eq!(body["error"], "invalid_client");
    assert!(unlisted.header("www-authenticate").is_some());

    let wrong = Client {
        id: world.introspector.id.clone(),
        secret: "not-the-secret".to_owned(),
    };
    assert_eq!(introspect_as(&world, &wrong, &access).await.status(), 401);

    let anonymous = AxumTestRequest::post("/oauth2/introspect")
        .form(&[("token", access.as_str())])
        .send(app(&world.resources))
        .await;
    assert_eq!(anonymous.status(), 401, "{}", anonymous.body_text());
}

/// An active delegated token answers its members, and `grant_id` is the
/// consent grant, not the refresh family the JWT's own `grant_id` names.
#[tokio::test]
async fn an_active_token_answers_its_members_and_its_consent_grant() {
    let world = world().await;
    let client = register(&world).await;
    let (access, _) = connect(&world, &client).await;
    let claims = insecure_decode::<Claims>(&access).unwrap().claims;
    let held = the_grant(&world).await;

    let answer = introspect(&world, &access).await;
    assert_eq!(answer["active"], true, "{answer}");
    assert_eq!(answer["client_id"], client.id.as_str());
    assert_eq!(answer["sub"], world.user.id.to_string());
    assert_eq!(answer["scope"], claims.scope.as_str());
    assert_eq!(answer["aud"], claims.aud.as_str());
    assert_eq!(answer["iss"], claims.iss.as_str());
    assert_eq!(answer["exp"], claims.exp);
    assert_eq!(answer["iat"], claims.iat);
    assert_eq!(answer["token_type"], "Bearer");
    assert_eq!(answer["grant_id"], held.as_str(), "the consent grant");
    assert_ne!(
        answer["grant_id"].as_str(),
        claims.grant_id.as_deref(),
        "never the refresh family"
    );
    assert!(answer.get("email").is_none(), "no claim beyond the members");
    assert!(answer.get("health_inactive_cause").is_none());
}

/// A token whose grant the athlete revoked is inactive, `revoked`.
#[tokio::test]
async fn a_revoked_grant_answers_revoked() {
    let world = world().await;
    let client = register(&world).await;
    let (access, _) = connect(&world, &client).await;
    let held = the_grant(&world).await;
    assert!(world
        .resources
        .common
        .repos
        .oauth2_server
        .revoke_client_grant(&held, &world.user.id.to_string(), &world.tenant_id)
        .await
        .unwrap());
    forget_standing(&world, &client).await;

    assert_inactive(&introspect(&world, &access).await, "revoked");
}

/// A token whose grant is past its expiry is inactive, `expired`.
#[tokio::test]
async fn an_expired_grant_answers_expired() {
    let world = world().await;
    let client = register(&world).await;
    let (access, _) = connect(&world, &client).await;
    let held = the_grant(&world).await;
    let past = Timestamp::now() - Elapsed::minutes(1);
    let sql = "UPDATE oauth_client_grants SET expires_at = $1 WHERE id = $2";
    match world.resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(sql)
                .bind(past)
                .bind(&held)
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(sql)
                .bind(past)
                .bind(&held)
                .execute(db.pool())
                .await
                .unwrap();
        }
    }
    forget_standing(&world, &client).await;

    assert_inactive(&introspect(&world, &access).await, "expired");
}

/// A refresh token, or anything that is not a token this server issued, is
/// inactive.
#[tokio::test]
async fn a_refresh_token_or_garbage_is_inactive() {
    let world = world().await;
    let client = register(&world).await;
    let (_, refresh) = connect(&world, &client).await;

    assert_inactive(&introspect(&world, &refresh).await, "unknown");
    assert_inactive(&introspect(&world, "not-a-token").await, "unknown");
}

/// A first-party session is no client's token, so no resource server learns
/// about it here.
#[tokio::test]
async fn a_session_token_is_inactive() {
    let world = world().await;
    let session = world
        .resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &world.user,
            &world.resources.auth.jwks_manager,
            Some(world.tenant_id.clone()),
        )
        .unwrap();
    assert_inactive(&introspect(&world, &session).await, "unknown");
}

/// A client-credentials token, which no user or grant stands behind, is
/// active while it verifies and its client is registered.
#[tokio::test]
async fn a_client_credentials_token_is_active() {
    let world = world().await;
    let issued = AxumTestRequest::post("/oauth2/token")
        .form(&json!({
            "grant_type": "client_credentials",
            "client_id": world.introspector.id,
            "client_secret": world.introspector.secret,
        }))
        .send(app(&world.resources))
        .await;
    assert_eq!(issued.status(), 200, "{}", issued.body_text());
    let body: Value = serde_json::from_str(&issued.body_text()).unwrap();
    let token = body["access_token"].as_str().unwrap().to_owned();

    let answer = introspect(&world, &token).await;
    assert_eq!(answer["active"], true, "{answer}");
    assert_eq!(answer["client_id"], world.introspector.id.as_str());
    assert_eq!(answer["sub"], world.introspector.id.as_str());
    assert!(answer.get("grant_id").is_none(), "no consent grant");
}

/// The client `pierre-cli oauth2-client create` provisions holds no scope:
/// it authenticates, which is all introspection asks of it, and a
/// `client_credentials` request from it is refused whatever it names, so it
/// mints nothing.
#[tokio::test]
async fn an_admin_provisioned_introspection_client_authenticates_and_mints_nothing() {
    let world = world().await;
    let manager =
        ClientRegistrationManager::new(world.resources.common.repos.oauth2_server.clone());
    let (id, secret) = manager
        .register_introspection_client("Satellite resource server")
        .await
        .unwrap();
    let secret = secret.expose_secret().to_owned();
    assert!(manager.authenticate_client(&id, &secret).await.is_ok());
    for scope in [None, Some(""), Some("fitness:read")] {
        let mut form = json!({
            "grant_type": "client_credentials",
            "client_id": id,
            "client_secret": secret,
        });
        if let Some(scope) = scope {
            form["scope"] = json!(scope);
        }
        let issued = AxumTestRequest::post("/oauth2/token")
            .form(&form)
            .send(app(&world.resources))
            .await;
        assert_eq!(issued.status(), 400, "{scope:?}: {}", issued.body_text());
        let body: Value = serde_json::from_str(&issued.body_text()).unwrap();
        assert_eq!(body["error"], "invalid_scope", "{scope:?}: {body}");
    }
}

/// The athlete connects `client` choosing Training coach on the picker: the
/// access token.
async fn connect_as_training_coach(world: &World, client: &Client) -> String {
    with_grant_lines(&world.resources.mcp.messaging_strings_registry);
    let challenge = general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()));
    let state = format!("state-{}", Uuid::new_v4().simple());
    let request = [
        ("response_type", "code"),
        ("client_id", client.id.as_str()),
        ("redirect_uri", REDIRECT),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ];
    let page = consent_page(
        app(&world.resources),
        &world.cookie,
        &authorize_query(&request),
    )
    .await;
    let digest = hidden_field(&page, "review_digest_training_coach").unwrap();
    let csrf = world
        .resources
        .auth
        .csrf_manager
        .generate_token(world.user.id)
        .unwrap();
    let mut form = request.to_vec();
    form.extend([
        ("grant", "training_coach"),
        ("review_digest_training_coach", digest.as_str()),
        ("csrf_token", csrf.as_str()),
        ("decision", "approve"),
    ]);
    let approved = AxumTestRequest::post("/oauth2/consent")
        .header("cookie", &world.cookie)
        .form(&form)
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
    serde_json::from_str::<Value>(&exchanged.body_text()).unwrap()["access_token"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// A token under a preset grant carries the grant's resolved details: the
/// preset by name, never a stream. A plain grant's carries none.
#[tokio::test]
async fn a_preset_grant_introspects_with_its_resolved_details() {
    let world = world().await;
    // A plain grant, given while the picker's lines are not in the catalogue.
    let plain_client = register(&world).await;
    let (plain, _) = connect(&world, &plain_client).await;
    let client = register(&world).await;
    let access = connect_as_training_coach(&world, &client).await;

    let answer = introspect(&world, &access).await;
    assert_eq!(answer["active"], true, "{answer}");
    let details = &answer["authorization_details"];
    assert_eq!(details[0]["type"], "https://pdpp.dev/data-access");
    assert_eq!(details[0]["selection_preset"], "training_coach");
    assert!(details[0]["window"]["since"].is_string(), "{details}");
    assert_eq!(
        details[1]["type"],
        "https://dravr.ai/pdpp/health-profile/v0/health-constraints"
    );
    assert_eq!(details[1]["max_tier"], "T1");
    assert!(
        !answer.to_string().contains("health.vault"),
        "no stream names: {answer}"
    );

    let plain_answer = introspect(&world, &plain).await;
    assert_eq!(plain_answer["active"], true);
    assert!(plain_answer.get("authorization_details").is_none());
}

// ============================================================================
// /oauth2/token-validate: the SDK's check, a thin caller of introspection
// ============================================================================

/// `POST /oauth2/token-validate` with `token` as a Bearer credential and
/// `client_id` in the body, when given.
async fn token_validate(
    world: &World,
    authorization: Option<&str>,
    client_id: Option<&str>,
) -> Value {
    let mut request = AxumTestRequest::post("/oauth2/token-validate");
    if let Some(authorization) = authorization {
        request = request.header("authorization", authorization);
    }
    let body = client_id.map_or_else(|| json!({}), |client_id| json!({"client_id": client_id}));
    let response = request.json(&body).send(app(&world.resources)).await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    serde_json::from_str(&response.body_text()).unwrap()
}

/// What the SDK has always been answered, for every credential it sends.
#[tokio::test]
async fn token_validate_answers_the_sdk_as_before() {
    let world = world().await;
    let client = register(&world).await;
    let (access, _) = connect(&world, &client).await;
    let bearer = format!("Bearer {access}");

    assert_eq!(
        token_validate(&world, Some(&bearer), None).await,
        json!({"valid": true})
    );
    assert_eq!(
        token_validate(&world, Some(&bearer), Some(&client.id)).await,
        json!({"valid": true})
    );
    assert_eq!(
        token_validate(&world, Some("Bearer not-a-token"), None).await,
        json!({"valid": false, "error": "invalid_token", "error_description": "Access token is invalid or expired"})
    );
    assert_eq!(
        token_validate(&world, Some(&format!("Basic {access}")), None).await,
        json!({"valid": false, "error": "invalid_request", "error_description": "Authorization header must use Bearer scheme"})
    );
    assert_eq!(
        token_validate(&world, None, Some(&client.id)).await,
        json!({"valid": true})
    );
    assert_eq!(
        token_validate(&world, None, Some("no-such-client")).await,
        json!({"valid": false, "error": "invalid_client", "error_description": "Client ID not found or invalid"})
    );
    assert_eq!(
        token_validate(&world, None, None).await,
        json!({"valid": false, "error": "invalid_request", "error_description": "Either access token or client_id must be provided"})
    );
}

/// The answer follows introspection: a token whose grant was withdrawn, and
/// a first-party session, which no client holds, are not valid here either.
#[tokio::test]
async fn token_validate_follows_introspection() {
    let world = world().await;
    let client = register(&world).await;
    let (access, _) = connect(&world, &client).await;
    let held = the_grant(&world).await;
    assert!(world
        .resources
        .common
        .repos
        .oauth2_server
        .revoke_client_grant(&held, &world.user.id.to_string(), &world.tenant_id)
        .await
        .unwrap());
    forget_standing(&world, &client).await;
    let invalid = json!({"valid": false, "error": "invalid_token", "error_description": "Access token is invalid or expired"});
    assert_eq!(
        token_validate(&world, Some(&format!("Bearer {access}")), None).await,
        invalid,
        "a withdrawn grant's token"
    );

    let session = world
        .resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &world.user,
            &world.resources.auth.jwks_manager,
            Some(world.tenant_id.clone()),
        )
        .unwrap();
    assert_eq!(
        token_validate(&world, Some(&format!("Bearer {session}")), None).await,
        invalid,
        "a first-party session"
    );
}
