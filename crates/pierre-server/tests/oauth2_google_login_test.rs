// ABOUTME: "Continue with Google" on the hosted OAuth login page goes straight to Google and back to consent (carnet#652)
// ABOUTME: Drives start, callback, code exchange and ID-token checks against a local Google, and the account rules behind them
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! An athlete who signs in with Google could not authorize an MCP connector:
//! the hosted `/oauth2/login` page took only a password. The authorization
//! server now signs athletes in with Google itself — the start sends them to
//! Google's account chooser, the callback exchanges the code over the back
//! channel, checks the ID token and returns to consent — through the same
//! account rules the web app's Firebase sign-in follows.
//!
//! Google is a local listener here: it serves the token endpoint and the
//! signing keys, and a test mints the ID token with the nonce the start put in
//! the redirect to the chooser.

mod common;
mod helpers;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use axum::http::header::COOKIE;
use axum::http::HeaderMap;
use base64::{engine::general_purpose, Engine as _};
use chrono::{Duration, Utc};
use common::{create_test_server_resources_with_config, create_test_user_with_email};
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use helpers::google_oidc::{
    GoogleIdClaims, GoogleStub, GOOGLE_CHOOSER, GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET,
};
use helpers::google_token::{now_secs, unreachable_key_set_url, TestSigner};
use helpers::notify_capture::{capture_logs, capture_notify, named, only};
use pierre_auth::config::{GoogleSignInConfig, OAuth2ServerConfig};
use pierre_auth::dto::auth::LoginRequest;
use pierre_auth::google_oidc::google_callback_url;
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::models::ClientRegistrationRequest;
use pierre_auth::oauth2_server::rate_limiting::OAuth2RateLimiter;
use pierre_auth::security::cookies::auth_cookie_name;
use pierre_config::environment::ServerConfig;
use pierre_core::constants::oauth2_client_retention::MAX_PENDING_REGISTRATIONS;
use pierre_core::errors::ErrorCode;
use pierre_core::models::{
    ApiKey, ApiKeyTier, CreateUserMcpTokenRequest, OAuth2RefreshToken, User, UserStatus,
    FEDERATED_ONLY_PASSWORD_HASH,
};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_identity::oauth2::{OAuth2Context, OAuth2Routes};
use pierre_services::auth::{FederatedIdentity, SignupSource};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

/// The authorization server's host.
const ISSUER: &str = "https://app.example.test";
/// The MCP resource server's host — not the issuer's.
const MCP_RESOURCE: &str = "https://mcp.example.test";
const REDIRECT: &str = "https://client.example.test/callback";
const STATE: &str = "google-login-client-state";
const VERIFIER: &str = "google-login-pkce-verifier-0123456789-abcdefghijklmnopqrstuv";
const CLIENT_NAME: &str = "Google login client";
/// The Set-Cookie name of the session on an HTTPS issuer.
const SESSION: &str = "__Host-pierre_session";
/// The Set-Cookie name of the sign-in transaction on an HTTPS issuer.
const TRANSACTION: &str = "__Host-pierre_google_sign_in";
/// What the page says to a sign-in refused for its account.
const REFUSED_PAGE: &str = "Google sign-in could not be completed for this account.";
/// A string no page or log line may ever carry: the token endpoint's body.
const SENTINEL: &str = "sentinel-google-body-9f2c";

/// One deployment with Google sign-in pointed at a local Google.
struct Fixture {
    resources: Arc<ServerContext>,
    google: GoogleStub,
    signer: TestSigner,
    client_id: String,
}

impl Fixture {
    async fn new() -> Self {
        Self::with(ISSUER, |config| config).await
    }

    /// A fixture whose Google configuration `adjust` may rewrite.
    async fn with(
        issuer: &str,
        adjust: impl FnOnce(GoogleSignInConfig) -> GoogleSignInConfig,
    ) -> Self {
        let signer = TestSigner::generate();
        let google = GoogleStub::serve(vec![signer.jwk()]).await;
        let oauth2_server = OAuth2ServerConfig {
            issuer_url: issuer.to_owned(),
            mcp_resource_url: MCP_RESOURCE.to_owned(),
            google_sign_in: Some(Box::new(adjust(google.config()))),
            ..OAuth2ServerConfig::default()
        };
        let resources = resources_with(oauth2_server).await;
        let client_id = register(&resources, vec![REDIRECT.to_owned()]).await;
        Self {
            resources,
            google,
            signer,
            client_id,
        }
    }

    fn routes(&self) -> axum::Router {
        oauth2_routes(&self.resources)
    }

    /// `GET /oauth2/login/google` for this fixture's client.
    async fn start(&self) -> Started {
        started(
            &AxumTestRequest::get(&format!(
                "/oauth2/login/google?{}",
                authorize_query(&self.client_id)
            ))
            .send(self.routes())
            .await,
        )
    }

    /// The callback for `started`, carrying its cookie, its state and `code`.
    async fn callback(&self, started: &Started, code: &str) -> AxumTestResponse {
        self.callback_with(
            Some(&started.cookie),
            &format!("state={}&code={code}", urlencoding::encode(&started.state)),
        )
        .await
    }

    async fn callback_with(&self, cookie: Option<&str>, query: &str) -> AxumTestResponse {
        let mut request = AxumTestRequest::get(&format!("/oauth2/login/google/callback?{query}"));
        if let Some(cookie) = cookie {
            request = request.header("cookie", cookie);
        }
        request.send(self.routes()).await
    }

    /// A whole sign-in: start, Google answering with the token `claims`
    /// builds from the start's nonce, and the callback.
    async fn sign_in(&self, claims: impl FnOnce(&str) -> GoogleIdClaims) -> AxumTestResponse {
        let started = self.start().await;
        self.google
            .set_id_token(&self.signer.mint(&claims(&started.nonce)));
        self.callback(&started, &fresh_code()).await
    }

    async fn allow(&self, email: &str) {
        self.resources
            .common
            .repos
            .pre_approved_emails
            .allow(email, None, None)
            .await
            .unwrap();
    }

    async fn user(&self, email: &str) -> Option<User> {
        self.resources
            .common
            .repos
            .users
            .get_by_email(email)
            .await
            .unwrap()
    }

    async fn verified(&self, user_id: Uuid) -> bool {
        self.resources
            .common
            .repos
            .email_verification
            .is_verified(user_id)
            .await
            .unwrap()
    }

    async fn linked_subject(&self, user_id: Uuid) -> Option<String> {
        self.resources
            .common
            .repos
            .federated_identities
            .subject_for_user(user_id, "google.com")
            .await
            .unwrap()
    }

    /// An existing account for `email`, active, with its address verified
    /// when `verified`.
    async fn account(&self, email: &str, verified: bool) -> User {
        let (_, user) = create_test_user_with_email(&self.resources.agent.database, email)
            .await
            .unwrap();
        if verified {
            self.resources
                .common
                .repos
                .email_verification
                .mark_verified(user.id)
                .await
                .unwrap();
        }
        user
    }
}

/// What the start answered: the redirect to Google and the cookie.
struct Started {
    location: Url,
    state: String,
    nonce: String,
    /// `name=value` of the transaction cookie, as a browser sends it back
    cookie: String,
    /// The full `Set-Cookie` value
    set_cookie: String,
}

