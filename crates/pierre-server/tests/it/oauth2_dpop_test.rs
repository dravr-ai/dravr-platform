// ABOUTME: DPoP (RFC 9449) end to end — a key-bound grant at PAR and the token endpoint, and its token at /mcp and A2A
// ABOUTME: A bound token needs the DPoP scheme and a fresh proof by its key; an unbound Bearer client works as before

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Over the server's own HTTP composition: a client pushes its request with a
//! `DPoP` proof, so the code is bound to the key; the token endpoint then
//! mints a `DPoP` token only for a proof by that key, and `/mcp`, `/mcp/tools`
//! and A2A admit the token only under the `DPoP` scheme with a fresh proof for
//! that request, that token and that key.

use crate::common;
use crate::helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use crate::helpers::consent_form::{
    authorize_query, consent_page, hidden_field, plain_digest, review_digest,
};

use std::net::SocketAddr;
use std::slice;
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use axum::Router;
use base64::{engine::general_purpose, Engine as _};
use dravr_tronc::dates::Timestamp;
use jsonwebtoken::dangerous::insecure_decode;
use jsonwebtoken::{encode, Algorithm, Header};
use pierre_auth::auth::Claims;
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::models::ClientRegistrationRequest;
use pierre_auth::security::cookies::auth_cookie_name;
use pierre_core::constants::limits::OAUTH_DELEGATED_ACCESS_TOKEN_EXPIRY_MINUTES;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::models::User;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_test_support::dpop::{Key, A, B};
use secrecy::ExposeSecret;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

const REDIRECT: &str = "https://dpop.example.test/callback";
const VERIFIER: &str = "dpop-bound-pkce-verifier-0123456789-abcdefghijklmnopqrstuvwxyz";

fn app(resources: &Arc<ServerContext>) -> Router {
    ProviderToolRouter::build_http_app(resources)
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_949))))
}

struct Client {
    id: String,
    secret: String,
}

struct World {
    resources: Arc<ServerContext>,
    user: User,
    cookie: String,
}

impl World {
    /// The authorization server's own URL for `path`: a token or PAR proof's
    /// `htu`.
    fn issuer_url(&self, path: &str) -> String {
        format!(
            "{}{path}",
            self.resources
                .common
                .config
                .oauth2_server
                .issuer_url
                .trim_end_matches('/')
        )
    }

    /// The MCP resource's origin, as a client dials it.
    fn origin(&self) -> Url {
        let resource = &self.resources.common.config.oauth2_server.mcp_resource_url;
        Url::parse(resource).unwrap()
    }

    /// The URL a client dials `path` at on the resource server: a proof's
    /// `htu` at `/mcp` or A2A.
    fn resource_url(&self, path: &str) -> String {
        format!("{}{path}", self.origin().origin().ascii_serialization())
    }

    /// The `Host` and `X-Forwarded-Proto` a request to the resource carries.
    fn dialed(&self, request: AxumTestRequest) -> AxumTestRequest {
        let origin = self.origin();
        let host = origin.port().map_or_else(
            || origin.host_str().unwrap().to_owned(),
            |port| format!("{}:{port}", origin.host_str().unwrap()),
        );
        request
            .header("host", &host)
            .header("x-forwarded-proto", origin.scheme())
    }
}

async fn world() -> World {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await.unwrap();
    let email = format!("dpop_{}@example.test", Uuid::new_v4());
    let (user, session) = common::create_test_tenant(&resources, &email)
        .await
        .unwrap();
    World {
        resources,
        user,
        cookie: format!("{}={session}", auth_cookie_name()),
    }
}

