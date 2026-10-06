// ABOUTME: Signs an athlete in the way Dravr's own web and mobile apps do: authorization code + PKCE via the hosted login page
// ABOUTME: Drives /oauth2/authorize, POST /oauth2/login, the cookie-bearing authorize and POST /oauth/token over one test router
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(dead_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The first-party sign-in (carnet#787), for the suites that need a signed-in
//! athlete or assert the sign-in's own rules.
//!
//! The apps never send a password to `/oauth/token`: the athlete types it on
//! the hosted login page, and the app redeems the authorization code it is
//! sent back with its PKCE verifier. [`SignIn::run`] walks that whole flow
//! over a router mounting both the authorization server (`/oauth2/*`) and the
//! app's auth routes (`/oauth/token`, `/api/auth/*`), with the server's own
//! shared rate limiter, so the sign-in windows a test fills persist from one
//! request to the next as they do when the routes are served.
//!
//! The individual steps ([`authorize_uri`], [`submit_login`], [`redeem`], …)
//! are exported for the tests that assert one step's refusal.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use axum::Router;
use base64::{engine::general_purpose, Engine as _};
use pierre_auth::config::OAuth2ServerConfig;
use pierre_auth::oauth2_server::first_party::{
    CALLBACK_PATH, MOBILE_CALLBACK_URI, MOBILE_CLIENT_ID, WEB_CLIENT_ID,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_auth::AuthRoutes;
use pierre_routes_identity::oauth2::{OAuth2Context, OAuth2Routes};
use rand::RngCore;
use sha2::{Digest, Sha256};
use url::Url;

use super::axum_test::{AxumTestRequest, AxumTestResponse};
use pierre_contremaitre::MessagingStringsRegistry;

/// The address every request of a sign-in arrives from unless a test names
/// another ([`SignIn::arriving_from`]).
pub const DEFAULT_PEER: [u8; 4] = [127, 0, 0, 1];

/// One of Dravr's own apps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirstPartyClient {
    /// The web app, `dravr-web`: its callback is `/auth/callback` on a
    /// configured web origin, its session a cookie.
    Web,
    /// The mobile app, `dravr-mobile`: its callback is `dravr://auth/callback`.
    Mobile,
}

impl FirstPartyClient {
    /// The `client_id` the app signs in as.
    #[must_use]
    pub const fn client_id(self) -> &'static str {
        match self {
            Self::Web => WEB_CLIENT_ID,
            Self::Mobile => MOBILE_CLIENT_ID,
        }
    }

    /// The callback this deployment lets the app receive its code at: the
    /// web app's on the first configured web origin, the mobile app's scheme.
    #[must_use]
    pub fn redirect_uri(self, config: &OAuth2ServerConfig) -> String {
        match self {
            Self::Web => web_redirect_uri(config),
            Self::Mobile => MOBILE_CALLBACK_URI.to_owned(),
        }
    }
}

/// `/auth/callback` on the deployment's first web origin.
#[must_use]
pub fn web_redirect_uri(config: &OAuth2ServerConfig) -> String {
    let origin = config
        .first_party_redirects
        .web_origins
        .first()
        .expect("the deployment names a first-party web origin");
    format!("{}{CALLBACK_PATH}", origin.trim_end_matches('/'))
}

/// A PKCE verifier and its S256 challenge (RFC 7636).
#[derive(Clone, Debug)]
pub struct Pkce {
    /// The secret the app keeps and redeems the code with
    pub verifier: String,
    /// `BASE64URL(SHA256(verifier))`, sent to `/oauth2/authorize`
    pub challenge: String,
}

impl Pkce {
    /// A fresh 32-byte verifier and its challenge.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0_u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        let verifier = general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let challenge = s256(&verifier);
        Self {
            verifier,
            challenge,
        }
    }
}