fn started(response: &AxumTestResponse) -> Started {
    assert_eq!(response.status(), 303, "{}", response.body_text());
    let location = response.header("location").expect("a redirect to Google");
    assert!(location.starts_with(GOOGLE_CHOOSER), "{location}");
    let location = Url::parse(location).unwrap();
    let set_cookie = response
        .header("set-cookie")
        .expect("the transaction cookie")
        .to_owned();
    let cookie = set_cookie.split(';').next().unwrap().to_owned();
    Started {
        state: query_param(&location, "state").unwrap(),
        nonce: query_param(&location, "nonce").unwrap(),
        location,
        cookie,
        set_cookie,
    }
}

fn query_param(url: &Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// The attributes of a `Set-Cookie` value, name/value pair first.
fn cookie_parts(set_cookie: &str) -> (String, Vec<String>) {
    let mut parts = set_cookie.split(';').map(|p| p.trim().to_owned());
    let pair = parts.next().unwrap();
    let mut attributes: Vec<String> = parts.collect();
    attributes.sort();
    (pair, attributes)
}

/// The session cookie a response sets, if any: its JWT.
fn session(response: &AxumTestResponse) -> Option<String> {
    response
        .header_all("set-cookie")
        .into_iter()
        .find_map(|c| c.strip_prefix(&format!("{SESSION}=")))
        .map(|rest| rest.split(';').next().unwrap().to_owned())
        .filter(|jwt| !jwt.is_empty())
}

/// Whether a response clears the transaction cookie.
fn clears_transaction(response: &AxumTestResponse) -> bool {
    response
        .header_all("set-cookie")
        .iter()
        .any(|c| c.starts_with(&format!("{TRANSACTION}=;")) && c.contains("Max-Age=0"))
}

fn fresh_code() -> String {
    format!("4/0A-test-code-{}", Uuid::new_v4().simple())
}

fn pkce_challenge() -> String {
    general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()))
}

fn authorize_query(client_id: &str) -> String {
    format!(
        "response_type=code&client_id={}&redirect_uri={}&state={}&code_challenge={}&code_challenge_method=S256",
        urlencoding::encode(client_id),
        urlencoding::encode(REDIRECT),
        urlencoding::encode(STATE),
        pkce_challenge(),
    )
}

/// The `/oauth2/authorize` return the callback must rebuild for `client_id`.
fn expected_authorize(client_id: &str) -> String {
    format!(
        "/oauth2/authorize?client_id={}&redirect_uri={}&response_type=code&state={}&code_challenge={}&code_challenge_method=S256",
        urlencoding::encode(client_id),
        urlencoding::encode(REDIRECT),
        urlencoding::encode(STATE),
        pkce_challenge(),
    )
}

async fn resources_with(oauth2_server: OAuth2ServerConfig) -> Arc<ServerContext> {
    Box::pin(create_test_server_resources_with_config(ServerConfig {
        oauth2_server,
        activity_fetch_limit: 100,
        ..ServerConfig::default()
    }))
    .await
    .unwrap()
}

/// The OAuth routes as the server mounts them, Google sign-in included.
fn oauth2_routes(resources: &Arc<ServerContext>) -> axum::Router {
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
        accounts: resources.oauth2_accounts(),
        google_sign_in: resources.oauth2_google_sign_in(),
    };
    OAuth2Routes::routes(context).layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_652))))
}

async fn register(resources: &Arc<ServerContext>, redirect_uris: Vec<String>) -> String {
    ClientRegistrationManager::new(resources.common.repos.oauth2_server.clone())
        .register_client(
            ClientRegistrationRequest {
                redirect_uris,
                client_name: Some(CLIENT_NAME.to_owned()),
                client_uri: None,
                grant_types: None,
                response_types: None,
                scope: None,
            },
            MAX_PENDING_REGISTRATIONS,
        )
        .await
        .unwrap()
        .client_id
}

/// The page `/oauth2/authorize` answers the session `jwt` with.
async fn authorize_page(f: &Fixture, location: &str, jwt: &str) -> AxumTestResponse {
    AxumTestRequest::get(location)
        .header("cookie", &format!("{SESSION}={jwt}"))
        .send(f.routes())
        .await
}

fn refused_without_session(response: &AxumTestResponse) {
    assert!(
        response.status() >= 400,
        "refused: {} {}",
        response.status(),
        response.body_text()
    );
    assert!(response.header("location").is_none(), "no redirect");
    assert!(session(response).is_none(), "no session cookie");
}

// ── A: the whole flow, a new account ────────────────────────────────────────

#[tokio::test]
async fn google_sign_in_goes_to_google_and_back_to_consent_for_a_new_account() {
    let f = Fixture::new().await;
    f.allow("new.athlete@gmail.com").await;

    let started = f.start().await;
    let google = &started.location;
    let param = |name: &str| query_param(google, name);
    assert_eq!(param("response_type").as_deref(), Some("code"));
    assert_eq!(param("client_id").as_deref(), Some(GOOGLE_CLIENT_ID));
    assert_eq!(param("scope").as_deref(), Some("openid email profile"));
    assert_eq!(param("prompt").as_deref(), Some("select_account"));
    assert_eq!(param("code_challenge_method").as_deref(), Some("S256"));
    assert_eq!(
        param("redirect_uri").as_deref(),
        Some("https://app.example.test/oauth2/login/google/callback"),
        "Google returns to the issuer, never the MCP host"
    );
    assert_eq!(started.state.len(), 43);
    assert_eq!(started.nonce.len(), 43);
    assert_ne!(started.state, started.nonce);
    assert_ne!(started.state, STATE, "Google's state is not the client's");
    let (pair, attributes) = cookie_parts(&started.set_cookie);
    assert!(pair.starts_with(&format!("{TRANSACTION}=")), "{pair}");
    assert_eq!(
        attributes,
        [
            "HttpOnly",
            "Max-Age=600",
            "Path=/",
            "SameSite=Lax",
            "Secure"
        ]
    );

    let (events, guard) = capture_notify();
    let google_sub = "108000000000000000001";
    f.google
        .set_id_token(&f.signer.mint(&GoogleIdClaims::issued(
            google_sub,
            "New.Athlete@gmail.com",
            &started.nonce,
        )));
    let code = fresh_code();
    let callback = f.callback(&started, &code).await;
    drop(guard);

    assert_eq!(callback.status(), 303, "{}", callback.body_text());
    let location = callback.header("location").unwrap().to_owned();
    assert_eq!(location, expected_authorize(&f.client_id));
    assert_eq!(callback.header("cache-control"), Some("no-store"));
    assert!(clears_transaction(&callback));
    let set_session = callback
        .header_all("set-cookie")
        .into_iter()
        .find(|c| c.starts_with(&format!("{SESSION}=")))
        .unwrap()
        .to_owned();
    let (_, attributes) = cookie_parts(&set_session);
    assert_eq!(
        attributes,
        [
            "HttpOnly",
            "Max-Age=86400",
            "Path=/",
            "SameSite=Lax",
            "Secure"
        ]
    );
    let jwt = session(&callback).unwrap();

    // The code exchange Google received: client_secret_post plus the PKCE
    // verifier whose challenge went to the chooser.
    let requests = f.google.token_requests();
    assert_eq!(requests.len(), 1);
    let form = &requests[0];
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["code"], code);
    assert_eq!(form["client_id"], GOOGLE_CLIENT_ID);
    assert_eq!(form["client_secret"], GOOGLE_CLIENT_SECRET);
    assert_eq!(
        form["redirect_uri"],
        "https://app.example.test/oauth2/login/google/callback"
    );
    assert_eq!(
        general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(form["code_verifier"].as_bytes())),
        param("code_challenge").unwrap()
    );

    // The account: federated only, Google's, verified, in its own workspace.
    let user = f.user("new.athlete@gmail.com").await.unwrap();
    assert_eq!(user.email, "new.athlete@gmail.com");
    assert_eq!(
        user.firebase_uid, None,
        "a Google sub is not a Firebase UID"
    );
    assert_eq!(user.auth_provider, "google.com");
    assert_eq!(user.password_hash, FEDERATED_ONLY_PASSWORD_HASH);
    assert_eq!(user.display_name.as_deref(), Some("Google Athlete"));
    assert_eq!(user.user_status, UserStatus::Active);
    assert!(f.verified(user.id).await);
    assert_eq!(f.linked_subject(user.id).await.as_deref(), Some(google_sub));
    let tenants = f
        .resources
        .common
        .repos
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap();
    assert_eq!(tenants.len(), 1);
    let signed_up = only(&events, "user.signed_up");
    assert_eq!(signed_up.field("source"), "google");
    assert_eq!(signed_up.field("user_id"), user.id.to_string());
    assert_eq!(
        only(&events, "user.login").field("user_id"),
        user.id.to_string()
    );
    let claims = f
        .resources
        .auth
        .auth_manager
        .validate_authorization_session_token(&jwt, &f.resources.auth.jwks_manager)
        .unwrap();
    assert_eq!(claims.sub, user.id.to_string());
    assert!(
        f.resources
            .auth
            .auth_manager
            .validate_session_token(&jwt, &f.resources.auth.jwks_manager)
            .is_err(),
        "the Google sign-in cookie is no first-party session (carnet#787)"
    );

    // Consent, naming the client, where it returns, and the account.
    let consent = authorize_page(&f, &location, &jwt).await;
    assert_eq!(consent.status(), 200, "{}", consent.body_text());
    let page = consent.body_text();
    assert!(page.contains(CLIENT_NAME), "{page}");
    assert!(
        page.contains("It will return you to client.example.test"),
        "{page}"
    );
    assert!(
        page.contains("Signed in as new.athlete@gmail.com"),
        "{page}"
    );
    let csrf_token = page
        .split("name=\"csrf_token\" value=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap()
        .to_owned();
    let challenge = pkce_challenge();
    let approved = AxumTestRequest::post("/oauth2/consent")
        .header("cookie", &format!("{SESSION}={jwt}"))
        .form(&[
            ("response_type", "code"),
            ("client_id", f.client_id.as_str()),
            ("redirect_uri", REDIRECT),
            ("state", STATE),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("resource", ""),
            ("csrf_token", csrf_token.as_str()),
            ("decision", "approve"),
        ])
        .send(f.routes())
        .await;
    assert_eq!(approved.status(), 303, "{}", approved.body_text());
    let back = Url::parse(approved.header("location").unwrap()).unwrap();
    assert!(back.as_str().starts_with(REDIRECT));
    assert_eq!(query_param(&back, "state").as_deref(), Some(STATE));
    assert!(query_param(&back, "code").is_some_and(|c| !c.is_empty()));
}