async fn register(world: &World) -> Client {
    let registered =
        ClientRegistrationManager::new(world.resources.common.repos.oauth2_server.clone())
            .register_client(
                ClientRegistrationRequest {
                    redirect_uris: vec![REDIRECT.to_owned()],
                    client_name: Some("DPoP client".to_owned()),
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

fn challenge() -> String {
    general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()))
}

/// `POST /oauth2/par` with `proofs` as its `DPoP` headers and `extra` beside
/// a complete code request.
async fn push_with(
    world: &World,
    client: &Client,
    proofs: &[String],
    extra: &[(&str, &str)],
) -> AxumTestResponse {
    let challenge = challenge();
    let state = format!("state-{}", Uuid::new_v4().simple());
    let mut form = vec![
        ("client_id", client.id.as_str()),
        ("client_secret", client.secret.as_str()),
        ("response_type", "code"),
        ("redirect_uri", REDIRECT),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ];
    form.extend_from_slice(extra);
    let mut request = AxumTestRequest::post("/oauth2/par").form(&form);
    for proof in proofs {
        request = request.header("dpop", proof);
    }
    request.send(app(&world.resources)).await
}

/// The athlete approves the request pushed under `request_uri`: its code.
async fn approve(world: &World, client: &Client, request_uri: &str) -> String {
    let csrf = world
        .resources
        .auth
        .csrf_manager
        .generate_token(world.user.id)
        .unwrap();
    let screen = AxumTestRequest::get(&format!(
        "/oauth2/authorize?client_id={}&request_uri={}",
        urlencoding::encode(&client.id),
        urlencoding::encode(request_uri)
    ))
    .header("cookie", &world.cookie)
    .send(app(&world.resources))
    .await;
    // A remembered approval mints at once.
    if screen.status() == 303 {
        return code_in(screen.header("location").unwrap());
    }
    let (digest_field, digest) =
        plain_digest(&screen.body_text()).unwrap_or(("review_digest", String::new()));
    let approved = AxumTestRequest::post("/oauth2/consent")
        .header("cookie", &world.cookie)
        .form(&[
            ("client_id", client.id.as_str()),
            ("request_uri", request_uri),
            ("decision", "approve"),
            ("csrf_token", csrf.as_str()),
            (digest_field, digest.as_str()),
        ])
        .send(app(&world.resources))
        .await;
    assert_eq!(approved.status(), 303, "{}", approved.body_text());
    code_in(approved.header("location").unwrap())
}

/// The code a redirect to the client carries.
fn code_in(location: &str) -> String {
    Url::parse(location)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "code")
        .unwrap_or_else(|| panic!("a code: {location}"))
        .1
        .into_owned()
}

/// A code bound to `key` by a proof at PAR.
async fn bound_code(world: &World, client: &Client, key: Key) -> String {
    let proof = key
        .proof("POST", &world.issuer_url("/oauth2/par"), None)
        .unwrap();
    let pushed = push_with(world, client, &[proof], &[]).await;
    assert_eq!(pushed.status(), 201, "{}", pushed.body_text());
    let body: Value = serde_json::from_str(&pushed.body_text()).unwrap();
    approve(world, client, body["request_uri"].as_str().unwrap()).await
}

/// An unbound code: a push without a proof.
async fn unbound_code(world: &World, client: &Client) -> String {
    let pushed = push_with(world, client, &[], &[]).await;
    assert_eq!(pushed.status(), 201, "{}", pushed.body_text());
    let body: Value = serde_json::from_str(&pushed.body_text()).unwrap();
    approve(world, client, body["request_uri"].as_str().unwrap()).await
}

/// `POST /oauth2/token` with `form` beside the client's credentials and
/// `proofs` as its `DPoP` headers.
async fn token_request(
    world: &World,
    client: &Client,
    mut form: Value,
    proofs: &[String],
) -> AxumTestResponse {
    form["client_id"] = json!(client.id);
    form["client_secret"] = json!(client.secret);
    let mut request = AxumTestRequest::post("/oauth2/token").form(&form);
    for proof in proofs {
        request = request.header("dpop", proof);
    }
    request.send(app(&world.resources)).await
}

async fn exchange(
    world: &World,
    client: &Client,
    code: &str,
    proofs: &[String],
) -> AxumTestResponse {
    token_request(
        world,
        client,
        json!({
            "grant_type": "authorization_code",
            "code": code,
            "redirect_uri": REDIRECT,
            "code_verifier": VERIFIER,
        }),
        proofs,
    )
    .await
}

