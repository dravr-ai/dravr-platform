// ABOUTME: An athlete whose self-registration lands active is signed straight in with a first-party code (carnet#1147)
// ABOUTME: Pins the code's redemption, every case that mints none, and the unchanged response when the app asks for nothing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `POST /api/auth/register` accepts the authorization request the app would
//! have sent `/oauth2/authorize` (`sign_in`). When the account comes out
//! active it answers with `sign_in_code`: a code minted by the same
//! authorization server, bound to the app's PKCE challenge and callback,
//! which the app redeems at `/oauth/token`. Every other outcome — an account
//! that waits on review or on its address, a request the authorization server
//! refuses, a client that is not Dravr's own — still registers, and carries
//! no code.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

use crate::common;
use crate::helpers;

use common::{create_test_server_resources_with_config, test_server_config};
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use helpers::first_party_sign_in::{
    first_party_router, fresh_state, redeem, FirstPartyClient, Pkce, DEFAULT_PEER,
};
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::first_party::WEB_CLIENT_ID;
use pierre_auth::oauth2_server::models::ClientRegistrationRequest;
use pierre_auth::security::cookies::auth_cookie_name;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::models::UserStatus;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_services::signup_codes::{self, MintRequest};
use pierre_services::signup_policy::{set_signup_policy, SignupPolicyUpdate};
use serde_json::{json, Value};
use uuid::Uuid;

const PASSWORD: &str = "securePassword123";

/// A server whose self-registrations wait on review unless the address was
/// pre-approved, so an active outcome here is the allow-list's doing.
async fn setup() -> Arc<ServerContext> {
    let mut config = test_server_config();
    config.app_behavior.ci_mode = true;
    config.app_behavior.auto_approve_users = false;
    create_test_server_resources_with_config(config)
        .await
        .unwrap()
}

/// A fresh address, pre-approved so its registration lands active.
async fn preapproved(resources: &ServerContext) -> String {
    let email = unique_email();
    resources
        .common
        .repos
        .pre_approved_emails
        .allow(&email, None, Some("register sign-in"))
        .await
        .unwrap();
    email
}

fn unique_email() -> String {
    format!("register-sign-in-{}@example.com", Uuid::new_v4())
}

/// A new client address per registration, so the per-address sign-up window
/// never decides a test.
fn fresh_peer() -> [u8; 4] {
    static NEXT: AtomicU16 = AtomicU16::new(1);
    let [hi, lo] = NEXT.fetch_add(1, Ordering::Relaxed).to_be_bytes();
    [198, 19, hi, lo]
}

/// The `sign_in` block the app attaches, for `client` at its configured
/// callback, carrying `pkce`'s S256 challenge.
fn sign_in_for(resources: &ServerContext, client: FirstPartyClient, pkce: &Pkce) -> Value {
    json!({
        "client_id": client.client_id(),
        "redirect_uri": client.redirect_uri(&resources.common.config.oauth2_server),
        "code_challenge": pkce.challenge,
        "code_challenge_method": "S256",
        "state": fresh_state(),
    })
}

/// `POST /api/auth/register` with `extra` merged into the account's body.
async fn register(resources: &ServerContext, email: &str, extra: Value) -> (u16, Value) {
    let mut body = json!({
        "email": email,
        "password": PASSWORD,
        "display_name": "Register Sign-In",
    });
    if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            body.insert(key.clone(), value.clone());
        }
    }
    let response = AxumTestRequest::post("/api/auth/register")
        .json(&body)
        .send(first_party_router(resources, fresh_peer()))
        .await;
    let status = response.status();
    (status, response.json())
}

/// `POST /api/auth/register` carrying `sign_in`.
async fn register_signing_in(
    resources: &ServerContext,
    email: &str,
    sign_in: Value,
) -> (u16, Value) {
    register(resources, email, json!({ "sign_in": sign_in })).await
}

/// Redeem `code` as `client` at its configured callback.
async fn redeem_as(
    resources: &ServerContext,
    client: FirstPartyClient,
    code: &str,
    verifier: &str,
    scope: Option<&str>,
) -> AxumTestResponse {
    redeem(
        first_party_router(resources, DEFAULT_PEER),
        client.client_id(),
        code,
        &client.redirect_uri(&resources.common.config.oauth2_server),
        Some(verifier),
        scope,
    )
    .await
}

fn code_of(body: &Value) -> String {
    body["sign_in_code"]
        .as_str()
        .unwrap_or_else(|| panic!("an active registration carries a sign-in code: {body}"))
        .to_owned()
}

fn assert_no_code(body: &Value) {
    assert!(
        body.get("sign_in_code").is_none(),
        "no sign-in code may be issued here: {body}"
    );
}

fn oauth_error(response: AxumTestResponse) -> (u16, String) {
    let status = response.status();
    let body: Value = response.json();
    (
        status,
        body["error"].as_str().unwrap_or_default().to_owned(),
    )
}