/// The S256 challenge of `verifier`.
#[must_use]
pub fn s256(verifier: &str) -> String {
    general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// A fresh opaque `state`.
#[must_use]
pub fn fresh_state() -> String {
    let mut bytes = [0_u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// The authorization server's routes and the app's auth routes, sharing the
/// server's rate limiter as they do when mounted, every request arriving
/// from `peer`.
pub fn first_party_router(resources: &ServerContext, peer: [u8; 4]) -> Router {
    let oauth2 = OAuth2Context {
        database: resources.agent.database.clone(),
        oauth2_server: resources.common.repos.oauth2_server.clone(),
        tenants: resources.common.repos.tenants.clone(),
        users: resources.common.repos.users.clone(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        config: Arc::new(resources.common.config.oauth2_server.clone()),
        rate_limiter: resources.auth.oauth2_rate_limiter.clone(),
        refresh_token_expiry_days: resources.common.config.auth.refresh_token_expiry_days,
        csrf_manager: resources.auth.csrf_manager.clone(),
        accounts: resources.oauth2_accounts(),
        google_sign_in: resources.oauth2_google_sign_in(),
        strings: Arc::new(MessagingStringsRegistry::new()),
    };
    OAuth2Routes::routes(oauth2)
        .merge(AuthRoutes::routes(resources.auth_routes_context()))
        .layer(MockConnectInfo(SocketAddr::from((peer, 40_787))))
}

/// `GET /oauth2/authorize` for `client_id`, sending the code to
/// `redirect_uri` with `pkce`'s challenge and `state`.
#[must_use]
pub fn authorize_uri(client_id: &str, redirect_uri: &str, challenge: &str, state: &str) -> String {
    format!(
        "/oauth2/authorize?response_type=code&client_id={}&redirect_uri={}&code_challenge={}\
         &code_challenge_method=S256&state={}",
        urlencoding::encode(client_id),
        urlencoding::encode(redirect_uri),
        urlencoding::encode(challenge),
        urlencoding::encode(state),
    )
}

/// `POST /oauth2/login`: the hosted form, carrying the authorize request's
/// hidden fields and the athlete's `email` and `password`, from an English
/// browser (the page answers in the browser's language).
pub async fn submit_login(
    router: Router,
    client_id: &str,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
    email: &str,
    password: &str,
) -> AxumTestResponse {
    AxumTestRequest::post("/oauth2/login")
        .header("accept-language", "en")
        .form(&[
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
            ("state", state),
            ("email", email),
            ("password", password),
        ])
        .send(router)
        .await
}

/// `POST /oauth/token` redeeming `code`: the form the apps send, with the
/// verifier and the scope left out when `None`.
pub async fn redeem(
    router: Router,
    client_id: &str,
    code: &str,
    redirect_uri: &str,
    code_verifier: Option<&str>,
    scope: Option<&str>,
) -> AxumTestResponse {
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("client_id", client_id),
        ("code", code),
        ("redirect_uri", redirect_uri),
    ];
    if let Some(verifier) = code_verifier {
        form.push(("code_verifier", verifier));
    }
    if let Some(scope) = scope {
        form.push(("scope", scope));
    }
    AxumTestRequest::post("/oauth/token")
        .form(&form)
        .send(router)
        .await
}

/// Whether `response` is a redirect (302 or 303).
#[must_use]
pub fn is_redirect(response: &AxumTestResponse) -> bool {
    matches!(response.status(), 302 | 303)
}

/// The `Location` a redirect points at.
#[must_use]
pub fn location(response: &AxumTestResponse) -> String {
    response
        .header("location")
        .unwrap_or_else(|| {
            panic!(
                "expected a redirect, got {}: {}",
                response.status(),
                response.body_text()
            )
        })
        .to_owned()
}

/// The `name=value` of the first `Set-Cookie` a response carries, ready for
/// a `Cookie` header.
#[must_use]
pub fn session_cookie(response: &AxumTestResponse) -> String {
    response
        .header("set-cookie")
        .and_then(|cookie| cookie.split(';').next())
        .expect("a session cookie")
        .to_owned()
}

/// The query parameter `name` of a redirect to `redirect_uri`, or `None`
/// when the redirect does not go there or does not carry it.
#[must_use]
pub fn callback_param(location: &str, redirect_uri: &str, name: &str) -> Option<String> {
    if !location.starts_with(&format!("{redirect_uri}?")) {
        return None;
    }
    let (_, query) = location.split_once('?')?;
    Url::parse(&format!("http://callback.invalid/?{query}"))
        .ok()?
        .query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// How a first-party sign-in ended.
pub enum SignInOutcome {
    /// The hosted login form refused the sign-in — a wrong password, an
    /// unknown or suspended account, a full sign-in window: its response.
    LoginRefused(AxumTestResponse),
    /// `/oauth2/authorize` refused the request (an unaccepted `redirect_uri`,
    /// before the login page) or sent no code back once signed in: its
    /// response.
    AuthorizeRefused(AxumTestResponse),
    /// The code was redeemed: the `/oauth/token` response, whatever its
    /// status.
    Token(AxumTestResponse),
}

impl SignInOutcome {
    /// The `/oauth/token` response, asserting the flow reached it.
    #[must_use]
    pub fn token(self) -> AxumTestResponse {
        match self {
            Self::Token(response) => response,
            Self::LoginRefused(response) => panic!(
                "the hosted login form refused the sign-in ({}): {}",
                response.status(),
                response.body_text()
            ),
            Self::AuthorizeRefused(response) => panic!(
                "/oauth2/authorize issued no code ({}): {}",
                response.status(),
                response.body_text()
            ),
        }
    }

    /// The JSON body of a successful sign-in, asserting it was one.
    #[must_use]
    pub fn signed_in(self) -> serde_json::Value {
        let response = self.token();
        assert_eq!(response.status(), 200, "{}", response.body_text());
        response.json()
    }

    /// `/oauth2/authorize`'s refusal, asserting the sign-in ended there.
    #[must_use]
    pub fn authorize_refused(self) -> AxumTestResponse {
        match self {
            Self::AuthorizeRefused(response) => response,
            Self::LoginRefused(response) | Self::Token(response) => panic!(
                "expected /oauth2/authorize to refuse the sign-in, got {}: {}",
                response.status(),
                response.body_text()
            ),
        }
    }

    /// The hosted login form's refusal, asserting the sign-in ended there.
    #[must_use]
    pub fn login_refused(self) -> AxumTestResponse {
        match self {
            Self::LoginRefused(response) => response,
            Self::AuthorizeRefused(response) | Self::Token(response) => panic!(
                "expected the hosted login form to refuse the sign-in, got {}: {}",
                response.status(),
                response.body_text()
            ),
        }
    }
}

/// An authorization code the signed-in `/oauth2/authorize` sent to the app's
/// callback, with what the app needs to redeem it.
pub struct IssuedCode {
    /// The router the flow ran over, sharing the server's state
    pub router: Router,
    /// The app the code was issued to
    pub client_id: &'static str,
    /// The callback the code was sent to
    pub redirect_uri: String,
    /// The code
    pub code: String,
    /// The PKCE verifier of the challenge the code was issued for
    pub verifier: String,
}

impl IssuedCode {
    /// Redeem the code as the app does, with its verifier, at `/oauth/token`.
    pub async fn redeem(&self, scope: Option<&str>) -> AxumTestResponse {
        redeem(
            self.router.clone(),
            self.client_id,
            &self.code,
            &self.redirect_uri,
            Some(&self.verifier),
            scope,
        )
        .await
    }
}

/// A first-party sign-in to run: the web app by default, at its configured
/// callback, no refresh token, from [`DEFAULT_PEER`].
#[derive(Clone, Debug)]
pub struct SignIn {
    email: String,
    password: String,
    client: FirstPartyClient,
    redirect_uri: Option<String>,
    offline_access: bool,
    peer: [u8; 4],
}

impl SignIn {
    /// The web app signing `email` in with `password`.
    #[must_use]
    pub fn new(email: &str, password: &str) -> Self {
        Self {
            email: email.to_owned(),
            password: password.to_owned(),
            client: FirstPartyClient::Web,
            redirect_uri: None,
            offline_access: false,
            peer: DEFAULT_PEER,
        }
    }

    /// Sign in as `client` instead.
    #[must_use]
    pub const fn client(mut self, client: FirstPartyClient) -> Self {
        self.client = client;
        self
    }

    /// Ask for the code at `redirect_uri` rather than the client's
    /// configured callback.
    #[must_use]
    pub fn redirect_uri(mut self, redirect_uri: &str) -> Self {
        self.redirect_uri = Some(redirect_uri.to_owned());
        self
    }

    /// Ask `/oauth/token` for a refresh token (`scope=offline_access`).
    #[must_use]
    pub const fn offline_access(mut self) -> Self {
        self.offline_access = true;
        self
    }

    /// Send every request from `peer`.
    #[must_use]
    pub const fn arriving_from(mut self, peer: [u8; 4]) -> Self {
        self.peer = peer;
        self
    }

    /// Walk the flow: `/oauth2/authorize` sends the signed-out athlete to the
    /// hosted login page, the form signs them in and returns to
    /// `/oauth2/authorize` with its session cookie, which sends the code to
    /// the app's callback with the `state` it was given, and the app redeems
    /// it with its verifier at `/oauth/token`.
    pub async fn run(&self, resources: &ServerContext) -> SignInOutcome {
        match self.begin(resources).await {
            Ok(issued) => {
                let scope = self.offline_access.then_some("offline_access");
                SignInOutcome::Token(issued.redeem(scope).await)
            }
            Err(refused) => *refused,
        }
    }

    /// The flow up to the code the app receives, asserting it got one — for
    /// the tests that redeem it themselves.
    pub async fn issue_code(&self, resources: &ServerContext) -> IssuedCode {
        match self.begin(resources).await {
            Ok(issued) => issued,
            Err(outcome) => match *outcome {
                SignInOutcome::LoginRefused(refused)
                | SignInOutcome::AuthorizeRefused(refused)
                | SignInOutcome::Token(refused) => panic!(
                    "the sign-in issued no code ({}): {}",
                    refused.status(),
                    refused.body_text()
                ),
            },
        }
    }

    async fn begin(&self, resources: &ServerContext) -> Result<IssuedCode, Box<SignInOutcome>> {
        let router = first_party_router(resources, self.peer);
        let client_id = self.client.client_id();
        let redirect_uri = self.redirect_uri.clone().unwrap_or_else(|| {
            self.client
                .redirect_uri(&resources.common.config.oauth2_server)
        });
        let pkce = Pkce::generate();
        let state = fresh_state();

        // The apps always ask for a fresh login (OpenID Connect
        // `prompt=login`), so a session left on the authorization server
        // never signs a different athlete in.
        let signed_out = AxumTestRequest::get(&format!(
            "{}&prompt=login",
            authorize_uri(client_id, &redirect_uri, &pkce.challenge, &state)
        ))
        .send(router.clone())
        .await;
        if !(is_redirect(&signed_out) && location(&signed_out).starts_with("/oauth2/login?")) {
            return Err(Box::new(SignInOutcome::AuthorizeRefused(signed_out)));
        }

        let login = submit_login(
            router.clone(),
            client_id,
            &redirect_uri,
            &pkce.challenge,
            &state,
            &self.email,
            &self.password,
        )
        .await;
        if !is_redirect(&login) {
            return Err(Box::new(SignInOutcome::LoginRefused(login)));
        }
        let resume = location(&login);
        assert!(
            resume.starts_with("/oauth2/authorize?"),
            "the signed-in form returns to /oauth2/authorize, got {resume}"
        );

        let authorized = AxumTestRequest::get(&resume)
            .header("cookie", &session_cookie(&login))
            .send(router.clone())
            .await;
        let callback = authorized.header("location").map(str::to_owned);
        let Some(code) = callback
            .as_deref()
            .and_then(|callback| callback_param(callback, &redirect_uri, "code"))
        else {
            return Err(Box::new(SignInOutcome::AuthorizeRefused(authorized)));
        };
        assert_eq!(
            callback_param(callback.as_deref().unwrap(), &redirect_uri, "state").as_deref(),
            Some(state.as_str()),
            "the code comes back with the app's state"
        );

        Ok(IssuedCode {
            router,
            client_id,
            redirect_uri,
            code,
            verifier: pkce.verifier,
        })
    }
}