async fn refresh(
    world: &World,
    client: &Client,
    refresh_token: &str,
    proofs: &[String],
) -> AxumTestResponse {
    token_request(
        world,
        client,
        json!({"grant_type": "refresh_token", "refresh_token": refresh_token}),
        proofs,
    )
    .await
}

/// A token endpoint refusal of the proof: 400 `invalid_dpop_proof`.
fn assert_proof_refused(response: &AxumTestResponse) {
    assert_eq!(response.status(), 400, "{}", response.body_text());
    let body: Value = serde_json::from_str(&response.body_text()).unwrap();
    assert_eq!(body["error"], "invalid_dpop_proof", "{body}");
}

/// The issued tokens: access and refresh.
fn issued(response: &AxumTestResponse) -> (String, String, Value) {
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let body: Value = serde_json::from_str(&response.body_text()).unwrap();
    (
        body["access_token"].as_str().unwrap().to_owned(),
        body["refresh_token"].as_str().unwrap().to_owned(),
        body,
    )
}

/// A bound token for `key`: pushed, approved and exchanged with its proofs.
async fn bound_tokens(world: &World, client: &Client, key: Key) -> (String, String) {
    let code = bound_code(world, client, key).await;
    let proof = key
        .proof("POST", &world.issuer_url("/oauth2/token"), None)
        .unwrap();
    let (access, refresh, _) = issued(&exchange(world, client, &code, &[proof]).await);
    (access, refresh)
}

/// `POST /mcp` `tools/list`, presenting `authorization` and `proofs`.
async fn mcp(world: &World, authorization: &str, proofs: &[String]) -> AxumTestResponse {
    let mut request = world
        .dialed(AxumTestRequest::post("/mcp"))
        .header("authorization", authorization)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}));
    for proof in proofs {
        request = request.header("dpop", proof);
    }
    request.send(app(&world.resources)).await
}

/// `POST /mcp` with the `DPoP` scheme and one proof by `key` for `token`.
async fn mcp_with(world: &World, token: &str, key: Key) -> AxumTestResponse {
    let proof = key
        .proof("POST", &world.resource_url("/mcp"), Some(token))
        .unwrap();
    mcp(world, &format!("DPoP {token}"), &[proof]).await
}

/// A refusal at the resource server: 401, with the `DPoP` challenge beside
/// the Bearer one.
fn assert_refused_with_dpop_challenge(response: &AxumTestResponse) {
    assert_eq!(response.status(), 401, "{}", response.body_text());
    let challenge = response.header("www-authenticate").unwrap_or_default();
    assert!(challenge.starts_with("DPoP error="), "{challenge}");
    assert!(challenge.contains("algs=\""), "{challenge}");
    assert!(challenge.contains("ES256"), "{challenge}");
    assert!(
        challenge.contains("Bearer resource_metadata="),
        "{challenge}"
    );
}