// ── B, C, D: existing accounts ───────────────────────────────────────────────

#[tokio::test]
async fn a_verified_password_account_is_reached_and_keeps_its_password() {
    let f = Fixture::new().await;
    let existing = f.account("runner@gmail.com", true).await;

    let callback = f
        .sign_in(|nonce| GoogleIdClaims::issued("108000000000000000002", "runner@gmail.com", nonce))
        .await;

    assert_eq!(callback.status(), 303, "{}", callback.body_text());
    let user = f.user("runner@gmail.com").await.unwrap();
    assert_eq!(user.id, existing.id, "no second account");
    assert_eq!(user.password_hash, existing.password_hash);
    assert_eq!(
        f.linked_subject(user.id).await.as_deref(),
        Some("108000000000000000002")
    );
    f.resources
        .oauth2_accounts()
        .login(LoginRequest {
            email: "runner@gmail.com".to_owned(),
            password: "password123".to_owned(),
            timezone: None,
        })
        .await
        .expect("the password still signs in");
}

#[tokio::test]
async fn a_firebase_account_keeps_its_firebase_uid_and_both_paths_meet_on_the_google_sub() {
    let f = Fixture::new().await;
    let mut existing = f.account("fire@gmail.com", true).await;
    existing.firebase_uid = Some("fb-uid".to_owned());
    "google.com".clone_into(&mut existing.auth_provider);
    f.resources
        .common
        .repos
        .users
        .update(&existing)
        .await
        .unwrap();

    let callback = f
        .sign_in(|nonce| GoogleIdClaims::issued("108000000000000000003", "fire@gmail.com", nonce))
        .await;
    assert_eq!(callback.status(), 303, "{}", callback.body_text());
    let user = f.user("fire@gmail.com").await.unwrap();
    assert_eq!(user.id, existing.id);
    assert_eq!(
        user.firebase_uid.as_deref(),
        Some("fb-uid"),
        "left untouched"
    );

    // A later Firebase sign-in naming the same Google account, under another
    // Firebase UID and a changed address, reaches the same account by the sub.
    let login = f
        .resources
        .oauth2_accounts()
        .login_with_federated_identity(FederatedIdentity {
            firebase_uid: Some("fb-uid-other"),
            google_subject: Some("108000000000000000003"),
            email: "fire.renamed@gmail.com".to_owned(),
            email_verified: true,
            display_name: None,
            provider: "google.com",
            signup_source: SignupSource::Firebase,
        })
        .await
        .unwrap();
    assert_eq!(login.user.user_id, existing.id.to_string());
}

#[tokio::test]
async fn the_email_is_matched_whatever_its_casing() {
    let f = Fixture::new().await;
    let existing = f.account("mixed@gmail.com", true).await;

    let callback = f
        .sign_in(|nonce| {
            GoogleIdClaims::issued("108000000000000000004", " Mixed@Gmail.COM ", nonce)
        })
        .await;

    assert_eq!(callback.status(), 303, "{}", callback.body_text());
    assert_eq!(f.user("mixed@gmail.com").await.unwrap().id, existing.id);
}

// ── E: the email must be verified and present ───────────────────────────────

#[tokio::test]
async fn an_unverified_or_absent_email_is_refused_without_an_account_or_session() {
    let f = Fixture::new().await;
    let existing = f.account("taken@gmail.com", true).await;
    let (logs, guard) = capture_logs();

    for (email, verified) in [
        (Some("fresh@gmail.com"), Some(false)),
        (Some("fresh@gmail.com"), None),
        (Some("taken@gmail.com"), Some(false)),
        (None, Some(true)),
    ] {
        let callback = f
            .sign_in(|nonce| GoogleIdClaims {
                email: email.map(str::to_owned),
                email_verified: verified,
                ..GoogleIdClaims::issued("108000000000000000005", "unused@gmail.com", nonce)
            })
            .await;
        refused_without_session(&callback);
        let page = callback.body_text();
        assert!(page.contains(REFUSED_PAGE), "{page}");
        assert!(!page.contains("fresh@gmail.com") && !page.contains("taken@gmail.com"));
        assert!(clears_transaction(&callback));
    }
    drop(guard);

    assert!(
        f.user("fresh@gmail.com").await.is_none(),
        "no account created"
    );
    let unchanged = f.user("taken@gmail.com").await.unwrap();
    assert_eq!(unchanged.password_hash, existing.password_hash);
    assert_eq!(f.linked_subject(existing.id).await, None);
    let lines = logs.lock().unwrap();
    assert!(lines
        .iter()
        .any(|l| l.event.starts_with("Google sign-in refused")));
    assert!(!lines
        .iter()
        .any(|l| l.event.starts_with("Google sign-in could not complete")));
}