async fn status_of(resources: &ServerContext, email: &str) -> UserStatus {
    resources
        .common
        .repos
        .users
        .get_by_email(email)
        .await
        .unwrap()
        .expect("the account was created")
        .user_status
}

/// The keys of a registration answer with no code: exactly the ones the
/// endpoint answered with before it could sign anyone in.
fn registration_keys() -> BTreeSet<String> {
    [
        "user_id",
        "email",
        "user_status",
        "display_name",
        "activation",
        "message",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn keys_of(body: &Value) -> BTreeSet<String> {
    body.as_object()
        .expect("a JSON object")
        .keys()
        .cloned()
        .collect()
}

// ── The code an active registration earns ────────────────────────────────────

/// The web app registering a pre-approved address is answered with a code
/// that buys the same session the hosted login does: cookies, the CSRF
/// token and the user.
#[tokio::test]
async fn an_active_web_registration_is_signed_in_with_its_code() {
    let resources = setup().await;
    let email = preapproved(&resources).await;
    let pkce = Pkce::generate();

    let (status, body) = register_signing_in(
        &resources,
        &email,
        sign_in_for(&resources, FirstPartyClient::Web, &pkce),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["activation"], "active", "{body}");
    let code = code_of(&body);

    let response = redeem_as(
        &resources,
        FirstPartyClient::Web,
        &code,
        &pkce.verifier,
        None,
    )
    .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let cookies = response.header_all("set-cookie").join("\n");
    assert!(
        cookies.contains(&format!("{}=", auth_cookie_name())),
        "{cookies}"
    );
    assert!(cookies.contains("csrf_token="), "{cookies}");
    let session: Value = response.json();
    assert!(session["access_token"].is_string(), "{session}");
    assert!(session["csrf_token"].is_string(), "{session}");
    assert_eq!(session["user"]["email"], email.as_str(), "{session}");
    assert_eq!(session["user"]["user_status"], "active", "{session}");
    assert!(session.get("refresh_token").is_none(), "{session}");
}

/// The mobile app redeems its code at its own callback and, asking for
/// `offline_access` at the token endpoint as it does after the hosted login,
/// gets a refresh token.
#[tokio::test]
async fn an_active_mobile_registration_gets_a_refresh_token_with_offline_access() {
    let resources = setup().await;
    let email = preapproved(&resources).await;
    let pkce = Pkce::generate();

    let (status, body) = register_signing_in(
        &resources,
        &email,
        sign_in_for(&resources, FirstPartyClient::Mobile, &pkce),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let code = code_of(&body);

    let response = redeem_as(
        &resources,
        FirstPartyClient::Mobile,
        &code,
        &pkce.verifier,
        Some("offline_access"),
    )
    .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let session: Value = response.json();
    assert!(session["access_token"].is_string(), "{session}");
    assert!(session["refresh_token"].is_string(), "{session}");
    assert_eq!(session["user"]["email"], email.as_str(), "{session}");
}

/// The code is the PKCE-bound, single-use code of any authorization: the
/// wrong verifier is refused, and a second redemption of a spent code too.
#[tokio::test]
async fn the_code_needs_the_verifier_and_redeems_once() {
    let resources = setup().await;

    let pkce = Pkce::generate();
    let (_, body) = register_signing_in(
        &resources,
        &preapproved(&resources).await,
        sign_in_for(&resources, FirstPartyClient::Web, &pkce),
    )
    .await;
    let code = code_of(&body);
    let stranger = Pkce::generate();
    assert_eq!(
        oauth_error(
            redeem_as(
                &resources,
                FirstPartyClient::Web,
                &code,
                &stranger.verifier,
                None
            )
            .await
        ),
        (400, "invalid_grant".to_owned()),
        "a verifier that does not match the challenge must not redeem the code"
    );

    let pkce = Pkce::generate();
    let (_, body) = register_signing_in(
        &resources,
        &preapproved(&resources).await,
        sign_in_for(&resources, FirstPartyClient::Web, &pkce),
    )
    .await;
    let code = code_of(&body);
    let first = redeem_as(
        &resources,
        FirstPartyClient::Web,
        &code,
        &pkce.verifier,
        None,
    )
    .await;
    assert_eq!(first.status(), 200, "{}", first.body_text());
    assert_eq!(
        oauth_error(
            redeem_as(
                &resources,
                FirstPartyClient::Web,
                &code,
                &pkce.verifier,
                None
            )
            .await
        ),
        (400, "invalid_grant".to_owned()),
        "a spent code must not redeem a second time"
    );
}

// ── Registrations that earn no code ──────────────────────────────────────────

/// An account that waits on an operator's review is not signed in, whatever
/// the app asked; its answer has today's keys and no more.
#[tokio::test]
async fn a_registration_awaiting_review_gets_no_code() {
    let resources = setup().await;
    let email = unique_email();

    let (status, body) = register_signing_in(
        &resources,
        &email,
        sign_in_for(&resources, FirstPartyClient::Web, &Pkce::generate()),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["activation"], "review", "{body}");
    assert_no_code(&body);
    assert_eq!(keys_of(&body), registration_keys(), "{body}");
    assert_eq!(
        status_of(&resources, &email).await,
        UserStatus::Pending,
        "an address no rule admitted waits on review"
    );
}

/// An account the confirmation link will activate is not signed in: not
/// under the verified-address policy, and not with a signup code, which
/// activates only a proven address.
#[tokio::test]
async fn a_registration_awaiting_its_address_gets_no_code() {
    let resources = setup().await;
    set_signup_policy(
        &resources.agent.database,
        SignupPolicyUpdate {
            activate_on_verified_email: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let (status, body) = register_signing_in(
        &resources,
        &unique_email(),
        sign_in_for(&resources, FirstPartyClient::Web, &Pkce::generate()),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["activation"], "confirm_email", "{body}");
    assert_no_code(&body);

    let resources = setup().await;
    let code = signup_codes::mint(&resources.common.repos, None, &MintRequest::default())
        .await
        .unwrap()
        .code;
    let (status, body) = register(
        &resources,
        &unique_email(),
        json!({
            "signup_code": code,
            "sign_in": sign_in_for(&resources, FirstPartyClient::Web, &Pkce::generate()),
        }),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["activation"], "confirm_email", "{body}");
    assert_no_code(&body);
}

/// A sign-in request the authorization server refuses — a callback off the
/// deployment's allowlist, `plain` PKCE, a challenge too short to be S256 —
/// still registers the account, active, and answers without a code.
#[tokio::test]
async fn a_refused_sign_in_request_still_registers_without_a_code() {
    let resources = setup().await;
    let pkce = Pkce::generate();
    let mut off_allowlist = sign_in_for(&resources, FirstPartyClient::Web, &pkce);
    off_allowlist["redirect_uri"] = json!("https://attacker.example.com/auth/callback");
    let mut plain = sign_in_for(&resources, FirstPartyClient::Web, &pkce);
    plain["code_challenge"] = json!(pkce.verifier);
    plain["code_challenge_method"] = json!("plain");
    let mut short = sign_in_for(&resources, FirstPartyClient::Web, &pkce);
    short["code_challenge"] = json!("abcdefghijklmnopqrst");

    for (case, sign_in) in [
        ("off-allowlist redirect", off_allowlist),
        ("plain PKCE", plain),
        ("20-character challenge", short),
    ] {
        let email = preapproved(&resources).await;
        let (status, body) = register_signing_in(&resources, &email, sign_in).await;
        assert_eq!(status, 201, "{case}: {body}");
        assert_eq!(body["activation"], "active", "{case}: {body}");
        assert_no_code(&body);
        assert_eq!(
            status_of(&resources, &email).await,
            UserStatus::Active,
            "{case}"
        );
    }
}

/// Without `sign_in` an active registration answers exactly as it did before
/// the app could be signed in.
#[tokio::test]
async fn an_active_registration_without_sign_in_answers_as_before() {
    let resources = setup().await;
    let email = preapproved(&resources).await;

    let (status, body) = register(&resources, &email, json!({})).await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["activation"], "active", "{body}");
    assert_eq!(keys_of(&body), registration_keys(), "{body}");
}

/// Only Dravr's own apps skip consent, so a registered integration naming
/// itself in `sign_in` at its own registered callback gets no code.
#[tokio::test]
async fn a_third_party_client_gets_no_code() {
    let resources = setup().await;
    let redirect_uri = "https://integration.example.com/callback";
    let client_id = ClientRegistrationManager::new(resources.common.repos.oauth2_server.clone())
        .register_client(
            ClientRegistrationRequest {
                redirect_uris: vec![redirect_uri.to_owned()],
                client_name: Some("An integration".to_owned()),
                client_uri: None,
                grant_types: None,
                response_types: None,
                scope: None,
            },
            MAX_PENDING_REGISTRATIONS,
        )
        .await
        .unwrap()
        .client_id;
    assert_ne!(
        client_id, WEB_CLIENT_ID,
        "a registered integration is issued its own client id"
    );

    let pkce = Pkce::generate();
    let email = preapproved(&resources).await;
    let (status, body) = register_signing_in(
        &resources,
        &email,
        json!({
            "client_id": client_id,
            "redirect_uri": redirect_uri,
            "code_challenge": pkce.challenge,
            "code_challenge_method": "S256",
            "state": fresh_state(),
        }),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["activation"], "active", "{body}");
    assert_no_code(&body);
}