/// A code bound at PAR is exchanged with a proof by its key: the token is a
/// `DPoP` token carrying `cnf.jkt`, `/mcp` admits it under the `DPoP` scheme
/// with a fresh proof, and refuses it as Bearer and on a replayed proof.
#[tokio::test]
async fn a_bound_code_mints_a_dpop_token_the_mcp_door_admits_with_its_proof() {
    let world = world().await;
    let client = register(&world).await;
    let code = bound_code(&world, &client, A).await;
    let proof = A
        .proof("POST", &world.issuer_url("/oauth2/token"), None)
        .unwrap();
    let (access, _, body) = issued(&exchange(&world, &client, &code, &[proof]).await);
    assert_eq!(body["token_type"], "DPoP", "{body}");
    assert_eq!(
        body["expires_in"],
        OAUTH_DELEGATED_ACCESS_TOKEN_EXPIRY_MINUTES * 60,
        "a third-party client's token lives 15 minutes"
    );
    let claims = insecure_decode::<Claims>(&access).unwrap().claims;
    assert_eq!(
        claims.cnf.map(|cnf| cnf.jkt),
        Some(A.thumbprint().unwrap()),
        "bug: the access token is bound to the key the proof was signed with"
    );

    let admitted = mcp_with(&world, &access, A).await;
    assert_eq!(admitted.status(), 200, "{}", admitted.body_text());

    // RFC 9449 §7.2: never as Bearer, with or without a proof.
    assert_refused_with_dpop_challenge(&mcp(&world, &format!("Bearer {access}"), &[]).await);
    let proof = A
        .proof("POST", &world.resource_url("/mcp"), Some(&access))
        .unwrap();
    assert_refused_with_dpop_challenge(&mcp(&world, &format!("Bearer {access}"), &[proof]).await);

    // A proof is good once.
    let proof = A
        .proof("POST", &world.resource_url("/mcp"), Some(&access))
        .unwrap();
    let first = mcp(&world, &format!("DPoP {access}"), slice::from_ref(&proof)).await;
    assert_eq!(first.status(), 200, "{}", first.body_text());
    assert_refused_with_dpop_challenge(&mcp(&world, &format!("DPoP {access}"), &[proof]).await);

    // `GET /mcp/tools` checks the same way, for its own method and URL.
    let tools_proof = A
        .proof("GET", &world.resource_url("/mcp/tools"), Some(&access))
        .unwrap();
    let tools = world
        .dialed(AxumTestRequest::get("/mcp/tools"))
        .header("authorization", &format!("DPoP {access}"))
        .header("dpop", &tools_proof)
        .send(app(&world.resources))
        .await;
    assert_eq!(tools.status(), 200, "{}", tools.body_text());
    let posted_proof = A
        .proof("POST", &world.resource_url("/mcp/tools"), Some(&access))
        .unwrap();
    let wrong_method = world
        .dialed(AxumTestRequest::get("/mcp/tools"))
        .header("authorization", &format!("DPoP {access}"))
        .header("dpop", &posted_proof)
        .send(app(&world.resources))
        .await;
    assert_eq!(wrong_method.status(), 401, "{}", wrong_method.body_text());
}

/// Every proof that does not fit the request, the token or the bound key is
/// refused at `/mcp`: another key's, another token's (`ath`), another URL's
/// (`htu`), another method's (`htm`), a stale one, and two at once.
#[tokio::test]
async fn a_proof_that_does_not_fit_the_request_the_token_or_the_key_is_refused() {
    let world = world().await;
    let client = register(&world).await;
    let (access, _) = bound_tokens(&world, &client, A).await;
    let mcp_url = world.resource_url("/mcp");
    let dpop = format!("DPoP {access}");

    // A valid proof, signed by a key the token is not bound to.
    assert_refused_with_dpop_challenge(&mcp_with(&world, &access, B).await);
    // Bound to another token.
    let other_ath = A
        .proof("POST", &mcp_url, Some("another-access-token"))
        .unwrap();
    assert_refused_with_dpop_challenge(&mcp(&world, &dpop, &[other_ath]).await);
    // Made for no token at all.
    let no_ath = A.proof("POST", &mcp_url, None).unwrap();
    assert_refused_with_dpop_challenge(&mcp(&world, &dpop, &[no_ath]).await);
    // For another URL.
    let elsewhere = A
        .proof("POST", "https://elsewhere.example.test/mcp", Some(&access))
        .unwrap();
    assert_refused_with_dpop_challenge(&mcp(&world, &dpop, &[elsewhere]).await);
    // For another method.
    let get = A.proof("GET", &mcp_url, Some(&access)).unwrap();
    assert_refused_with_dpop_challenge(&mcp(&world, &dpop, &[get]).await);
    // Made ten minutes ago.
    let stale = A.proof_aged("POST", &mcp_url, Some(&access), 600).unwrap();
    assert_refused_with_dpop_challenge(&mcp(&world, &dpop, &[stale]).await);
    // Two proofs (RFC 9449 §4.3).
    let one = A.proof("POST", &mcp_url, Some(&access)).unwrap();
    let two = A.proof("POST", &mcp_url, Some(&access)).unwrap();
    assert_refused_with_dpop_challenge(&mcp(&world, &dpop, &[one, two]).await);
    // None at all.
    assert_refused_with_dpop_challenge(&mcp(&world, &dpop, &[]).await);

    // A dialed host that is none of the server's origins: no htu can match.
    let proof = A.proof("POST", &mcp_url, Some(&access)).unwrap();
    let forged = AxumTestRequest::post("/mcp")
        .header("host", "elsewhere.example.test")
        .header("authorization", &dpop)
        .header("dpop", &proof)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}))
        .send(app(&world.resources))
        .await;
    assert_eq!(forged.status(), 401, "{}", forged.body_text());

    // And the right proof still works.
    assert_eq!(
        mcp_with(&world, &access, A).await.status(),
        200,
        "bug: the bound token with a proof by its key is admitted"
    );
}