// ── F: the ID token ──────────────────────────────────────────────────────────

#[tokio::test]
async fn every_id_token_that_fails_a_check_is_refused() {
    let f = Fixture::new().await;
    let impostor = TestSigner::generate();
    let sub = "108000000000000000006";
    let email = "checks@gmail.com";

    let cases: Vec<(&str, Minter<'_>)> = vec![
        (
            "another audience",
            Box::new(|n| {
                f.signer.mint(&GoogleIdClaims {
                    aud: Some("another-client.apps.googleusercontent.com".to_owned()),
                    azp: None,
                    ..GoogleIdClaims::issued(sub, email, n)
                })
            }),
        ),
        (
            "a foreign issuer",
            Box::new(|n| {
                f.signer.mint(&GoogleIdClaims {
                    iss: Some("https://accounts.example.com".to_owned()),
                    ..GoogleIdClaims::issued(sub, email, n)
                })
            }),
        ),
        (
            "no issuer",
            Box::new(|n| {
                f.signer.mint(&GoogleIdClaims {
                    iss: None,
                    ..GoogleIdClaims::issued(sub, email, n)
                })
            }),
        ),
        (
            "no subject",
            Box::new(|n| {
                f.signer.mint(&GoogleIdClaims {
                    sub: None,
                    ..GoogleIdClaims::issued(sub, email, n)
                })
            }),
        ),
        (
            "another sign-in's nonce",
            Box::new(|_| {
                f.signer
                    .mint(&GoogleIdClaims::issued(sub, email, "someone-elses-nonce"))
            }),
        ),
        (
            "no nonce (a service-account-shaped token)",
            Box::new(|n| {
                f.signer.mint(&GoogleIdClaims {
                    nonce: None,
                    ..GoogleIdClaims::issued(sub, email, n)
                })
            }),
        ),
        (
            "another authorized party",
            Box::new(|n| {
                f.signer.mint(&GoogleIdClaims {
                    azp: Some("another-client.apps.googleusercontent.com".to_owned()),
                    ..GoogleIdClaims::issued(sub, email, n)
                })
            }),
        ),
        (
            "expired past the leeway",
            Box::new(|n| {
                f.signer.mint(&GoogleIdClaims {
                    exp: Some(now_secs() - 120),
                    iat: Some(now_secs() - 3720),
                    ..GoogleIdClaims::issued(sub, email, n)
                })
            }),
        ),
        (
            "an impostor key under the published kid",
            Box::new(|n| {
                impostor.mint_with_kid(&GoogleIdClaims::issued(sub, email, n), &f.signer.kid)
            }),
        ),
        (
            "an unpublished kid",
            Box::new(|n| {
                f.signer
                    .mint_with_kid(&GoogleIdClaims::issued(sub, email, n), "unpublished-kid")
            }),
        ),
        ("garbage", Box::new(|_| "not.a.jwt".to_owned())),
    ];

    for (case, mint) in cases {
        let started = f.start().await;
        f.google.set_id_token(&mint(&started.nonce));
        let callback = f.callback(&started, &fresh_code()).await;
        assert!(
            callback.status() >= 400 && session(&callback).is_none(),
            "{case}: {} {}",
            callback.status(),
            callback.body_text()
        );
        assert!(callback.body_text().contains(REFUSED_PAGE), "{case}");
    }
    assert!(f.user(email).await.is_none(), "no account for any of them");

    // Google's bare issuer spelling is Google's own.
    let accepted = f
        .sign_in(|n| GoogleIdClaims {
            iss: Some("accounts.google.com".to_owned()),
            ..GoogleIdClaims::issued(sub, email, n)
        })
        .await;
    assert_eq!(accepted.status(), 303, "{}", accepted.body_text());
}

#[tokio::test]
async fn unreachable_google_keys_are_a_server_fault_not_a_refusal() {
    let f = Fixture::with(ISSUER, |config| GoogleSignInConfig {
        jwks_url: unreachable_key_set_url(),
        ..config
    })
    .await;
    let (logs, guard) = capture_logs();

    let callback = f
        .sign_in(|n| GoogleIdClaims::issued("108000000000000000007", "keys@gmail.com", n))
        .await;
    drop(guard);

    assert_eq!(callback.status(), 500, "{}", callback.body_text());
    assert!(callback
        .body_text()
        .contains("Google sign-in could not be completed"));
    assert!(session(&callback).is_none());
    assert!(logs
        .lock()
        .unwrap()
        .iter()
        .any(|l| l.event.starts_with("Google sign-in could not complete")));
}

// ── G, H: the transaction cookie ─────────────────────────────────────────────

#[tokio::test]
async fn a_callback_this_browser_did_not_start_never_reaches_google() {
    let f = Fixture::new().await;
    let started = f.start().await;
    f.google
        .set_id_token(&f.signer.mint(&GoogleIdClaims::issued(
            "108000000000000000008",
            "tabs@gmail.com",
            &started.nonce,
        )));

    // No cookie at all: login CSRF.
    let no_cookie = f
        .callback_with(None, &format!("state={}&code=c1", started.state))
        .await;
    refused_without_session(&no_cookie);
    assert!(!clears_transaction(&no_cookie));

    // A state this browser's transaction was not started for (another tab's).
    let forged = f
        .callback_with(Some(&started.cookie), "state=forged-state&code=c2")
        .await;
    refused_without_session(&forged);
    assert!(no_cookie.body_text().contains("started in another browser"));
    assert!(
        !clears_transaction(&forged),
        "another tab's sign-in stays usable"
    );

    // The same transaction under its unprefixed name is not this server's
    // host-only cookie: a sibling host could have set it.
    let value = started.cookie.split_once('=').unwrap().1;
    let tossed = f
        .callback_with(
            Some(&format!("pierre_google_sign_in={value}")),
            &format!("state={}&code=c3", started.state),
        )
        .await;
    refused_without_session(&tossed);

    // A flipped byte fails its seal.
    let mut flipped = value.to_owned();
    let last = flipped.pop().unwrap();
    flipped.push(if last == 'A' { 'B' } else { 'A' });
    let tampered = f
        .callback_with(
            Some(&format!("{TRANSACTION}={flipped}")),
            &format!("state={}&code=c4", started.state),
        )
        .await;
    refused_without_session(&tampered);

    assert!(
        f.google.token_requests().is_empty(),
        "Google is never called for a callback this browser did not start"
    );

    // The genuine callback still completes, and an injected redirect_uri is
    // ignored: the return is rebuilt from the sealed request.
    f.allow("tabs@gmail.com").await;
    let genuine = f
        .callback_with(
            Some(&started.cookie),
            &format!(
                "state={}&code={}&redirect_uri=https%3A%2F%2Fevil.example",
                started.state,
                fresh_code()
            ),
        )
        .await;
    assert_eq!(genuine.status(), 303, "{}", genuine.body_text());
    assert_eq!(
        genuine.header("location").unwrap(),
        expected_authorize(&f.client_id)
    );

    // Replayed: Google refuses the spent code; no session.
    let requests_before = f.google.token_requests().len();
    let code = f.google.token_requests()[0]["code"].clone();
    let replayed = f
        .callback_with(
            Some(&started.cookie),
            &format!("state={}&code={code}", started.state),
        )
        .await;
    refused_without_session(&replayed);
    assert_eq!(f.google.token_requests().len(), requests_before + 1);
}