/// A bound code is redeemed only with a proof by its key, and a refused
/// exchange leaves it unspent; the token endpoint refuses a replayed proof
/// and two proofs.
#[tokio::test]
async fn a_bound_code_is_redeemed_only_with_its_key() {
    let world = world().await;
    let client = register(&world).await;
    let code = bound_code(&world, &client, A).await;
    let token_url = world.issuer_url("/oauth2/token");

    assert_proof_refused(&exchange(&world, &client, &code, &[]).await);
    let other_key = B.proof("POST", &token_url, None).unwrap();
    assert_proof_refused(&exchange(&world, &client, &code, &[other_key]).await);
    let wrong_url = A
        .proof("POST", &world.issuer_url("/oauth2/par"), None)
        .unwrap();
    assert_proof_refused(&exchange(&world, &client, &code, &[wrong_url]).await);
    let one = A.proof("POST", &token_url, None).unwrap();
    let two = A.proof("POST", &token_url, None).unwrap();
    assert_proof_refused(&exchange(&world, &client, &code, &[one, two]).await);

    let proof = A.proof("POST", &token_url, None).unwrap();
    let (_, _, body) = issued(&exchange(&world, &client, &code, slice::from_ref(&proof)).await);
    assert_eq!(body["token_type"], "DPoP", "{body}");

    // The same proof again is a replay, whatever it is sent with.
    let second_code = bound_code(&world, &client, A).await;
    assert_proof_refused(&exchange(&world, &client, &second_code, &[proof]).await);
}

/// A refresh family bound to a key rotates only with a proof by it; its
/// successor stays bound, and validate-and-refresh, which carries no proof,
/// refuses it without rotating it away.
#[tokio::test]
async fn a_bound_refresh_token_rotates_only_with_its_key() {
    let world = world().await;
    let client = register(&world).await;
    let (access, refresh_token) = bound_tokens(&world, &client, A).await;
    let token_url = world.issuer_url("/oauth2/token");

    assert_proof_refused(&refresh(&world, &client, &refresh_token, &[]).await);
    let other_key = B.proof("POST", &token_url, None).unwrap();
    assert_proof_refused(&refresh(&world, &client, &refresh_token, &[other_key]).await);

    // validate-and-refresh, with the token expired, does not rotate it.
    let stale = expired(&world.resources, &access);
    let verdict = AxumTestRequest::post("/oauth2/validate-and-refresh")
        .header("authorization", &format!("Bearer {stale}"))
        .json(&json!({ "refresh_token": refresh_token }))
        .send(app(&world.resources))
        .await;
    let verdict: Value = serde_json::from_str(&verdict.body_text()).unwrap();
    assert_eq!(verdict["status"], "invalid", "{verdict}");
    assert_eq!(verdict["reason"], "refresh_at_token_endpoint", "{verdict}");

    let proof = A.proof("POST", &token_url, None).unwrap();
    let (rotated_access, rotated_refresh, body) =
        issued(&refresh(&world, &client, &refresh_token, &[proof]).await);
    assert_eq!(body["token_type"], "DPoP", "{body}");
    let claims = insecure_decode::<Claims>(&rotated_access).unwrap().claims;
    assert_eq!(
        claims.cnf.map(|cnf| cnf.jkt),
        Some(A.thumbprint().unwrap()),
        "bug: the access token is bound to the key the proof was signed with"
    );

    // The successor is bound too.
    assert_proof_refused(&refresh(&world, &client, &rotated_refresh, &[]).await);
}