/// Mints the ID token Google answers with, from the sign-in's nonce.
type Minter<'a> = Box<dyn Fn(&str) -> String + 'a>;

/// A transaction sealed as the start seals it, with `expires_at` and AAD chosen.
fn sealed_transaction(f: &Fixture, state: &str, expires_at: i64, aad: &str) -> String {
    let transaction = serde_json::json!({
        "state": state,
        "nonce": "sealed-nonce",
        "code_verifier": VERIFIER,
        "expires_at": expires_at,
        "request": {
            "response_type": "code",
            "client_id": f.client_id,
            "redirect_uri": REDIRECT,
            "scope": null,
            "state": STATE,
            "code_challenge": pkce_challenge(),
            "code_challenge_method": "S256",
            "resource": null
        }
    });
    f.resources
        .common
        .repos
        .security
        .encrypt_data_with_aad(&transaction.to_string(), aad)
        .unwrap()
}

#[tokio::test]
async fn an_expired_or_foreign_transaction_is_refused_before_google() {
    let f = Fixture::new().await;

    let expired = sealed_transaction(
        &f,
        "expired-state",
        (Utc::now() - Duration::seconds(5)).timestamp(),
        "oauth2:google-sign-in:v1",
    );
    let callback = f
        .callback_with(
            Some(&format!("{TRANSACTION}={expired}")),
            "state=expired-state&code=c1",
        )
        .await;
    refused_without_session(&callback);
    assert!(
        callback.body_text().contains("expired"),
        "{}",
        callback.body_text()
    );
    assert!(clears_transaction(&callback), "an expired one is cleared");

    let foreign = sealed_transaction(
        &f,
        "foreign-state",
        (Utc::now() + Duration::seconds(300)).timestamp(),
        "another:purpose",
    );
    let callback = f
        .callback_with(
            Some(&format!("{TRANSACTION}={foreign}")),
            "state=foreign-state&code=c2",
        )
        .await;
    refused_without_session(&callback);

    assert!(f.google.token_requests().is_empty());
}

// ── I: account status ────────────────────────────────────────────────────────

#[tokio::test]
async fn a_pending_account_signs_in_but_is_not_shown_consent() {
    let f = Fixture::new().await;

    let callback = f
        .sign_in(|n| GoogleIdClaims::issued("108000000000000000009", "waiting@gmail.com", n))
        .await;
    assert_eq!(callback.status(), 303, "{}", callback.body_text());
    let user = f.user("waiting@gmail.com").await.unwrap();
    assert_eq!(user.user_status, UserStatus::Pending);

    let jwt = session(&callback).unwrap();
    let page = authorize_page(&f, callback.header("location").unwrap(), &jwt).await;
    assert_eq!(page.status(), 400);
    let text = page.body_text();
    assert!(text.contains("waiting for approval"), "{text}");
    assert!(!text.contains("csrf_token"), "no consent form: {text}");
    assert!(page.header("location").is_none(), "no code for the client");
}

#[tokio::test]
async fn a_suspended_account_is_refused_at_the_callback() {
    let f = Fixture::new().await;
    let existing = f.account("banned@gmail.com", true).await;
    f.resources
        .common
        .repos
        .users
        .update_status(existing.id, UserStatus::Suspended, None)
        .await
        .unwrap();

    let callback = f
        .sign_in(|n| GoogleIdClaims::issued("108000000000000000010", "banned@gmail.com", n))
        .await;
    refused_without_session(&callback);
    assert!(callback.body_text().contains(REFUSED_PAGE));
}

// ── J: Google's own answers ──────────────────────────────────────────────────

#[tokio::test]
async fn a_cancelled_chooser_returns_to_the_login_page() {
    let f = Fixture::new().await;
    let started = f.start().await;

    let cancelled = f
        .callback_with(
            Some(&started.cookie),
            &format!("state={}&error=access_denied", started.state),
        )
        .await;

    assert_eq!(cancelled.status(), 303);
    let location = cancelled.header("location").unwrap();
    assert_eq!(
        location,
        expected_authorize(&f.client_id).replacen("/oauth2/authorize?", "/oauth2/login?", 1)
    );
    assert!(clears_transaction(&cancelled));
    assert!(session(&cancelled).is_none());
    assert!(f.google.token_requests().is_empty());

    // Google's error under a state this browser did not start builds nothing.
    let forged = f
        .callback_with(Some(&started.cookie), "state=forged&error=access_denied")
        .await;
    refused_without_session(&forged);
}