/// A client that sends no proof keeps exactly what it had: Bearer tokens
/// `/mcp` admits as Bearer. The `DPoP` scheme on such a token is refused.
#[tokio::test]
async fn an_unbound_client_still_gets_bearer_tokens_the_mcp_door_admits() {
    let world = world().await;
    let client = register(&world).await;
    let code = unbound_code(&world, &client).await;
    let (access, refresh_token, body) = issued(&exchange(&world, &client, &code, &[]).await);
    assert_eq!(body["token_type"], "Bearer", "{body}");
    let claims = insecure_decode::<Claims>(&access).unwrap().claims;
    assert!(claims.cnf.is_none(), "an unbound token carries no cnf");

    let admitted = mcp(&world, &format!("Bearer {access}"), &[]).await;
    assert_eq!(admitted.status(), 200, "{}", admitted.body_text());
    assert_refused_with_dpop_challenge(&mcp_with(&world, &access, A).await);

    let (_, _, refreshed) = issued(&refresh(&world, &client, &refresh_token, &[]).await);
    assert_eq!(refreshed["token_type"], "Bearer", "{refreshed}");
}

/// A2A admits a bound token under the `DPoP` scheme with a proof for its own
/// URL, and refuses it as Bearer.
#[tokio::test]
async fn a_bound_token_calls_a2a_with_its_proof() {
    let world = world().await;
    let client = register(&world).await;
    let (access, _) = bound_tokens(&world, &client, A).await;
    let call = |authorization: String, proof: Option<String>| {
        let mut request = world
            .dialed(AxumTestRequest::post("/a2a/jsonrpc"))
            .header("A2A-Version", "1.0")
            .header("authorization", &authorization)
            .json(&json!({"jsonrpc": "2.0", "method": "GetExtendedAgentCard", "id": 1}));
        if let Some(proof) = proof {
            request = request.header("dpop", &proof);
        }
        request.send(app(&world.resources))
    };

    let proof = A
        .proof("POST", &world.resource_url("/a2a/jsonrpc"), Some(&access))
        .unwrap();
    let admitted = call(format!("DPoP {access}"), Some(proof)).await;
    assert_eq!(admitted.status(), 200, "{}", admitted.body_text());
    let body: Value = serde_json::from_str(&admitted.body_text()).unwrap();
    assert!(body.get("error").is_none(), "{body}");

    let bearer = call(format!("Bearer {access}"), None).await;
    assert_eq!(bearer.status(), 401, "{}", bearer.body_text());
    let challenge = bearer.header("www-authenticate").unwrap_or_default();
    assert!(
        challenge.starts_with("DPoP error=\"invalid_token\""),
        "a bound token presented as Bearer is named invalid_token (RFC 9449 §7.1): {challenge}"
    );
    let other_key = B
        .proof("POST", &world.resource_url("/a2a/jsonrpc"), Some(&access))
        .unwrap();
    let refused = call(format!("DPoP {access}"), Some(other_key)).await;
    assert_eq!(refused.status(), 401, "{}", refused.body_text());
    let challenge = refused.header("www-authenticate").unwrap_or_default();
    assert!(
        challenge.starts_with("DPoP error=\"invalid_dpop_proof\""),
        "a proof by another key is named invalid_dpop_proof, as /mcp names it: {challenge}"
    );
}