#[tokio::test]
async fn a_refused_code_is_a_refusal_and_a_failing_google_a_fault_neither_echoing_googles_body() {
    let f = Fixture::new().await;
    let (logs, guard) = capture_logs();

    let started = f.start().await;
    f.google.set_error(
        400,
        &format!(r#"{{"error":"invalid_grant","error_description":"{SENTINEL}"}}"#),
    );
    let refused = f.callback(&started, &fresh_code()).await;
    refused_without_session(&refused);
    assert!(refused.body_text().contains(REFUSED_PAGE));

    let started = f.start().await;
    f.google
        .set_error(500, &format!(r#"{{"message":"{SENTINEL}"}}"#));
    let failed = f.callback(&started, &fresh_code()).await;
    refused_without_session(&failed);
    assert_eq!(failed.status(), 503);
    assert!(failed
        .body_text()
        .contains("Google sign-in is unavailable right now"));
    drop(guard);

    for page in [refused.body_text(), failed.body_text()] {
        assert!(!page.contains(SENTINEL), "{page}");
        assert!(!page.contains(GOOGLE_CLIENT_SECRET));
    }
    let lines = logs.lock().unwrap();
    for line in lines.iter() {
        let text = format!("{} {:?}", line.event, line.fields);
        assert!(!text.contains(SENTINEL), "{text}");
        assert!(!text.contains(GOOGLE_CLIENT_SECRET), "{text}");
    }
    assert!(lines
        .iter()
        .any(|l| l.event.starts_with("Google sign-in refused")));
    assert!(lines
        .iter()
        .any(|l| l.event.starts_with("Google sign-in could not complete")));
}

// ── K: the start refuses what it would not authorize ─────────────────────────

#[tokio::test]
async fn the_start_refuses_a_request_it_would_not_authorize() {
    let f = Fixture::new().await;
    let no_google = |response: &AxumTestResponse| {
        assert!(response
            .header("location")
            .is_none_or(|l| !l.starts_with(GOOGLE_CHOOSER)));
        assert!(response.header("set-cookie").is_none());
    };

    let unknown_client = AxumTestRequest::get(&format!(
        "/oauth2/login/google?{}",
        authorize_query("mcp_client_unknown")
    ))
    .send(f.routes())
    .await;
    assert_eq!(unknown_client.status(), 400);
    no_google(&unknown_client);

    let unregistered = AxumTestRequest::get(&format!(
        "/oauth2/login/google?{}",
        authorize_query(&f.client_id).replace(
            &urlencoding::encode(REDIRECT).into_owned(),
            &urlencoding::encode("https://evil.example/callback")
        )
    ))
    .send(f.routes())
    .await;
    assert_eq!(unregistered.status(), 400);
    no_google(&unregistered);

    let no_pkce = AxumTestRequest::get(&format!(
        "/oauth2/login/google?response_type=code&client_id={}&redirect_uri={}&state=s",
        urlencoding::encode(&f.client_id),
        urlencoding::encode(REDIRECT)
    ))
    .send(f.routes())
    .await;
    no_google(&no_pkce);

    // A request too large to ride a cookie.
    let long_redirect = format!("https://client.example.test/{}", "a".repeat(3000));
    let big_client = register(&f.resources, vec![long_redirect.clone()]).await;
    let too_large = AxumTestRequest::get(&format!(
        "/oauth2/login/google?response_type=code&client_id={}&redirect_uri={}&state=s&code_challenge={}&code_challenge_method=S256",
        urlencoding::encode(&big_client),
        urlencoding::encode(&long_redirect),
        pkce_challenge()
    ))
    .send(f.routes())
    .await;
    assert_eq!(too_large.status(), 400);
    assert!(too_large.body_text().contains("too large"));
    no_google(&too_large);

    // Rate-limited in the authorize window, with its Retry-After.
    let routes = f.routes();
    let uri = format!("/oauth2/login/google?{}", authorize_query(&f.client_id));
    let mut limited = None;
    for _ in 0..80 {
        let response = AxumTestRequest::get(&uri).send(routes.clone()).await;
        if response.status() == 429 {
            limited = Some(response);
            break;
        }
    }
    let limited = limited.expect("the start is rate-limited");
    assert!(limited.header("retry-after").is_some());
}

// ── N, L3, M: configuration and the login page ───────────────────────────────

#[tokio::test]
async fn without_google_configured_neither_route_goes_to_google_and_the_page_has_no_button() {
    let resources = resources_with(OAuth2ServerConfig {
        issuer_url: ISSUER.to_owned(),
        mcp_resource_url: MCP_RESOURCE.to_owned(),
        ..OAuth2ServerConfig::default()
    })
    .await;
    let client_id = register(&resources, vec![REDIRECT.to_owned()]).await;
    let query = authorize_query(&client_id);

    for uri in [
        format!("/oauth2/login/google?{query}"),
        "/oauth2/login/google/callback?state=s&code=c".to_owned(),
    ] {
        let response = AxumTestRequest::get(&uri)
            .send(oauth2_routes(&resources))
            .await;
        assert_eq!(response.status(), 400, "{uri}");
        assert!(response.header("location").is_none(), "{uri}");
        assert!(response.body_text().contains("not available"), "{uri}");
    }

    let page = AxumTestRequest::get(&format!("/oauth2/login?{query}"))
        .send(oauth2_routes(&resources))
        .await
        .body_text();
    assert!(!page.contains("Continue with Google"), "{page}");
}

#[tokio::test]
async fn the_login_page_links_google_on_the_issuer_host_for_this_request() {
    let f = Fixture::new().await;
    let page = AxumTestRequest::get(&format!("/oauth2/login?{}", authorize_query(&f.client_id)))
        .send(f.routes())
        .await
        .body_text();

    let expected = format!(
        "https://app.example.test/oauth2/login/google?{}",
        expected_authorize(&f.client_id).split_once('?').unwrap().1
    );
    assert!(
        page.contains(&format!(
            "href=\"{}\">Continue with Google</a>",
            expected.replace('&', "&amp;")
        )),
        "{page}"
    );
}

#[tokio::test]
async fn the_server_mounts_both_routes_past_its_csrf_layer() {
    let f = Fixture::new().await;
    let app = ProviderToolRouter::build_http_app(&f.resources)
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_653))));

    let page = AxumTestRequest::get(&format!("/oauth2/login?{}", authorize_query(&f.client_id)))
        .send(app.clone())
        .await
        .body_text();
    assert!(page.contains("Continue with Google"), "{page}");

    // A web-app session cookie in the jar does not stand in the way.
    let start = AxumTestRequest::get(&format!(
        "/oauth2/login/google?{}",
        authorize_query(&f.client_id)
    ))
    .header(
        "cookie",
        &format!("{}=a-web-app-session", auth_cookie_name()),
    )
    .send(app)
    .await;
    started(&start);
}

/// The whole sign-in through the server's own router, CSRF and request-budget
/// layers included: start, Google's answer, the callback, then the pending
/// authorization, which shows consent for the new session.
#[tokio::test]
async fn the_production_router_takes_google_sign_in_through_to_consent() {
    let f = Fixture::new().await;
    f.allow("router.athlete@gmail.com").await;
    let app = ProviderToolRouter::build_http_app(&f.resources)
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_654))));

    let started = started(
        &AxumTestRequest::get(&format!(
            "/oauth2/login/google?{}",
            authorize_query(&f.client_id)
        ))
        .send(app.clone())
        .await,
    );
    f.google
        .set_id_token(&f.signer.mint(&GoogleIdClaims::issued(
            "108000000000000000030",
            "router.athlete@gmail.com",
            &started.nonce,
        )));
    let callback = AxumTestRequest::get(&format!(
        "/oauth2/login/google/callback?state={}&code={}",
        urlencoding::encode(&started.state),
        fresh_code()
    ))
    .header("cookie", &started.cookie)
    .send(app.clone())
    .await;
    assert_eq!(callback.status(), 303, "{}", callback.body_text());
    let location = callback.header("location").unwrap().to_owned();
    assert_eq!(location, expected_authorize(&f.client_id));
    let jwt = session(&callback).unwrap();

    let consent = AxumTestRequest::get(&location)
        .header("cookie", &format!("{SESSION}={jwt}"))
        .send(app)
        .await;
    assert_eq!(consent.status(), 200);
    let page = consent.body_text();
    assert!(page.contains(CLIENT_NAME), "{page}");
    assert!(page.contains("router.athlete@gmail.com"), "{page}");
    assert!(page.contains("name=\"csrf_token\""), "{page}");
}

#[tokio::test]
async fn the_default_server_offers_no_google_button() {
    let resources = resources_with(OAuth2ServerConfig::default()).await;
    let client_id = register(&resources, vec![REDIRECT.to_owned()]).await;
    let app = ProviderToolRouter::build_http_app(&resources)
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_654))));

    let page = AxumTestRequest::get(&format!("/oauth2/login?{}", authorize_query(&client_id)))
        .send(app)
        .await
        .body_text();
    assert!(page.contains("name=\"password\""), "{page}");
    assert!(!page.contains("Continue with Google"), "{page}");
}

// ── P: the Google account id comes first ─────────────────────────────────────

#[tokio::test]
async fn a_linked_google_account_is_found_by_its_sub_even_after_its_email_changes() {
    let f = Fixture::new().await;
    f.allow("before@gmail.com").await;
    let first = f
        .sign_in(|n| GoogleIdClaims::issued("108000000000000000011", "before@gmail.com", n))
        .await;
    assert_eq!(first.status(), 303);
    let account = f.user("before@gmail.com").await.unwrap();

    let renamed = f
        .sign_in(|n| GoogleIdClaims::issued("108000000000000000011", "after@gmail.com", n))
        .await;
    assert_eq!(renamed.status(), 303, "{}", renamed.body_text());
    let jwt = session(&renamed).unwrap();
    let claims = f
        .resources
        .auth
        .auth_manager
        .validate_authorization_session_token(&jwt, &f.resources.auth.jwks_manager)
        .unwrap();
    assert_eq!(claims.sub, account.id.to_string());
    assert!(
        f.user("after@gmail.com").await.is_none(),
        "no second account"
    );
}

#[tokio::test]
async fn an_address_now_held_by_another_google_account_is_not_attached() {
    let f = Fixture::new().await;
    let account = f.account("reassigned@gmail.com", true).await;
    f.resources
        .common
        .repos
        .federated_identities
        .link_subject(account.id, "google.com", "108000000000000000012")
        .await
        .unwrap();

    let other = f
        .sign_in(|n| GoogleIdClaims::issued("108000000000000000099", "reassigned@gmail.com", n))
        .await;
    refused_without_session(&other);
    assert_eq!(
        f.linked_subject(account.id).await.as_deref(),
        Some("108000000000000000012")
    );

    // Deleting the account takes its links with it.
    f.resources
        .common
        .repos
        .users
        .delete(account.id, None)
        .await
        .unwrap();
    assert_eq!(
        f.resources
            .common
            .repos
            .federated_identities
            .user_for_subject("google.com", "108000000000000000012")
            .await
            .unwrap(),
        None
    );
}

// ── Q: an account registered before its owner proved the address ─────────────

#[tokio::test]
async fn proving_an_unverified_accounts_email_retires_every_credential_it_held() {
    let f = Fixture::new().await;
    let repos = &f.resources.common.repos;
    let mut squatted = f.account("victim@gmail.com", false).await;
    squatted.firebase_uid = Some("attacker-firebase-uid".to_owned());
    repos.users.update(&squatted).await.unwrap();
    let accounts = f.resources.oauth2_accounts();
    let session_refresh = accounts
        .issue_refresh_token(squatted.id, None)
        .await
        .unwrap();
    let tenant_id = repos.tenants.list_for_user(squatted.id).await.unwrap()[0]
        .id
        .to_string();
    let connector_refresh = format!("connector-refresh-{}", Uuid::new_v4().simple());
    repos
        .oauth2_server
        .store_refresh_token(&OAuth2RefreshToken {
            token: connector_refresh.clone(),
            client_id: f.client_id.clone(),
            user_id: squatted.id,
            tenant_id,
            scope: None,
            expires_at: Utc::now() + Duration::days(30),
            created_at: Utc::now(),
            revoked: false,
            family_id: Uuid::new_v4().to_string(),
            resource: None,
        })
        .await
        .unwrap();
    let mcp = repos
        .user_mcp_tokens
        .create_token(
            squatted.id,
            &CreateUserMcpTokenRequest {
                name: "squatter's client".to_owned(),
                expires_in_days: None,
            },
        )
        .await
        .unwrap();

    let api_key = ApiKey {
        id: Uuid::new_v4().to_string(),
        user_id: squatted.id,
        name: "squatter's script".to_owned(),
        key_prefix: "pk_squat".to_owned(),
        key_hash: "squatter-key-hash".to_owned(),
        description: None,
        tier: ApiKeyTier::Starter,
        rate_limit_requests: 100,
        rate_limit_window_seconds: 3600,
        is_active: true,
        last_used_at: None,
        expires_at: None,
        created_at: Utc::now(),
    };
    repos.api_keys.create(&api_key).await.unwrap();

    let callback = f
        .sign_in(|n| GoogleIdClaims::issued("108000000000000000013", "victim@gmail.com", n))
        .await;
    assert_eq!(callback.status(), 303, "{}", callback.body_text());

    let owner = f.user("victim@gmail.com").await.unwrap();
    assert_eq!(owner.id, squatted.id, "the same account, now the owner's");
    assert_eq!(owner.password_hash, FEDERATED_ONLY_PASSWORD_HASH);
    assert_eq!(
        owner.firebase_uid, None,
        "the foreign Firebase link is gone"
    );
    assert!(f.verified(owner.id).await);
    assert!(accounts
        .login(LoginRequest {
            email: "victim@gmail.com".to_owned(),
            password: "password123".to_owned(),
            timezone: None,
        })
        .await
        .is_err());
    let hosted = AxumTestRequest::post("/oauth2/login")
        .form(&[("email", "victim@gmail.com"), ("password", "password123")])
        .send(f.routes())
        .await;
    assert_eq!(hosted.status(), 401);
    assert!(accounts.refresh_session(&session_refresh).await.is_err());
    assert!(
        repos
            .oauth2_server
            .get_refresh_token_by_value(&connector_refresh)
            .await
            .unwrap()
            .unwrap()
            .revoked
    );
    assert!(repos
        .user_mcp_tokens
        .validate_token(&mcp.token_value)
        .await
        .is_err());
    let keys = repos.api_keys.get_for_user(owner.id).await.unwrap();
    assert_eq!(keys.len(), 1);
    assert!(
        !keys[0].is_active,
        "the squatter's API key no longer authenticates"
    );
}

#[tokio::test]
async fn the_firebase_path_retires_an_unverified_accounts_credentials_the_same_way() {
    let f = Fixture::new().await;
    let squatted = f.account("firebase.victim@example.org", false).await;

    let login = f
        .resources
        .oauth2_accounts()
        .login_with_federated_identity(FederatedIdentity {
            firebase_uid: Some("owner-firebase-uid"),
            google_subject: Some("108000000000000000014"),
            email: "firebase.victim@example.org".to_owned(),
            email_verified: true,
            display_name: None,
            provider: "google.com",
            signup_source: SignupSource::Firebase,
        })
        .await
        .unwrap();

    assert_eq!(login.user.user_id, squatted.id.to_string());
    let owner = f.user("firebase.victim@example.org").await.unwrap();
    assert_eq!(owner.password_hash, FEDERATED_ONLY_PASSWORD_HASH);
    assert_eq!(owner.firebase_uid.as_deref(), Some("owner-firebase-uid"));
    assert_eq!(
        f.linked_subject(owner.id).await.as_deref(),
        Some("108000000000000000014")
    );
}

// ── R: one attach rule for both sign-in paths ─────────────────────────────────

#[tokio::test]
async fn a_verified_address_on_any_domain_attaches_as_the_web_app_does() {
    let f = Fixture::new().await;
    let existing = f.account("coach@club.example", true).await;

    // Not a Google mail domain and no Workspace claim: Google verified the
    // address, and the web app's Firebase sign-in attaches on that same word.
    let attached = f
        .sign_in(|n| GoogleIdClaims::issued("108000000000000000015", "coach@club.example", n))
        .await;
    assert_eq!(attached.status(), 303, "{}", attached.body_text());
    assert_eq!(
        f.linked_subject(existing.id).await.as_deref(),
        Some("108000000000000000015")
    );
}

#[tokio::test]
async fn a_firebase_google_athletes_first_hosted_sign_in_keeps_everything() {
    let f = Fixture::new().await;
    let repos = &f.resources.common.repos;
    // An athlete who has only ever used the web app's Google sign-in: linked
    // to Firebase, never through the confirmation link, no Google sub on file.
    let mut athlete = f.account("rider@club.example", false).await;
    athlete.firebase_uid = Some("athlete-firebase-uid".to_owned());
    athlete.auth_provider = "google.com".to_owned();
    repos.users.update(&athlete).await.unwrap();
    let mcp = repos
        .user_mcp_tokens
        .create_token(
            athlete.id,
            &CreateUserMcpTokenRequest {
                name: "Claude Desktop".to_owned(),
                expires_in_days: None,
            },
        )
        .await
        .unwrap();

    let callback = f
        .sign_in(|n| GoogleIdClaims::issued("108000000000000000016", "rider@club.example", n))
        .await;
    assert_eq!(callback.status(), 303, "{}", callback.body_text());

    let after = f.user("rider@club.example").await.unwrap();
    assert_eq!(after.id, athlete.id);
    assert_eq!(
        after.firebase_uid.as_deref(),
        Some("athlete-firebase-uid"),
        "the Firebase link stays, so the web app still signs in"
    );
    assert_eq!(after.password_hash, athlete.password_hash);
    assert!(
        repos
            .user_mcp_tokens
            .validate_token(&mcp.token_value)
            .await
            .is_ok(),
        "the athlete's connectors keep working"
    );
    assert_eq!(
        f.linked_subject(after.id).await.as_deref(),
        Some("108000000000000000016")
    );
}