/// PAR takes `dpop_jkt` (RFC 9449 §10) as a parameter, and a proof and a
/// parameter naming two keys are refused; a code bound by the parameter alone
/// needs that key at the token endpoint.
#[tokio::test]
async fn pushed_dpop_jkt_binds_the_code_and_must_name_the_proofs_key() {
    let world = world().await;
    let client = register(&world).await;
    let par_url = world.issuer_url("/oauth2/par");

    let mismatched = push_with(
        &world,
        &client,
        &[A.proof("POST", &par_url, None).unwrap()],
        &[("dpop_jkt", &B.thumbprint().unwrap())],
    )
    .await;
    assert_proof_refused(&mismatched);

    let malformed = push_with(&world, &client, &[], &[("dpop_jkt", "not-a-thumbprint")]).await;
    assert_eq!(malformed.status(), 400, "{}", malformed.body_text());

    let pushed = push_with(
        &world,
        &client,
        &[],
        &[("dpop_jkt", &B.thumbprint().unwrap())],
    )
    .await;
    assert_eq!(pushed.status(), 201, "{}", pushed.body_text());
    let body: Value = serde_json::from_str(&pushed.body_text()).unwrap();
    let code = approve(&world, &client, body["request_uri"].as_str().unwrap()).await;
    let token_url = world.issuer_url("/oauth2/token");
    assert_proof_refused(
        &exchange(
            &world,
            &client,
            &code,
            &[A.proof("POST", &token_url, None).unwrap()],
        )
        .await,
    );
    let (access, _, _) = issued(
        &exchange(
            &world,
            &client,
            &code,
            &[B.proof("POST", &token_url, None).unwrap()],
        )
        .await,
    );
    let claims = insecure_decode::<Claims>(&access).unwrap().claims;
    assert_eq!(
        claims.cnf.map(|cnf| cnf.jkt),
        Some(B.thumbprint().unwrap()),
        "bug: the token is not bound to the key the code was bound to"
    );
}

/// `dpop_jkt` sent in the authorization query (RFC 9449 §10) crosses the
/// consent screen as a hidden field and binds the code: redeemed without a
/// proof by that key it is refused, with one it mints a `DPoP` token.
#[tokio::test]
async fn a_query_dpop_jkt_crosses_the_consent_screen_and_binds_the_code() {
    let world = world().await;
    let client = register(&world).await;
    let challenge = challenge();
    let state = format!("state-{}", Uuid::new_v4().simple());
    let jkt = A.thumbprint().unwrap();
    let query = authorize_query(&[
        ("response_type", "code"),
        ("client_id", client.id.as_str()),
        ("redirect_uri", REDIRECT),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("dpop_jkt", jkt.as_str()),
    ]);
    let page = consent_page(app(&world.resources), &world.cookie, &query).await;
    let carried = hidden_field(&page, "dpop_jkt");
    assert_eq!(
        carried.as_deref(),
        Some(jkt.as_str()),
        "the consent form carries dpop_jkt"
    );
    let (digest_field, digest) = review_digest(&page);
    let csrf = world
        .resources
        .auth
        .csrf_manager
        .generate_token(world.user.id)
        .unwrap();
    let approved = AxumTestRequest::post("/oauth2/consent")
        .header("cookie", &world.cookie)
        .form(&[
            ("response_type", "code"),
            ("client_id", client.id.as_str()),
            ("redirect_uri", REDIRECT),
            ("state", state.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("dpop_jkt", carried.as_deref().unwrap_or_default()),
            ("decision", "approve"),
            ("csrf_token", csrf.as_str()),
            (digest_field, digest.as_str()),
        ])
        .send(app(&world.resources))
        .await;
    assert_eq!(approved.status(), 303, "{}", approved.body_text());
    let code = code_in(approved.header("location").unwrap());

    let token_url = world.issuer_url("/oauth2/token");
    assert_proof_refused(&exchange(&world, &client, &code, &[]).await);
    let proof = A.proof("POST", &token_url, None).unwrap();
    let (_, _, body) = issued(&exchange(&world, &client, &code, &[proof]).await);
    assert_eq!(body["token_type"], "DPoP", "{body}");
}

/// `token` with its claims unchanged but for an expiry an hour past,
/// re-signed with the server's active key.
fn expired(resources: &Arc<ServerContext>, token: &str) -> String {
    let mut claims = insecure_decode::<Claims>(token).unwrap().claims;
    claims.iat -= 7200;
    claims.exp = Timestamp::now().epoch_seconds() - 3600;
    let key = resources.auth.jwks_manager.get_active_key().unwrap();
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(key.kid.clone());
    encode(&header, &claims, &key.encoding_key().unwrap()).unwrap()
}