// ── S: an issuer written with a trailing slash ───────────────────────────────

#[tokio::test]
async fn an_issuer_with_a_trailing_slash_registers_one_redirect_uri() {
    assert_eq!(
        google_callback_url("https://app.example.test/"),
        "https://app.example.test/oauth2/login/google/callback"
    );
    let f = Fixture::with("https://app.example.test/", |config| config).await;
    let started = f.start().await;
    assert_eq!(
        query_param(&started.location, "redirect_uri").as_deref(),
        Some("https://app.example.test/oauth2/login/google/callback")
    );
}

// ── The hosted password form follows the same account rules ─────────────────

#[tokio::test]
async fn the_password_form_refuses_a_suspended_account_and_gates_a_pending_one() {
    let f = Fixture::new().await;
    let (events, guard) = capture_notify();

    let suspended = f.account("suspended@example.test", true).await;
    f.resources
        .common
        .repos
        .users
        .update_status(suspended.id, UserStatus::Suspended, None)
        .await
        .unwrap();
    let refused = AxumTestRequest::post("/oauth2/login")
        .form(&[
            ("email", "suspended@example.test"),
            ("password", "password123"),
        ])
        .send(f.routes())
        .await;
    assert_eq!(refused.status(), 401);
    assert!(session(&refused).is_none());

    let pending = f.account("pending@example.test", true).await;
    f.resources
        .common
        .repos
        .users
        .update_status(pending.id, UserStatus::Pending, None)
        .await
        .unwrap();
    let challenge = pkce_challenge();
    let signed_in = AxumTestRequest::post("/oauth2/login")
        .form(&[
            ("email", "pending@example.test"),
            ("password", "password123"),
            ("client_id", f.client_id.as_str()),
            ("redirect_uri", REDIRECT),
            ("response_type", "code"),
            ("state", STATE),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("resource", ""),
        ])
        .send(f.routes())
        .await;
    drop(guard);
    assert_eq!(signed_in.status(), 302, "{}", signed_in.body_text());
    let jwt = session(&signed_in).expect("the password sets the same session cookie");
    let page = authorize_page(&f, signed_in.header("location").unwrap(), &jwt).await;
    assert!(
        page.body_text().contains("waiting for approval"),
        "{}",
        page.body_text()
    );
    assert!(!page.body_text().contains("csrf_token"));

    let logins = named(&events, "user.login");
    assert_eq!(logins.len(), 1, "one successful sign-in, one user.login");
    assert_eq!(logins[0].field("user_id"), pending.id.to_string());
}

#[tokio::test]
async fn a_session_cookie_without_its_host_prefix_is_not_the_servers_on_https() {
    let f = Fixture::new().await;
    let user = f.account("prefix@example.test", true).await;
    let jwt = f
        .resources
        .auth
        .auth_manager
        .generate_authorization_session_token(&user, &f.resources.auth.jwks_manager, None)
        .unwrap();

    let tossed = AxumTestRequest::get(&expected_authorize(&f.client_id))
        .header("cookie", &format!("pierre_session={jwt}"))
        .send(f.routes())
        .await;
    assert_eq!(tossed.status(), 303);
    assert!(tossed
        .header("location")
        .unwrap()
        .starts_with("/oauth2/login?"));

    let own = authorize_page(&f, &expected_authorize(&f.client_id), &jwt).await;
    assert_eq!(own.status(), 200, "{}", own.body_text());
}

#[tokio::test]
async fn over_plain_http_the_session_cookie_keeps_its_bare_name() {
    let resources = resources_with(OAuth2ServerConfig::default()).await;
    let (_, _user) = create_test_user_with_email(&resources.agent.database, "local@example.test")
        .await
        .unwrap();

    let signed_in = AxumTestRequest::post("/oauth2/login")
        .form(&[("email", "local@example.test"), ("password", "password123")])
        .send(oauth2_routes(&resources))
        .await;

    assert_eq!(signed_in.status(), 302);
    let cookie = signed_in.header("set-cookie").unwrap();
    let (pair, attributes) = cookie_parts(cookie);
    assert!(pair.starts_with("pierre_session="), "{cookie}");
    assert_eq!(
        attributes,
        ["HttpOnly", "Max-Age=86400", "Path=/", "SameSite=Lax"]
    );
}

/// The hosted login form is a page any script holding the password can post
/// to and read the cookie off, so the token it sets is no first-party session
/// (carnet#787): REST refuses it as a bearer and as the web app's cookie, the
/// MCP and A2A entry point refuses it, while `/oauth2/authorize` accepts it.
#[tokio::test]
async fn the_hosted_login_cookie_signs_in_the_authorize_flow_only() {
    let resources = resources_with(OAuth2ServerConfig::default()).await;
    let (_, user) = create_test_user_with_email(&resources.agent.database, "lifted@example.test")
        .await
        .unwrap();

    let signed_in = AxumTestRequest::post("/oauth2/login")
        .form(&[
            ("email", "lifted@example.test"),
            ("password", "password123"),
        ])
        .send(oauth2_routes(&resources))
        .await;
    assert_eq!(signed_in.status(), 302);
    let lifted = signed_in
        .header("set-cookie")
        .and_then(|c| c.strip_prefix("pierre_session="))
        .map(|rest| rest.split(';').next().unwrap().to_owned())
        .expect("the login sets the authorization server's cookie");

    let middleware = &resources.auth.auth_middleware;
    let bearer = format!("Bearer {lifted}");
    let rest = middleware
        .authenticate_request(Some(&bearer))
        .await
        .expect_err("REST must refuse the hosted login's token as a bearer");
    assert_eq!(rest.code, ErrorCode::AuthInvalid, "{rest}");

    let mut cookie_headers = HeaderMap::new();
    cookie_headers.insert(
        COOKIE,
        format!("{}={lifted}", auth_cookie_name()).parse().unwrap(),
    );
    assert!(
        middleware
            .authenticate_request_with_headers(&cookie_headers)
            .await
            .is_err(),
        "REST must refuse it planted as the web app's session cookie"
    );

    let mcp_resources = resources.common.config.oauth2_server.mcp_resources();
    assert!(
        middleware
            .authenticate_scoped_request(Some(&bearer), &mcp_resources)
            .await
            .is_err(),
        "MCP and A2A must refuse it"
    );

    // Where it belongs, it signs the athlete in: the consent screen renders.
    let client_id = register(&resources, vec![REDIRECT.to_owned()]).await;
    let consent = AxumTestRequest::get(&expected_authorize(&client_id))
        .header("cookie", &format!("pierre_session={lifted}"))
        .send(oauth2_routes(&resources))
        .await;
    assert_eq!(consent.status(), 200, "{}", consent.body_text());

    // And the reverse: a first-party session token is not the server's
    // sign-in, so the server's own cookie carrying one starts a login.
    let app_session = resources
        .auth
        .auth_manager
        .generate_token(&user, &resources.auth.jwks_manager)
        .unwrap();
    let refused = AxumTestRequest::get(&expected_authorize(&client_id))
        .header("cookie", &format!("pierre_session={app_session}"))
        .send(oauth2_routes(&resources))
        .await;
    assert_eq!(refused.status(), 303);
    assert!(refused
        .header("location")
        .unwrap()
        .starts_with("/oauth2/login?"));
}
